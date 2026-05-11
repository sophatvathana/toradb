use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema};

/// Parquet segment schema: id, text, metadata_json, optional embedding list.
pub fn doc_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::UInt64, false),
        Field::new("text", DataType::Utf8, false),
        Field::new("metadata_json", DataType::Utf8, true),
        Field::new(
            "embedding",
            DataType::List(Arc::new(Field::new("item", DataType::Float32, true))),
            true,
        ),
    ]))
}
