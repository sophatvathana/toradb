use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use arrow::array::{Array, BooleanArray, StringArray, UInt64Array};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::{
    ArrowPredicateFn, ArrowReaderOptions, ParquetRecordBatchReaderBuilder, RowFilter, RowSelection,
};
use parquet::arrow::ProjectionMask;
use parquet::file::metadata::PageIndexPolicy;
use parquet::file::statistics::Statistics;
use toradb_core::{ColumnTypeSpec, CompressionConfig};

use super::metadata_codec::{batch_to_docs, batch_to_id_metadata};
use super::typed_schema::is_legacy_arrow_schema;
use super::writer::ColumnarDoc;

pub fn read_segment(path: &Path) -> Result<Vec<ColumnarDoc>, String> {
    read_segment_with_compression(path, None)
}

/// True when the Parquet file uses the legacy four-column layout (no native typed fields).
pub fn segment_uses_legacy_layout(path: &Path) -> Result<bool, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    Ok(is_legacy_arrow_schema(builder.schema().as_ref()))
}

/// Read rows whose id is in `want`. When `id_bounds` is set and ids are sequential in the
/// segment (`row = id - min_id`), uses Parquet row selection to skip unrelated rows.
pub fn read_segment_matching_ids(
    path: &Path,
    want: &HashSet<u64>,
    id_bounds: Option<(u64, u64)>,
) -> Result<Vec<ColumnarDoc>, String> {
    if want.is_empty() {
        return Ok(Vec::new());
    }
    if let Some((min_id, max_id)) = id_bounds {
        if want.iter().all(|id| (min_id..=max_id).contains(id)) {
            let ids: Vec<u64> = want.iter().copied().collect();
            if let Ok(docs) = read_segment_by_row_selection(path, &ids, min_id) {
                let returned: HashSet<u64> = docs.iter().map(|d| d.id).collect();
                if ids.len() == docs.len() && ids.iter().all(|id| returned.contains(id)) {
                    return Ok(docs);
                }
            }
        }
    }
    read_segment_by_id_filter(path, want)
}

/// Fast path: map doc id -> row offset via contiguous segment ids, then `RowSelection`.
fn read_segment_by_row_selection(
    path: &Path,
    ids: &[u64],
    min_id: u64,
) -> Result<Vec<ColumnarDoc>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let options = ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional);
    let builder = ParquetRecordBatchReaderBuilder::try_new_with_options(file, options)
        .map_err(|e| e.to_string())?;
    let total_rows = builder.metadata().file_metadata().num_rows() as usize;
    let selection = row_selection_for_ids(ids, min_id, total_rows)?;
    let mut reader = builder
        .with_row_selection(selection)
        .build()
        .map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(ids.len());
    for batch in reader.by_ref() {
        let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
        out.extend(batch_to_docs(&batch, &[])?);
    }
    Ok(out)
}

fn row_selection_for_ids(
    ids: &[u64],
    min_id: u64,
    total_rows: usize,
) -> Result<RowSelection, String> {
    let mut rows: Vec<usize> = ids
        .iter()
        .map(|id| id.saturating_sub(min_id) as usize)
        .filter(|row| *row < total_rows)
        .collect();
    if rows.is_empty() {
        return Err("no ids in segment row range".into());
    }
    rows.sort_unstable();
    rows.dedup();

    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut start = rows[0];
    let mut end = start + 1;
    for &row in rows.iter().skip(1) {
        if row == end {
            end += 1;
        } else {
            ranges.push(start..end);
            start = row;
            end = row + 1;
        }
    }
    ranges.push(start..end);
    Ok(RowSelection::from_consecutive_ranges(
        ranges.into_iter(),
        total_rows,
    ))
}

fn rg_id_min_max(stats: &Statistics) -> Option<(u64, u64)> {
    let min_bytes = stats.min_bytes_opt()?;
    let max_bytes = stats.max_bytes_opt()?;
    if min_bytes.len() < 8 || max_bytes.len() < 8 {
        return None;
    }
    let min = u64::from_le_bytes(min_bytes[..8].try_into().ok()?);
    let max = u64::from_le_bytes(max_bytes[..8].try_into().ok()?);
    Some((min, max))
}

fn row_selection_from_rg_stats(
    builder: &ParquetRecordBatchReaderBuilder<File>,
    want_min: u64,
    want_max: u64,
) -> Option<RowSelection> {
    let row_groups = builder.metadata().row_groups();
    if row_groups.is_empty() {
        return None;
    }
    let mut ranges: Vec<parquet::arrow::arrow_reader::RowSelector> = Vec::new();
    let mut any_skipped = false;
    for rg in row_groups {
        let n = rg.num_rows() as usize;
        let keep = rg
            .column(0) // id is always column 0
            .statistics()
            .and_then(|s| rg_id_min_max(s))
            .map(|(rg_min, rg_max)| rg_max >= want_min && rg_min <= want_max)
            .unwrap_or(true); // no stats → keep (safe default)
        if keep {
            ranges.push(parquet::arrow::arrow_reader::RowSelector::select(n));
        } else {
            ranges.push(parquet::arrow::arrow_reader::RowSelector::skip(n));
            any_skipped = true;
        }
    }
    if !any_skipped {
        return None;
    }
    Some(RowSelection::from(ranges))
}

fn read_segment_by_id_filter(path: &Path, want: &HashSet<u64>) -> Result<Vec<ColumnarDoc>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let options = ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional);
    let builder = ParquetRecordBatchReaderBuilder::try_new_with_options(file, options)
        .map_err(|e| e.to_string())?;

    let want_min = want.iter().copied().min().unwrap_or(0);
    let want_max = want.iter().copied().max().unwrap_or(u64::MAX);
    let rg_selection = row_selection_from_rg_stats(&builder, want_min, want_max);

    let schema = builder.parquet_schema();
    let want = Arc::new(want.clone());
    let id_mask = ProjectionMask::leaves(schema, [0]);
    let id_predicate = ArrowPredicateFn::new(id_mask, {
        let want = Arc::clone(&want);
        move |batch: RecordBatch| {
            let ids = batch
                .column(0)
                .as_any()
                .downcast_ref::<UInt64Array>()
                .ok_or_else(|| {
                    arrow::error::ArrowError::SchemaError("expected id column".into())
                })?;
            let mask: BooleanArray = (0..ids.len())
                .map(|i| want.contains(&ids.value(i)))
                .collect();
            Ok(mask)
        }
    });
    let row_filter = RowFilter::new(vec![Box::new(id_predicate)]);
    let mut builder = builder.with_row_filter(row_filter);
    if let Some(sel) = rg_selection {
        builder = builder.with_row_selection(sel);
    }
    let mut reader = builder.build().map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(want.len());
    for batch in reader.by_ref() {
        let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
        out.extend(batch_to_docs(&batch, &[])?);
        if out.len() >= want.len() {
            break;
        }
    }
    Ok(out)
}

/// Inclusive min/max doc id in a segment Parquet file (scans id column only).
pub fn read_segment_id_bounds(path: &Path) -> Result<(u64, u64), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    let mut reader = builder.build().map_err(|e| e.to_string())?;
    let mut min_id = u64::MAX;
    let mut max_id = 0u64;
    let mut any = false;
    for batch in reader.by_ref() {
        let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
        let ids = batch
            .column_by_name("id")
            .ok_or("missing id column")?
            .as_any()
            .downcast_ref::<UInt64Array>()
            .ok_or("id type")?;
        for row in 0..batch.num_rows() {
            let id = ids.value(row);
            min_id = min_id.min(id);
            max_id = max_id.max(id);
            any = true;
        }
    }
    if !any {
        return Err("segment has no ids".into());
    }
    Ok((min_id, max_id))
}

pub fn parquet_row_count(path: &Path) -> Result<usize, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    Ok(builder.metadata().file_metadata().num_rows() as usize)
}

/// Build a per-segment BM25 snapshot by streaming Parquet batches (no full-text Vec).
pub fn bm25_snapshot_from_segment(path: &Path) -> Result<toradb_index::Bm25Snapshot, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    let mut reader = builder.build().map_err(|e| e.to_string())?;
    let mut index = toradb_index::Bm25Builder::default();
    for batch in reader.by_ref() {
        let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
        for (id, text) in batch_to_text_rows(&batch)? {
            index.add(id, &text);
        }
    }
    Ok(index.finish())
}

pub fn read_segment_texts(path: &Path) -> Result<Vec<(u64, String)>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    let mut reader = builder.build().map_err(|e| e.to_string())?;

    let mut rows = Vec::new();
    for batch in reader.by_ref() {
        let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
        rows.extend(batch_to_text_rows(&batch)?);
    }
    Ok(rows)
}

pub fn scan_segment_id_metadata(
    path: &Path,
    column_types: &[(String, ColumnTypeSpec)],
    mut f: impl FnMut(u64, HashMap<String, String>) -> Result<(), String>,
) -> Result<(), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let options = ArrowReaderOptions::new().with_page_index_policy(PageIndexPolicy::Optional);
    let builder = ParquetRecordBatchReaderBuilder::try_new_with_options(file, options)
        .map_err(|e| e.to_string())?;
    let mut reader = builder.build().map_err(|e| e.to_string())?;
    for batch in reader.by_ref() {
        let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
        for (id, metadata) in batch_to_id_metadata(&batch, column_types)? {
            f(id, metadata)?;
        }
    }
    Ok(())
}

pub fn read_segment_with_compression(
    path: &Path,
    _compression: Option<&CompressionConfig>,
) -> Result<Vec<ColumnarDoc>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| e.to_string())?;
    let mut reader = builder.build().map_err(|e| e.to_string())?;

    let mut docs = Vec::new();
    for batch in reader.by_ref() {
        let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
        docs.extend(batch_to_docs(&batch, &[])?);
    }
    Ok(docs)
}

pub fn iter_segment_batches(
    path: &Path,
    batch_size: usize,
) -> Result<impl Iterator<Item = Result<RecordBatch, String>>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| e.to_string())?
        .with_batch_size(batch_size);
    let reader = builder.build().map_err(|e| e.to_string())?;
    Ok(reader.map(|r| r.map_err(|e| e.to_string())))
}

pub fn decode_segment_bytes(bytes: &[u8]) -> Result<Vec<ColumnarDoc>, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("seg.parquet");
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    read_segment(&path)
}

fn batch_to_text_rows(batch: &RecordBatch) -> Result<Vec<(u64, String)>, String> {
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

    let mut out = Vec::with_capacity(batch.num_rows());
    for row in 0..batch.num_rows() {
        out.push((ids.value(row), texts.value(row).to_string()));
    }
    Ok(out)
}

#[cfg(all(feature = "io-uring", target_os = "linux"))]
pub fn read_segment_io_uring(path: &Path) -> Result<Vec<ColumnarDoc>, String> {
    crate::io::read_file_io_uring(path).and_then(|b| decode_segment_bytes(&b))
}

#[cfg(not(all(feature = "io-uring", target_os = "linux")))]
pub fn read_segment_io_uring(path: &Path) -> Result<Vec<ColumnarDoc>, String> {
    read_segment(path)
}
