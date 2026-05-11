use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, ListArray, StringArray, UInt64Array};
use arrow::buffer::OffsetBuffer;
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

use super::schema::doc_schema;

#[derive(Debug, Clone)]
pub struct ColumnarDoc {
    pub id: u64,
    pub text: String,
    pub metadata: HashMap<String, String>,
    pub embedding: Option<Vec<f32>>,
}

pub fn write_segment(path: &Path, docs: &[ColumnarDoc]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let schema = doc_schema();
    let batch = docs_to_batch(&schema, docs)?;
    let file = File::create(path).map_err(|e| e.to_string())?;
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).map_err(|e| e.to_string())?;
    writer.write(&batch).map_err(|e| e.to_string())?;
    writer.close().map_err(|e| e.to_string())?;
    Ok(())
}

fn docs_to_batch(schema: &Arc<arrow::datatypes::Schema>, docs: &[ColumnarDoc]) -> Result<RecordBatch, String> {
    let ids: Vec<u64> = docs.iter().map(|d| d.id).collect();
    let texts: Vec<&str> = docs.iter().map(|d| d.text.as_str()).collect();
    let mut meta = Vec::with_capacity(docs.len());
    for doc in docs {
        if doc.metadata.is_empty() {
            meta.push(None);
        } else {
            meta.push(Some(
                serde_json::to_string(&doc.metadata).map_err(|e| e.to_string())?,
            ));
        }
    }

    let id_arr = Arc::new(UInt64Array::from(ids)) as ArrayRef;
    let text_arr = Arc::new(StringArray::from(texts)) as ArrayRef;
    let meta_arr = Arc::new(StringArray::from(meta)) as ArrayRef;

    let embedding_arr = if docs.iter().any(|d| d.embedding.is_some()) {
        let mut offsets = vec![0i32];
        let mut values = Vec::new();
        for doc in docs {
            if let Some(ref emb) = doc.embedding {
                values.extend(emb.iter().copied());
                offsets.push(values.len() as i32);
            } else {
                offsets.push(values.len() as i32);
            }
        }
        let values_arr = Arc::new(Float32Array::from(values));
        let offsets_arr = OffsetBuffer::new(offsets.into());
        Arc::new(ListArray::new(
            Arc::new(arrow::datatypes::Field::new("item", arrow::datatypes::DataType::Float32, true)),
            offsets_arr,
            values_arr,
            None,
        )) as ArrayRef
    } else {
        Arc::new(ListArray::new_null(
            Arc::new(arrow::datatypes::Field::new("item", arrow::datatypes::DataType::Float32, true)),
            docs.len(),
        )) as ArrayRef
    };

    RecordBatch::try_new(
        schema.clone(),
        vec![id_arr, text_arr, meta_arr, embedding_arr],
    )
    .map_err(|e| e.to_string())
}
