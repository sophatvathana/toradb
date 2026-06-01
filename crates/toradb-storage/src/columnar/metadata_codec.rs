use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, BooleanArray, Date32Array, Float32Array, Float64Array, Int64Array, ListArray,
    StringArray, TimestampMillisecondArray, UInt64Array,
};
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{DataType, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use toradb_core::{ColumnType, ColumnTypeSpec};

use super::typed_schema::sorted_column_types;
use super::writer::ColumnarDoc;

/// Split ingest metadata into typed column values and overflow JSON.
pub fn split_metadata_for_write(
    metadata: &HashMap<String, String>,
    column_types: &[(String, ColumnTypeSpec)],
) -> (HashMap<String, String>, HashMap<String, String>) {
    let typed_names: HashMap<String, ColumnType> = sorted_column_types(column_types)
        .into_iter()
        .map(|(n, t)| (n, t.kind))
        .collect();
    let mut typed = HashMap::new();
    let mut overflow = HashMap::new();
    for (k, v) in metadata {
        let key = k.to_ascii_lowercase();
        if typed_names.contains_key(&key) {
            typed.insert(key, v.clone());
        } else {
            overflow.insert(k.clone(), v.clone());
        }
    }
    (typed, overflow)
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "1" | "yes" | "y" => Some(true),
        "false" | "f" | "0" | "no" | "n" => Some(false),
        _ => None,
    }
}

fn parse_date_days(s: &str) -> Option<i32> {
    let s = s.trim();
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    let sep = bytes[4];
    if (sep != b'-' && sep != b'/') || bytes[7] != sep {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    i32::try_from(days).ok()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn parse_timestamp_millis(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches('Z');
    let days = days_from_civil(
        s.get(0..4)?.parse().ok()?,
        s.get(5..7)?.parse().ok()?,
        s.get(8..10)?.parse().ok()?,
    );
    let mut millis = days * 86_400_000;
    if s.len() > 10 {
        let sep = s.as_bytes()[10];
        if sep != b'T' && sep != b' ' {
            return None;
        }
        let time = &s[11..];
        let mut parts = time.split(':');
        let hh: i64 = parts.next()?.parse().ok()?;
        let mm: i64 = parts.next()?.parse().ok()?;
        let mut ss = 0i64;
        let mut frac_ms = 0i64;
        if let Some(sec) = parts.next() {
            let mut sec_parts = sec.split('.');
            ss = sec_parts.next()?.parse().ok()?;
            if let Some(frac) = sec_parts.next() {
                let mut f = String::from(frac);
                f.truncate(3);
                while f.len() < 3 {
                    f.push('0');
                }
                frac_ms = f.parse().ok()?;
            }
        }
        millis += hh * 3_600_000 + mm * 60_000 + ss * 1_000 + frac_ms;
    }
    Some(millis)
}

fn encode_typed_column(
    _name: &str,
    ty: ColumnType,
    values: &[Option<String>],
) -> Result<ArrayRef, String> {
    match ty {
        ColumnType::Int => {
            let data: Vec<Option<i64>> = values
                .iter()
                .map(|v| v.as_ref().and_then(|s| s.trim().parse().ok()))
                .collect();
            Ok(Arc::new(Int64Array::from(data)) as ArrayRef)
        }
        ColumnType::Float => {
            let data: Vec<Option<f64>> = values
                .iter()
                .map(|v| v.as_ref().and_then(|s| s.trim().parse().ok()))
                .collect();
            Ok(Arc::new(Float64Array::from(data)) as ArrayRef)
        }
        ColumnType::Bool => {
            let data: Vec<Option<bool>> = values
                .iter()
                .map(|v| v.as_ref().and_then(|s| parse_bool(s)))
                .collect();
            Ok(Arc::new(BooleanArray::from(data)) as ArrayRef)
        }
        ColumnType::Date => {
            let data: Vec<Option<i32>> = values
                .iter()
                .map(|v| v.as_ref().and_then(|s| parse_date_days(s)))
                .collect();
            Ok(Arc::new(Date32Array::from(data)) as ArrayRef)
        }
        ColumnType::Timestamp => {
            let data: Vec<Option<i64>> = values
                .iter()
                .map(|v| v.as_ref().and_then(|s| parse_timestamp_millis(s)))
                .collect();
            Ok(Arc::new(TimestampMillisecondArray::from(data)) as ArrayRef)
        }
        ColumnType::Vector => {
            let mut offsets = vec![0i32];
            let mut flat = Vec::new();
            for v in values {
                if let Some(s) = v {
                    if let Ok(parsed) = serde_json::from_str::<Vec<f32>>(s) {
                        flat.extend(parsed);
                    } else {
                        flat.extend(s.split(',').filter_map(|p| p.trim().parse::<f32>().ok()));
                    }
                }
                offsets.push(flat.len() as i32);
            }
            let values_arr = Arc::new(Float32Array::from(flat));
            let offsets_arr = OffsetBuffer::new(offsets.into());
            Ok(Arc::new(ListArray::new(
                Arc::new(Field::new("item", DataType::Float32, true)),
                offsets_arr,
                values_arr,
                None,
            )) as ArrayRef)
        }
        ColumnType::Text | ColumnType::Json | ColumnType::Uuid => {
            let data: Vec<Option<&str>> = values
                .iter()
                .map(|v| v.as_ref().map(|s| s.as_str()))
                .collect();
            Ok(Arc::new(StringArray::from(data)) as ArrayRef)
        }
    }
}

use arrow::datatypes::Field;

fn decode_typed_value(ty: ColumnType, array: &dyn Array, row: usize) -> Option<String> {
    if array.is_null(row) {
        return None;
    }
    match ty {
        ColumnType::Int => array
            .as_any()
            .downcast_ref::<Int64Array>()
            .map(|a| a.value(row).to_string()),
        ColumnType::Float => array
            .as_any()
            .downcast_ref::<Float64Array>()
            .map(|a| a.value(row).to_string()),
        ColumnType::Bool => array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .map(|a| a.value(row).to_string()),
        ColumnType::Date => array
            .as_any()
            .downcast_ref::<Date32Array>()
            .map(|a| format!("{}", a.value(row))),
        ColumnType::Timestamp => array
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .map(|a| a.value(row).to_string()),
        ColumnType::Text | ColumnType::Json | ColumnType::Uuid => array
            .as_any()
            .downcast_ref::<StringArray>()
            .map(|a| a.value(row).to_string()),
        ColumnType::Vector => {
            let list = array.as_any().downcast_ref::<ListArray>()?;
            if list.is_null(row) {
                return None;
            }
            let vals = list.value(row);
            let floats = vals.as_any().downcast_ref::<Float32Array>()?;
            let v: Vec<f32> = (0..floats.len()).map(|i| floats.value(i)).collect();
            serde_json::to_string(&v).ok()
        }
    }
}

pub fn docs_to_batch(
    schema: &Arc<Schema>,
    column_types: &[(String, ColumnTypeSpec)],
    docs: &[ColumnarDoc],
) -> Result<RecordBatch, String> {
    let sorted = sorted_column_types(column_types);
    let ids: Vec<u64> = docs.iter().map(|d| d.id).collect();
    let texts: Vec<&str> = docs.iter().map(|d| d.text.as_str()).collect();

    let mut columns: Vec<ArrayRef> = Vec::with_capacity(schema.fields().len());
    columns.push(Arc::new(UInt64Array::from(ids)) as ArrayRef);
    columns.push(Arc::new(StringArray::from(texts)) as ArrayRef);

    for (name, ty) in &sorted {
        let values: Vec<Option<String>> = docs
            .iter()
            .map(|d| {
                let (typed, _) = split_metadata_for_write(&d.metadata, column_types);
                typed.get(name).cloned()
            })
            .collect();
        columns.push(encode_typed_column(name, ty.kind, &values)?);
    }

    let mut meta_json = Vec::with_capacity(docs.len());
    for doc in docs {
        let (_, overflow) = split_metadata_for_write(&doc.metadata, column_types);
        if overflow.is_empty() {
            meta_json.push(None);
        } else {
            meta_json.push(Some(
                serde_json::to_string(&overflow).map_err(|e| e.to_string())?,
            ));
        }
    }
    columns.push(Arc::new(StringArray::from(meta_json)) as ArrayRef);

    let embedding_arr = build_embedding_array(docs)?;
    columns.push(embedding_arr);

    RecordBatch::try_new(schema.clone(), columns).map_err(|e| e.to_string())
}

fn build_embedding_array(docs: &[ColumnarDoc]) -> Result<ArrayRef, String> {
    if docs.iter().any(|d| d.embedding.is_some()) {
        let mut offsets = vec![0i32];
        let mut values = Vec::new();
        for doc in docs {
            if let Some(ref emb) = doc.embedding {
                values.extend(emb.iter().copied());
            }
            offsets.push(values.len() as i32);
        }
        let values_arr = Arc::new(Float32Array::from(values));
        let offsets_arr = OffsetBuffer::new(offsets.into());
        Ok(Arc::new(ListArray::new(
            Arc::new(Field::new("item", DataType::Float32, true)),
            offsets_arr,
            values_arr,
            None,
        )) as ArrayRef)
    } else {
        Ok(Arc::new(ListArray::new_null(
            Arc::new(Field::new("item", DataType::Float32, true)),
            docs.len(),
        )) as ArrayRef)
    }
}

pub fn row_metadata_from_batch(
    batch: &RecordBatch,
    column_types: &[(String, ColumnTypeSpec)],
    row: usize,
) -> Result<HashMap<String, String>, String> {
    let schema = batch.schema();
    if super::typed_schema::is_legacy_arrow_schema(schema.as_ref()) {
        return legacy_json_metadata(batch, row);
    }

    let mut out = HashMap::new();
    if let Some(col) = batch.column_by_name("metadata_json") {
        let meta = col
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or("metadata_json type")?;
        if !meta.is_null(row) {
            let overflow: HashMap<String, String> =
                serde_json::from_str(meta.value(row)).map_err(|e| e.to_string())?;
            for (k, v) in overflow {
                out.insert(k.to_ascii_lowercase(), v);
            }
        }
    }
    for (name, ty) in sorted_column_types(column_types) {
        if let Some(col) = batch.column_by_name(&name) {
            if let Some(v) = decode_typed_value(ty.kind, col.as_ref(), row) {
                out.insert(name, v);
            }
        }
    }
    Ok(out)
}

fn legacy_json_metadata(
    batch: &RecordBatch,
    row: usize,
) -> Result<HashMap<String, String>, String> {
    let meta = batch
        .column_by_name("metadata_json")
        .ok_or("missing metadata_json column")?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or("metadata_json type")?;
    if meta.is_null(row) {
        return Ok(HashMap::new());
    }
    serde_json::from_str(meta.value(row)).map_err(|e| e.to_string())
}

pub fn infer_column_types_from_batch(batch: &RecordBatch) -> Vec<(String, ColumnTypeSpec)> {
    let schema = batch.schema();
    if super::typed_schema::is_legacy_arrow_schema(schema.as_ref()) {
        return Vec::new();
    }
    schema
        .fields()
        .iter()
        .filter_map(|f| {
            let name = f.name();
            if matches!(name.as_str(), "id" | "text" | "metadata_json" | "embedding") {
                return None;
            }
            Some((
                name.clone(),
                ColumnTypeSpec::new(arrow_type_to_column_type(f.data_type())),
            ))
        })
        .collect()
}

fn arrow_type_to_column_type(dt: &DataType) -> ColumnType {
    match dt {
        DataType::Int64 => ColumnType::Int,
        DataType::Float64 => ColumnType::Float,
        DataType::Boolean => ColumnType::Bool,
        DataType::Date32 => ColumnType::Date,
        DataType::Timestamp(TimeUnit::Millisecond, _) => ColumnType::Timestamp,
        DataType::List(_) => ColumnType::Vector,
        _ => ColumnType::Text,
    }
}

fn effective_column_types<'a>(
    batch: &RecordBatch,
    column_types: &'a [(String, ColumnTypeSpec)],
) -> Vec<(String, ColumnTypeSpec)> {
    if column_types.is_empty() {
        infer_column_types_from_batch(batch)
    } else {
        sorted_column_types(column_types)
    }
}

pub fn batch_to_docs(
    batch: &RecordBatch,
    column_types: &[(String, ColumnTypeSpec)],
) -> Result<Vec<ColumnarDoc>, String> {
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
    let emb_col = batch.column_by_name("embedding");
    let types = effective_column_types(batch, column_types);

    let mut out = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let metadata = row_metadata_from_batch(batch, &types, row)?;
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

pub fn batch_to_id_metadata(
    batch: &RecordBatch,
    column_types: &[(String, ColumnTypeSpec)],
) -> Result<Vec<(u64, HashMap<String, String>)>, String> {
    let ids = batch
        .column_by_name("id")
        .ok_or("missing id column")?
        .as_any()
        .downcast_ref::<UInt64Array>()
        .ok_or("id type")?;
    let types = effective_column_types(batch, column_types);
    let mut out = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        let metadata = row_metadata_from_batch(batch, &types, row)?;
        out.push((ids.value(row), metadata));
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

/// Build docs batch for legacy schema (all metadata in JSON).
pub fn docs_to_legacy_batch(
    schema: &Arc<Schema>,
    docs: &[ColumnarDoc],
) -> Result<RecordBatch, String> {
    docs_to_batch(schema, &[], docs)
}
