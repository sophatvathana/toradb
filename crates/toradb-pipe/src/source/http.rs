use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use super::{Batch, Cursor, FieldInfo, RawRow, Source};

pub struct HttpSource {
    client: reqwest::Client,
    /// Base endpoint, e.g. `https://api.example.com/items`.
    url: String,
    /// Optional JSON key holding the array (e.g. `"data"`); None = top-level array.
    records_key: Option<String>,
    /// Query parameter names for pagination.
    limit_param: String,
    offset_param: String,
}

impl HttpSource {
    pub fn new(url: String, records_key: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            records_key,
            limit_param: "limit".into(),
            offset_param: "offset".into(),
        }
    }

    fn page_url(&self, limit: usize, offset: u64) -> String {
        let sep = if self.url.contains('?') { '&' } else { '?' };
        format!(
            "{}{}{}={}&{}={}",
            self.url, sep, self.limit_param, limit, self.offset_param, offset
        )
    }
}

fn stringify_value(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()), // nested arrays/objects as JSON text
    }
}

fn object_to_row(obj: &serde_json::Map<String, Value>) -> RawRow {
    let mut values = HashMap::new();
    for (k, v) in obj {
        if let Some(s) = stringify_value(v) {
            values.insert(k.clone(), s);
        }
    }
    RawRow { values }
}

fn extract_records<'a>(body: &'a Value, key: &Option<String>) -> Option<&'a Vec<Value>> {
    match key {
        Some(k) => body.get(k).and_then(Value::as_array),
        None => body.as_array(),
    }
}

#[async_trait]
impl Source for HttpSource {
    async fn test(&self) -> Result<(), String> {
        let resp = self
            .client
            .get(self.page_url(1, 0))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP source returned {}", resp.status()))
        }
    }

    async fn introspect(&self) -> Result<Vec<FieldInfo>, String> {
        let resp = self
            .client
            .get(self.page_url(1, 0))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let body: Value = resp.json().await.map_err(|e| e.to_string())?;
        let records = extract_records(&body, &self.records_key)
            .ok_or("response did not contain a record array")?;
        let Some(Value::Object(first)) = records.first() else {
            return Ok(Vec::new());
        };
        Ok(first
            .keys()
            .map(|k| FieldInfo {
                name: k.clone(),
                data_type: "json".into(),
            })
            .collect())
    }

    async fn fetch_batch(&mut self, cursor: &Cursor, n: usize) -> Result<Batch, String> {
        let offset = match cursor {
            Cursor::Offset(o) => *o,
            _ => 0,
        };
        let resp = self
            .client
            .get(self.page_url(n, offset))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP source returned {}", resp.status()));
        }
        let body: Value = resp.json().await.map_err(|e| e.to_string())?;
        let records = extract_records(&body, &self.records_key)
            .ok_or("response did not contain a record array")?;
        let rows: Vec<RawRow> = records
            .iter()
            .filter_map(|v| v.as_object().map(object_to_row))
            .collect();
        let count = rows.len() as u64;
        Ok(Batch {
            rows,
            next: Cursor::Offset(offset + count),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_to_row_stringifies_scalars_and_json() {
        let json = serde_json::json!({
            "id": 7, "name": "x", "active": true, "tags": ["a","b"], "missing": null
        });
        let row = object_to_row(json.as_object().unwrap());
        assert_eq!(row.get("id"), Some("7"));
        assert_eq!(row.get("name"), Some("x"));
        assert_eq!(row.get("active"), Some("true"));
        assert_eq!(row.get("tags"), Some("[\"a\",\"b\"]"));
        assert!(row.get("missing").is_none());
    }
}
