//! Hugging Face Hub ingest: resolve Parquet shards (datasets-server or repo tree),
//! parallel download (hf_transfer), then ingest.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::dag::DagRunner;
use crate::hf_transfer::{self, default_download_settings};
use crate::ingest_file::{ingest_jsonl, ingest_parquet};

const HF_REVISION: &str = "main";

#[derive(Debug, Clone)]
pub struct HfIngestParams {
    pub dataset: String,
    pub config: Option<String>,
    pub split: String,
    pub text_column: String,
    pub limit: u64,
}

/// Local files downloaded from the Hub, ready for ingest.
pub struct HfDownloadBundle {
    pub download_dir: PathBuf,
    pub remote_paths: Vec<String>,
    pub format: &'static str,
}

#[derive(Debug, Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    entry_type: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct DatasetsServerParquetResponse {
    parquet_files: Vec<ParquetFileMeta>,
}

#[derive(Debug, Deserialize)]
struct ParquetFileMeta {
    url: String,
    filename: String,
}

/// Resolved file to download.
enum HfRemoteFile {
    /// Full download URL (from datasets-server or similar).
    Url { url: String, filename: String },
    /// Path relative to the dataset repo on the Hub.
    HubPath(String),
}

fn hf_auth_headers() -> HashMap<String, String> {
    let mut headers = HashMap::new();
    if let Ok(token) =
        std::env::var("HF_TOKEN").or_else(|_| std::env::var("HUGGING_FACE_HUB_TOKEN"))
    {
        if !token.is_empty() {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
    }
    headers
}

fn hf_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

async fn hub_get(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, String> {
    let mut req = client.get(url);
    for (k, v) in hf_auth_headers() {
        req = req.header(k, v);
    }
    req.send()
        .await
        .map_err(|e| format!("HF request failed: {e}"))
}

async fn list_parquet_via_datasets_server(
    dataset: &str,
    config: Option<&str>,
    split: &str,
) -> Result<Vec<HfRemoteFile>, String> {
    let client = hf_client()?;
    let mut query: Vec<(&str, &str)> = vec![("dataset", dataset), ("split", split)];
    if let Some(cfg) = config.filter(|c| !c.is_empty()) {
        query.push(("config", cfg));
    }
    let mut req = client
        .get("https://datasets-server.huggingface.co/parquet")
        .query(&query);
    for (k, v) in hf_auth_headers() {
        req = req.header(k, v);
    }
    let response = req
        .send()
        .await
        .map_err(|e| format!("datasets-server parquet API: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "datasets-server returned {} for dataset={dataset} split={split}",
            response.status()
        ));
    }

    let body: DatasetsServerParquetResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(body
        .parquet_files
        .into_iter()
        .map(|f| HfRemoteFile::Url {
            url: f.url,
            filename: f.filename,
        })
        .collect())
}

async fn list_dataset_files(dataset: &str) -> Result<Vec<String>, String> {
    let url =
        format!("https://huggingface.co/api/datasets/{dataset}/tree/{HF_REVISION}?recursive=1");
    let client = hf_client()?;
    let response = hub_get(&client, &url).await?;
    if !response.status().is_success() {
        return Err(format!(
            "HF Hub API returned {} for {}",
            response.status(),
            url
        ));
    }
    let entries: Vec<TreeEntry> = response.json().await.map_err(|e| e.to_string())?;
    let mut files: Vec<String> = entries
        .into_iter()
        .filter(|e| e.entry_type == "file")
        .map(|e| e.path)
        .collect();
    files.sort();
    Ok(files)
}

fn resolve_hub_download_url(dataset: &str, path: &str) -> String {
    format!("https://huggingface.co/datasets/{dataset}/resolve/{HF_REVISION}/{path}")
}

fn path_matches_config(path: &str, config: Option<&str>) -> bool {
    match config.filter(|c| !c.is_empty()) {
        None => true,
        Some(cfg) => {
            path.contains(&format!("/{cfg}/"))
                || path.contains(&format!("/{cfg}."))
                || path.starts_with(&format!("{cfg}/"))
                || path.starts_with(cfg)
        }
    }
}

fn path_matches_split(path: &str, split: &str) -> bool {
    split.is_empty() || path.contains(split) || !path.contains('/')
}

fn pick_ingest_files(
    files: &[String],
    split: &str,
    config: Option<&str>,
) -> Result<(Vec<HfRemoteFile>, &'static str), String> {
    let parquet: Vec<HfRemoteFile> = files
        .iter()
        .filter(|p| {
            p.ends_with(".parquet")
                && path_matches_split(p, split)
                && path_matches_config(p, config)
        })
        .map(|p| HfRemoteFile::HubPath(p.clone()))
        .collect();
    if !parquet.is_empty() {
        return Ok((parquet, "parquet"));
    }

    let jsonl: Vec<HfRemoteFile> = files
        .iter()
        .filter(|p| {
            (p.ends_with(".jsonl") || p.ends_with(".json"))
                && path_matches_split(p, split)
                && path_matches_config(p, config)
        })
        .map(|p| HfRemoteFile::HubPath(p.clone()))
        .collect();
    if !jsonl.is_empty() {
        return Ok((jsonl, "jsonl"));
    }

    let cfg_hint = config.unwrap_or("(default)");
    Err(format!(
        "no .parquet or .jsonl files found for dataset (config: {cfg_hint}, split: {split}). \
         Try a dataset config that exposes Parquet via the Hub (e.g. allenai/c4 with config km)."
    ))
}

async fn resolve_remote_files(
    params: &HfIngestParams,
) -> Result<(Vec<HfRemoteFile>, &'static str), String> {
    let config = params.config.as_deref();

    if let Ok(files) =
        list_parquet_via_datasets_server(&params.dataset, config, &params.split).await
    {
        if !files.is_empty() {
            return Ok((files, "parquet"));
        }
    }

    let all_files = list_dataset_files(&params.dataset).await?;
    pick_ingest_files(&all_files, &params.split, config)
}

fn truncate_shards(files: &mut Vec<HfRemoteFile>, limit: u64) {
    if files.len() > 1 && limit > 0 && limit < 50_000 {
        files.truncate(1);
    }
}

fn remote_label(file: &HfRemoteFile) -> String {
    match file {
        HfRemoteFile::Url { filename, .. } => filename.clone(),
        HfRemoteFile::HubPath(p) => p.clone(),
    }
}

/// List and download Hub files (async; safe to call from a Tokio runtime).
pub async fn download_hf_dataset(
    temp_dir: &Path,
    params: &HfIngestParams,
) -> Result<HfDownloadBundle, String> {
    download_hf_dataset_with_progress(temp_dir, params, None::<fn(&str, Option<u8>)>).await
}

/// Same as [`download_hf_dataset`] but reports `phase` and optional `progress` (0–100).
pub async fn download_hf_dataset_with_progress<F>(
    temp_dir: &Path,
    params: &HfIngestParams,
    mut on_progress: Option<F>,
) -> Result<HfDownloadBundle, String>
where
    F: FnMut(&str, Option<u8>),
{
    let mut report = |phase: &str, progress: Option<u8>| {
        if let Some(ref mut f) = on_progress {
            f(phase, progress);
        }
    };
    report("resolving", Some(0));
    std::fs::create_dir_all(temp_dir).map_err(|e| e.to_string())?;
    let download_dir = temp_dir.join(format!(
        "hf_{}_{}",
        params.dataset.replace('/', "_"),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&download_dir).map_err(|e| e.to_string())?;

    let (mut files, format) = resolve_remote_files(params).await?;
    if files.is_empty() {
        return Err("no ingestible files selected from Hugging Face dataset".to_string());
    }
    truncate_shards(&mut files, params.limit);
    let total_files = files.len();

    let headers = hf_auth_headers();
    let (max_files, chunk_size, parallel_failures, max_retries) = default_download_settings();
    let mut remote_paths = Vec::with_capacity(files.len());

    for (idx, file) in files.iter().enumerate() {
        // Map download shards to 5–80% so ingest (85–99%) and done (100%) do not flash instantly.
        let pct = Some((5 + (idx + 1) * 75 / total_files.max(1)).min(80) as u8);
        report(&format!("downloading {}/{}", idx + 1, total_files), pct);
        let (url, local_name) = match file {
            HfRemoteFile::Url { url, filename } => (url.clone(), filename.clone()),
            HfRemoteFile::HubPath(path) => {
                let name = PathBuf::from(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("shard_{idx}"));
                (resolve_hub_download_url(&params.dataset, path), name)
            }
        };
        remote_paths.push(remote_label(file));
        let local_path = download_dir.join(&local_name);
        hf_transfer::download(
            &url,
            &local_path,
            max_files,
            chunk_size,
            parallel_failures,
            max_retries,
            Some(headers.clone()),
        )
        .await
        .map_err(|e| format!("HF download failed for {}: {e}", remote_paths[idx]))?;
    }

    report("download complete", Some(82));
    Ok(HfDownloadBundle {
        download_dir,
        remote_paths,
        format,
    })
}

/// Ingest a prior `download_hf_dataset` result into the table (sync; hold `dag` lock only here).
pub fn ingest_hf_bundle(
    dag: &mut DagRunner,
    table: &str,
    bundle: &HfDownloadBundle,
    limit: u64,
) -> Result<u64, String> {
    let rows = match bundle.format {
        "parquet" => ingest_parquet(dag, table, &bundle.download_dir, limit)?,
        "jsonl" => {
            let first = bundle.download_dir.join(
                PathBuf::from(&bundle.remote_paths[0])
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "data.jsonl".to_string()),
            );
            ingest_jsonl(dag, table, &first, 200_000, limit)?
        }
        _ => return Err("unsupported ingest format".to_string()),
    };
    let _ = std::fs::remove_dir_all(&bundle.download_dir);
    Ok(rows)
}

/// Download from the Hub and ingest (async download + sync ingest).
pub async fn ingest_hf(
    dag: &mut DagRunner,
    table: &str,
    temp_dir: &Path,
    params: &HfIngestParams,
) -> Result<u64, String> {
    let bundle = download_hf_dataset(temp_dir, params).await?;
    ingest_hf_bundle(dag, table, &bundle, params.limit)
}
