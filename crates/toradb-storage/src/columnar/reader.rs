use std::fs::File;
use std::path::Path;

use arrow::array::{Array, Float32Array, ListArray, StringArray, UInt64Array};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use super::writer::ColumnarDoc;

pub fn read_segment(path: &Path) -> Result<Vec<ColumnarDoc>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    let mut reader = builder.build().map_err(|e| e.to_string())?;

    let mut docs = Vec::new();
    for batch in reader.by_ref() {
        let batch = batch.map_err(|e| e.to_string())?;
        docs.extend(batch_to_docs(&batch)?);
    }
    Ok(docs)
}

fn batch_to_docs(batch: &RecordBatch) -> Result<Vec<ColumnarDoc>, String> {
    let ids = batch
        .column_by_name("id")
        .ok_or("missing id column")?
        .as_any()
        .downcast_ref::<UInt64Array>()
        .ok_or("id type")?;
    let texts = batch
        .column_by_name("text")
        .ok_or("missing text column")?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or("text type")?;
    let meta = batch
        .column_by_name("metadata_json")
        .ok_or("missing metadata_json column")?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or("metadata_json type")?;
    let emb_col = batch.column_by_name("embedding");

    let mut out = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let metadata = if meta.is_null(row) {
            std::collections::HashMap::new()
        } else {
            serde_json::from_str(meta.value(row)).map_err(|e| e.to_string())?
        };
        let embedding = emb_col.and_then(|c| {
            c.as_any()
                .downcast_ref::<ListArray>()
                .and_then(|list| list_value(list, row))
        });
        out.push(ColumnarDoc {
            id: ids.value(row),
            text: texts.value(row).to_string(),
            metadata,
            embedding,
        });
    }
    Ok(out)
}

fn list_value(list: &ListArray, row: usize) -> Option<Vec<f32>> {
    if list.is_null(row) {
        return None;
    }
    let values = list.value(row);
    let floats = values.as_any().downcast_ref::<Float32Array>()?;
    Some((0..floats.len()).map(|i| floats.value(i)).collect())
}
