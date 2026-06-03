pub mod http;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum EmbedderConfig {
    Http {
        /// Base URL, e.g. `https://api.openai.com/v1` or `http://localhost:11434/v1`.
        base_url: String,
        model: String,
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        dim: Option<usize>,
    },
    Local {
        model_path: String,
        dim: usize,
    },
}

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
    fn dim(&self) -> Option<usize>;
}

pub fn build_embedder(config: &EmbedderConfig) -> Result<Box<dyn Embedder>, String> {
    match config {
        EmbedderConfig::Http {
            base_url,
            model,
            api_key,
            dim,
        } => Ok(Box::new(http::HttpEmbedder::new(
            base_url.clone(),
            model.clone(),
            api_key.clone(),
            *dim,
        ))),
        EmbedderConfig::Local { model_path, dim } => local_embedder(model_path, *dim),
    }
}

#[cfg(feature = "local-embed")]
fn local_embedder(model_path: &str, dim: usize) -> Result<Box<dyn Embedder>, String> {
    let _ = (model_path, dim);
    Err("local-embed: model loading not yet implemented".into())
}

#[cfg(not(feature = "local-embed"))]
fn local_embedder(_model_path: &str, _dim: usize) -> Result<Box<dyn Embedder>, String> {
    Err("local embedding requires building toradb-pipe with the `local-embed` feature".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_config_serde_roundtrip() {
        let cfg = EmbedderConfig::Http {
            base_url: "http://localhost:11434/v1".into(),
            model: "nomic-embed-text".into(),
            api_key: None,
            dim: Some(768),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(json.contains("\"kind\":\"http\""));
        let back: EmbedderConfig = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, EmbedderConfig::Http { .. }));
        assert!(build_embedder(&cfg).is_ok());
    }

    #[test]
    fn local_without_feature_errors() {
        let cfg = EmbedderConfig::Local {
            model_path: "/x".into(),
            dim: 384,
        };
        assert!(build_embedder(&cfg).is_err());
    }
}
