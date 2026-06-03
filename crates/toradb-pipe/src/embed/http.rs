use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::Embedder;

pub struct HttpEmbedder {
    client: reqwest::Client,
    endpoint: String,
    model: String,
    api_key: Option<String>,
    dim: Option<usize>,
}

impl HttpEmbedder {
    pub fn new(
        base_url: String,
        model: String,
        api_key: Option<String>,
        dim: Option<usize>,
    ) -> Self {
        let endpoint = format!("{}/embeddings", base_url.trim_end_matches('/'));
        Self {
            client: reqwest::Client::new(),
            endpoint,
            model,
            api_key,
            dim,
        }
    }
}

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[async_trait]
impl Embedder for HttpEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut req = self
            .client
            .post(&self.endpoint)
            .json(&EmbedRequest {
                model: &self.model,
                input: texts,
            });
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("embedding request failed ({status}): {body}"));
        }
        let parsed: EmbedResponse = resp.json().await.map_err(|e| e.to_string())?;
        if parsed.data.len() != texts.len() {
            return Err(format!(
                "embedding count mismatch: got {}, expected {}",
                parsed.data.len(),
                texts.len()
            ));
        }
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dim(&self) -> Option<usize> {
        self.dim
    }
}
