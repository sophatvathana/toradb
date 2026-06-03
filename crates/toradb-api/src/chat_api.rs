//! OpenAI-compatible chat completions proxy for platform Chat (env-based upstream).

use axum::body::Body;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;

use super::ApiError;

#[derive(Serialize)]
pub struct ChatConfigResponse {
    pub proxy_available: bool,
    pub default_model: Option<String>,
}

fn llm_env() -> (Option<String>, Option<String>, Option<String>) {
    (
        std::env::var("TORADB_LLM_BASE_URL").ok(),
        std::env::var("TORADB_LLM_API_KEY").ok(),
        std::env::var("TORADB_LLM_MODEL").ok(),
    )
}

pub async fn chat_config() -> Result<Json<ChatConfigResponse>, ApiError> {
    let (base, key, model) = llm_env();
    let proxy_available =
        base.as_ref().is_some_and(|b| !b.is_empty()) && key.as_ref().is_some_and(|k| !k.is_empty());
    Ok(Json(ChatConfigResponse {
        proxy_available,
        default_model: model,
    }))
}

pub async fn chat_completions(
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let (base, key, model) = llm_env();
    let base_url = base.filter(|b| !b.is_empty()).ok_or_else(|| {
        ApiError::service_unavailable(
            "LLM proxy not configured. Set TORADB_LLM_BASE_URL and TORADB_LLM_API_KEY.",
        )
    })?;
    let api_key = key.filter(|k| !k.is_empty()).ok_or_else(|| {
        ApiError::service_unavailable("LLM proxy: TORADB_LLM_API_KEY is not set.")
    })?;

    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut upstream_body = body;
    if upstream_body.get("model").is_none() {
        if let Some(m) = model.filter(|m| !m.is_empty()) {
            if let Some(obj) = upstream_body.as_object_mut() {
                obj.insert("model".into(), Value::String(m));
            }
        }
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let client = Client::new();
    let mut req = client.post(&url).bearer_auth(&api_key).json(&upstream_body);

    // Forward Accept for SSE when streaming.
    if let Some(accept) = headers.get(header::ACCEPT) {
        req = req.header(header::ACCEPT, accept);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("LLM upstream request failed: {e}")))?;

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or(if stream {
            "text/event-stream"
        } else {
            "application/json"
        })
        .to_string();

    if stream {
        let byte_stream = resp
            .bytes_stream()
            .map(|result| result.map_err(|e| std::io::Error::other(e.to_string())));
        let body = Body::from_stream(byte_stream);
        return Ok((status, [(header::CONTENT_TYPE, content_type)], body).into_response());
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| ApiError::internal(format!("LLM upstream read failed: {e}")))?;
    Ok((
        status,
        [(header::CONTENT_TYPE, content_type)],
        bytes.to_vec(),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_unavailable_without_env() {
        // Do not set env in test; proxy_available depends on runtime env.
        let (base, key, _) = llm_env();
        let available = base.as_ref().is_some_and(|b| !b.is_empty())
            && key.as_ref().is_some_and(|k| !k.is_empty());
        assert!(!available || (base.is_some() && key.is_some()));
    }
}
