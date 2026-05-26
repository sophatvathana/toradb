//! Arrow RecordBatch -> columnar segment rows without `IngestDoc` intermediate.

use std::collections::HashMap;
use arrow::array::{
    Array, Float32Array, Float64Array, Int32Array, Int64Array, LargeStringArray, ListArray,
    StringArray, UInt32Array, UInt64Array,
};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use toradb_storage::columnar::ColumnarDoc;

/// Convert an Arrow batch to columnar docs with sequential ids starting at `since_id`.
pub fn record_batch_to_columnar(
    batch: &RecordBatch,
    since_id: u64,
) -> Result<Vec<ColumnarDoc>, String> {
    let n = batch.num_rows();
    if n == 0 {
        return Ok(Vec::new());
    }

    let schema = batch.schema();
    let text_idx = find_text_column(schema.as_ref())?;
    let id_idx = schema.index_of("id").ok();
    let vector_names = ["embedding", "vector"];
    let vector_idx = vector_names
        .iter()
        .find_map(|name| schema.index_of(name).ok());

    let skip: std::collections::HashSet<&str> =
        ["text", "id", "embedding", "vector"].into_iter().collect();

    let mut meta_cols: Vec<(usize, String)> = Vec::new();
    for (i, field) in schema.fields().iter().enumerate() {
        let name = field.name();
        if skip.contains(name.as_str()) {
            continue;
        }
        meta_cols.push((i, name.clone()));
    }

    let mut out = Vec::with_capacity(n);
    let mut row_id = since_id;
    for row in 0..n {
        let text = utf8_value(batch, text_idx, row)
            .ok_or_else(|| format!("missing text at row {row}"))?;
        if text.is_empty() {
            continue;
        }

        let id = if let Some(idx) = id_idx {
            uint64_value(batch, idx, row).unwrap_or(row_id)
        } else {
            row_id
        };
        row_id = row_id.max(id).saturating_add(1);

        let mut metadata = HashMap::new();
        for (col_idx, name) in &meta_cols {
            if let Some(v) = cell_as_metadata_string(batch, *col_idx, row) {
                metadata.insert(name.clone(), v);
            }
        }

        let embedding = vector_idx.and_then(|idx| extract_vector(batch, idx, row));

        out.push(ColumnarDoc {
            id,
            text,
            metadata,
            embedding,
        });
    }
    Ok(out)
}

fn find_text_column(schema: &arrow::datatypes::Schema) -> Result<usize, String> {
    if let Ok(idx) = schema.index_of("text") {
        return Ok(idx);
    }
    for (i, field) in schema.fields().iter().enumerate() {
        if matches!(field.data_type(), DataType::Utf8 | DataType::LargeUtf8) {
            return Ok(i);
        }
    }
    Err("Arrow batch requires a Utf8 text column".into())
}

fn utf8_value(batch: &RecordBatch, col: usize, row: usize) -> Option<String> {
    let array = batch.column(col);
    match array.data_type() {
        DataType::Utf8 => array
            .as_any()
            .downcast_ref::<StringArray>()
            .and_then(|a| {
                if a.is_null(row) {
                    None
                } else {
                    Some(a.value(row).to_string())
                }
            }),
        DataType::LargeUtf8 => array
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .and_then(|a| {
                if a.is_null(row) {
                    None
                } else {
                    Some(a.value(row).to_string())
                }
            }),
        _ => None,
    }
}

fn uint64_value(batch: &RecordBatch, col: usize, row: usize) -> Option<u64> {
    let array = batch.column(col);
    match array.data_type() {
        DataType::UInt64 => array
            .as_any()
            .downcast_ref::<UInt64Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row)) }),
        DataType::Int64 => array
            .as_any()
            .downcast_ref::<Int64Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row) as u64) }),
        DataType::Utf8 => utf8_value(batch, col, row).and_then(|s| s.parse().ok()),
        _ => None,
    }
}

fn cell_as_metadata_string(batch: &RecordBatch, col: usize, row: usize) -> Option<String> {
    if let Some(s) = utf8_value(batch, col, row) {
        return Some(s);
    }
    let array = batch.column(col);
    match array.data_type() {
        DataType::Int64 => array
            .as_any()
            .downcast_ref::<Int64Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row).to_string()) }),
        DataType::Int32 => array
            .as_any()
            .downcast_ref::<Int32Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row).to_string()) }),
        DataType::UInt64 => array
            .as_any()
            .downcast_ref::<UInt64Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row).to_string()) }),
        DataType::UInt32 => array
            .as_any()
            .downcast_ref::<UInt32Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row).to_string()) }),
        DataType::Float64 => array
            .as_any()
            .downcast_ref::<Float64Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row).to_string()) }),
        DataType::Float32 => array
            .as_any()
            .downcast_ref::<Float32Array>()
            .and_then(|a| if a.is_null(row) { None } else { Some(a.value(row).to_string()) }),
        _ => None,
    }
}

fn extract_vector(batch: &RecordBatch, col: usize, row: usize) -> Option<Vec<f32>> {
    let array = batch.column(col);
    match array.data_type() {
        DataType::List(_) | DataType::LargeList(_) => {
            let list = array.as_any().downcast_ref::<ListArray>()?;
            if list.is_null(row) {
                return None;
            }
            let values = list.value(row);
            let floats = values.as_any().downcast_ref::<Float32Array>()?;
            let start = list.value_offsets()[row] as usize;
            let end = list.value_offsets()[row + 1] as usize;
            Some((start..end).map(|i| floats.value(i)).collect())
        }
        DataType::FixedSizeList(_, dim) => {
            let list = array
                .as_any()
                .downcast_ref::<arrow::array::FixedSizeListArray>()?;
            if list.is_null(row) {
                return None;
            }
            let values = list.value(row);
            let floats = values.as_any().downcast_ref::<Float32Array>()?;
            Some((0..*dim as usize).map(|i| floats.value(i)).collect())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::StringArray;
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;

    use super::record_batch_to_columnar;

    #[test]
    fn batch_to_columnar_assigns_ids() {
        let schema = Schema::new(vec![Field::new("text", DataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            Arc::new(schema),
            vec![Arc::new(StringArray::from(vec!["hello world"]))],
        )
        .expect("batch");
        let docs = record_batch_to_columnar(&batch, 10).expect("convert");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, 10);
        assert_eq!(docs[0].text, "hello world");
    }
}
