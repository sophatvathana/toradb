use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use toradb_core::{ColumnType, CompressionConfig};

use super::metadata_codec::{docs_to_batch, docs_to_legacy_batch};
use super::typed_schema::table_doc_schema;

pub fn write_segment_from_batches(
    path: &Path,
    schema: Arc<arrow::datatypes::Schema>,
    batches: impl Iterator<Item = Result<RecordBatch, String>>,
    compression: Option<&CompressionConfig>,
) -> Result<u64, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut props_builder = WriterProperties::builder();
    if let Some(cfg) = compression {
        if cfg.enabled {
            props_builder = props_builder.set_compression(Compression::ZSTD(Default::default()));
        }
    }
    let props = props_builder.build();
    let mut writer =
        ArrowWriter::try_new(file, schema, Some(props)).map_err(|e| e.to_string())?;
    let mut row_count = 0u64;
    for batch in batches {
        let batch = batch?;
        row_count += batch.num_rows() as u64;
        writer.write(&batch).map_err(|e| e.to_string())?;
    }
    writer.close().map_err(|e| e.to_string())?;
    Ok(row_count)
}

#[derive(Debug, Clone)]
pub struct ColumnarDoc {
    pub id: u64,
    pub text: String,
    pub metadata: HashMap<String, String>,
    pub embedding: Option<Vec<f32>>,
}

pub fn write_segment(path: &Path, docs: &[ColumnarDoc]) -> Result<(), String> {
    write_segment_with_compression(path, docs, None, &[])
}

pub fn write_segment_with_compression(
    path: &Path,
    docs: &[ColumnarDoc],
    compression: Option<&CompressionConfig>,
    column_types: &[(String, ColumnType)],
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let schema = table_doc_schema(column_types);
    let batch = if column_types.is_empty() {
        docs_to_legacy_batch(&schema, docs)?
    } else {
        docs_to_batch(&schema, column_types, docs)?
    };
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut props_builder = WriterProperties::builder();
    if let Some(cfg) = compression {
        if cfg.enabled {
            props_builder = props_builder.set_compression(Compression::ZSTD(Default::default()));
        }
    }
    let props = props_builder.build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).map_err(|e| e.to_string())?;
    writer.write(&batch).map_err(|e| e.to_string())?;
    writer.close().map_err(|e| e.to_string())?;
    Ok(())
}
