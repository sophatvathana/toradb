// Parallel HTTP download logic adapted from Hugging Face hf_transfer:
// https://raw.githubusercontent.com/huggingface/hf_transfer/refs/heads/main/src/lib.rs
// (see also `third_party/hf_transfer_lib.rs` in this crate for the upstream reference copy)

use std::collections::HashMap;
use std::fmt::Display;
use std::fs::remove_file;
use std::io::SeekFrom;
use std::path::Path;
use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use rand::{thread_rng, Rng};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_RANGE, RANGE};
use reqwest::Url;
use std::sync::Arc;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use tokio::time::sleep;

const BASE_WAIT_TIME: usize = 300;
const MAX_WAIT_TIME: usize = 10_000;

#[derive(Debug)]
enum DownloadError {
    Io(std::io::Error),
    Request(reqwest::Error),
    Header(reqwest::header::ToStrError),
    Message(String),
}

impl From<std::io::Error> for DownloadError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<reqwest::Error> for DownloadError {
    fn from(value: reqwest::Error) -> Self {
        Self::Request(value)
    }
}

impl From<reqwest::header::ToStrError> for DownloadError {
    fn from(value: reqwest::header::ToStrError) -> Self {
        Self::Header(value)
    }
}

impl Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Request(e) => write!(f, "request: {e}"),
            Self::Header(e) => write!(f, "header: {e}"),
            Self::Message(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for DownloadError {}

fn jitter() -> usize {
    thread_rng().gen_range(0..=500)
}

fn exponential_backoff(base_wait_time: usize, n: usize, max: usize) -> usize {
    (base_wait_time + n.pow(2) + jitter()).min(max)
}

/// Download `url` to `filename` using parallel range requests.
pub async fn download(
    url: &str,
    filename: impl AsRef<Path>,
    max_files: usize,
    chunk_size: usize,
    parallel_failures: usize,
    max_retries: usize,
    headers: Option<HashMap<String, String>>,
) -> Result<(), String> {
    if parallel_failures > max_files {
        return Err("parallel_failures cannot be > max_files".to_string());
    }
    if (parallel_failures == 0) != (max_retries == 0) {
        return Err("for retry mechanism set both parallel_failures and max_retries".to_string());
    }

    let filename = filename.as_ref().to_path_buf();
    let result = download_inner(
        url.to_string(),
        filename.clone(),
        max_files,
        chunk_size,
        parallel_failures,
        max_retries,
        headers,
    )
    .await;

    if result.is_err() && filename.exists() {
        let _ = remove_file(&filename);
    }
    result.map_err(|e| e.to_string())
}

async fn download_inner(
    url: String,
    filename: std::path::PathBuf,
    max_files: usize,
    chunk_size: usize,
    parallel_failures: usize,
    max_retries: usize,
    input_headers: Option<HashMap<String, String>>,
) -> Result<(), DownloadError> {
    let client = reqwest::Client::new();

    let mut headers = HeaderMap::new();
    let mut auth_token = None;
    if let Some(input_headers) = input_headers {
        headers.reserve(input_headers.len());
        for (k, v) in input_headers {
            let name: HeaderName = k
                .parse()
                .map_err(|_| DownloadError::Message(format!("invalid header: {k}")))?;
            let value: HeaderValue = v
                .as_str()
                .try_into()
                .map_err(|_| DownloadError::Message(format!("invalid header value: {v}")))?;
            if name == AUTHORIZATION {
                auth_token = Some(value);
            } else {
                headers.insert(name, value);
            }
        }
    }

    let response = if let Some(token) = auth_token.as_ref() {
        client.get(&url).header(AUTHORIZATION, token)
    } else {
        client.get(&url)
    }
    .headers(headers.clone())
    .header(RANGE, "bytes=0-0")
    .send()
    .await?
    .error_for_status()?;

    let redirected_url = response.url().clone();
    if Url::parse(&url)
        .map_err(|e| DownloadError::Message(format!("failed to parse url: {e}")))?
        .host()
        == redirected_url.host()
    {
        if let Some(token) = auth_token {
            headers.insert(AUTHORIZATION, token);
        }
    }

    let content_range = response
        .headers()
        .get(CONTENT_RANGE)
        .ok_or_else(|| DownloadError::Message("no content-range header".into()))?
        .to_str()?;

    let size_parts: Vec<&str> = content_range.split('/').collect();
    let length: usize = size_parts
        .last()
        .ok_or_else(|| DownloadError::Message("no content length in range".into()))?
        .parse()
        .map_err(|e| DownloadError::Message(format!("invalid content length: {e}")))?;

    let mut handles = FuturesUnordered::new();
    let semaphore = Arc::new(Semaphore::new(max_files));
    let parallel_failures_semaphore = Arc::new(Semaphore::new(parallel_failures));

    for start in (0..length).step_by(chunk_size) {
        let url = redirected_url.to_string();
        let filename = filename.clone();
        let client = client.clone();
        let headers = headers.clone();
        let stop = std::cmp::min(start + chunk_size - 1, length);
        let semaphore = semaphore.clone();
        let parallel_failures_semaphore = parallel_failures_semaphore.clone();

        handles.push(tokio::spawn(async move {
            let permit = semaphore
                .acquire_owned()
                .await
                .map_err(|e| DownloadError::Message(format!("semaphore acquire: {e}")))?;
            let mut chunk =
                download_chunk(&client, &url, &filename, start, stop, headers.clone()).await;
            let mut i = 0;
            if parallel_failures > 0 {
                while let Err(dlerr) = chunk {
                    if i >= max_retries {
                        return Err(DownloadError::Message(format!(
                            "failed after {max_retries} retries: {dlerr}"
                        )));
                    }
                    let _parallel_failure_permit = parallel_failures_semaphore
                        .clone()
                        .try_acquire_owned()
                        .map_err(|err| {
                            DownloadError::Message(format!(
                                "too many parallel failures ({parallel_failures}): {dlerr} ({err})"
                            ))
                        })?;
                    let wait_time = exponential_backoff(BASE_WAIT_TIME, i, MAX_WAIT_TIME);
                    sleep(Duration::from_millis(wait_time as u64)).await;
                    chunk = download_chunk(&client, &url, &filename, start, stop, headers.clone())
                        .await;
                    i += 1;
                }
            }
            drop(permit);
            chunk
        }));
    }

    while let Some(result) = handles.next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(e) => {
                return Err(DownloadError::Message(format!("task join: {e}")));
            }
        }
    }
    Ok(())
}

async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    filename: &Path,
    start: usize,
    stop: usize,
    headers: HeaderMap,
) -> Result<(), DownloadError> {
    let range = format!("bytes={start}-{stop}");
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(false)
        .create(true)
        .open(filename)
        .await?;
    file.seek(SeekFrom::Start(start as u64)).await?;
    let response = client
        .get(url)
        .headers(headers)
        .header(RANGE, range)
        .send()
        .await?
        .error_for_status()?;
    let content = response.bytes().await?;
    file.write_all(&content).await?;
    Ok(())
}

/// Default parallel download settings (tune via env `HF_HUB_ENABLE_HF_TRANSFER` style limits).
pub fn default_download_settings() -> (usize, usize, usize, usize) {
    let max_files = std::env::var("HF_TRANSFER_MAX_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let chunk_size = std::env::var("HF_TRANSFER_CHUNK_SIZE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000_000);
    (max_files, chunk_size, 3, 5)
}
