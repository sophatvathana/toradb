pub mod http;
pub mod sql;

#[cfg(feature = "pg-cdc")]
pub mod pg_cdc;

use std::collections::HashMap;

use async_trait::async_trait;
use toradb_index::IngestDoc;

use crate::model::ColumnMapping;

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub name: String,
    pub data_type: String,
}

#[derive(Clone, Debug, Default)]
pub struct RawRow {
    pub values: HashMap<String, String>,
}

impl RawRow {
    pub fn get(&self, col: &str) -> Option<&str> {
        self.values.get(col).map(String::as_str)
    }
}

#[derive(Clone, Debug, Default)]
pub enum Cursor {
    #[default]
    Start,
    Offset(u64),
    Value(String),
}

pub struct Batch {
    pub rows: Vec<RawRow>,
    pub next: Cursor,
}

#[async_trait]
pub trait Source: Send {
    async fn test(&self) -> Result<(), String>;
    async fn introspect(&self) -> Result<Vec<FieldInfo>, String>;
    async fn fetch_batch(&mut self, cursor: &Cursor, n: usize) -> Result<Batch, String>;
}

pub fn row_to_doc(row: &RawRow, mapping: &ColumnMapping) -> (IngestDoc, Option<String>) {
    let text = row.get(&mapping.text_column).unwrap_or("").to_string();

    let mut metadata = HashMap::new();
    for (name, value) in &row.values {
        if mapping.wants_metadata(name) {
            metadata.insert(name.clone(), value.clone());
        }
    }

    let vector = mapping
        .vector_column
        .as_deref()
        .and_then(|c| row.get(c))
        .and_then(parse_vector);

    let cursor = mapping
        .cursor_column
        .as_deref()
        .and_then(|c| row.get(c))
        .map(str::to_string);

    (
        IngestDoc {
            text,
            metadata,
            vector,
            sparse: None,
        },
        cursor,
    )
}

pub fn parse_vector(s: &str) -> Option<Vec<f32>> {
    let t = s.trim().trim_start_matches('[').trim_end_matches(']');
    let parsed: Result<Vec<f32>, _> = t
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<f32>())
        .collect();
    parsed.ok().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vector_handles_brackets_and_plain() {
        assert_eq!(parse_vector("[1, 2, 3]"), Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(parse_vector("0.5,0.25"), Some(vec![0.5, 0.25]));
        assert_eq!(parse_vector("[]"), None);
        assert_eq!(parse_vector("not a vector"), None);
        assert_eq!(parse_vector(""), None);
    }

    #[test]
    fn row_to_doc_maps_text_metadata_vector() {
        let mut values = HashMap::new();
        values.insert("body".to_string(), "hello".to_string());
        values.insert("tag".to_string(), "x".to_string());
        values.insert("emb".to_string(), "[0.1,0.2]".to_string());
        values.insert("updated".to_string(), "42".to_string());
        let row = RawRow { values };
        let mapping = ColumnMapping {
            text_column: "body".into(),
            metadata_columns: vec!["tag".into()],
            vector_column: Some("emb".into()),
            id_column: None,
            cursor_column: Some("updated".into()),
        };
        let (doc, cursor) = row_to_doc(&row, &mapping);
        assert_eq!(doc.text, "hello");
        assert_eq!(doc.metadata.get("tag").map(String::as_str), Some("x"));
        assert!(doc.metadata.get("emb").is_none());
        assert_eq!(doc.vector, Some(vec![0.1, 0.2]));
        assert_eq!(cursor.as_deref(), Some("42"));
    }
}
