//! Parquet and JSONL file ingest helpers shared by CLI and platform API.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use arrow::array::StringArray;
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::dag::DagRunner;

/// Ingest Parquet file(s) from a path (file or directory of `.parquet` files).
pub fn ingest_parquet(
    dag: &mut DagRunner,
    table: &str,
    path: &Path,
    limit: u64,
) -> Result<u64, String> {
    let files = parquet_files(path)?;
    let mut total = 0u64;
    for file in files {
        if limit > 0 && total >= limit {
            break;
        }
        let f = File::open(&file).map_err(|e| e.to_string())?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(f).map_err(|e| e.to_string())?;
        let mut reader = builder.build().map_err(|e| e.to_string())?;
        for batch in reader.by_ref() {
            if limit > 0 && total >= limit {
                break;
            }
            let batch: RecordBatch = batch.map_err(|e| e.to_string())?;
            if batch.num_rows() == 0 {
                continue;
            }
            if limit > 0 {
                let remain = (limit - total) as usize;
                if batch.num_rows() > remain {
                    let batch = batch.slice(0, remain);
                    let added = dag.ingest_record_batch(table, &batch)? as u64;
                    total += added;
                    break;
                }
            }
            let added = dag.ingest_record_batch(table, &batch)? as u64;
            total += added;
        }
    }
    Ok(total)
}

/// Ingest newline-delimited JSON with a `text` / `passage` / `content` field per row.
pub fn ingest_jsonl(
    dag: &mut DagRunner,
    table: &str,
    path: &Path,
    batch_size: usize,
    limit: u64,
) -> Result<u64, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut texts: Vec<String> = Vec::with_capacity(batch_size);
    let mut total = 0u64;

    for line in reader.lines() {
        if limit > 0 && total >= limit {
            break;
        }
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let row: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let text = row
            .get("text")
            .or_else(|| row.get("passage"))
            .or_else(|| row.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        texts.push(text);
        total += 1;
        if texts.len() >= batch_size {
            flush_text_batch(dag, table, &mut texts)?;
        }
    }
    if !texts.is_empty() {
        flush_text_batch(dag, table, &mut texts)?;
    }
    Ok(total)
}

fn parquet_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("parquet") {
            out.push(p);
        }
    }
    out.sort();
    if out.is_empty() {
        return Err(format!("no parquet files under {}", path.display()));
    }
    Ok(out)
}

fn flush_text_batch(
    dag: &mut DagRunner,
    table: &str,
    texts: &mut Vec<String>,
) -> Result<(), String> {
    if texts.is_empty() {
        return Ok(());
    }
    let schema = arrow::datatypes::Schema::new(vec![arrow::datatypes::Field::new(
        "text",
        arrow::datatypes::DataType::Utf8,
        false,
    )]);
    let arr = StringArray::from(std::mem::take(texts));
    let batch = RecordBatch::try_new(std::sync::Arc::new(schema), vec![std::sync::Arc::new(arr)])
        .map_err(|e| e.to_string())?;
    dag.ingest_record_batch(table, &batch)?;
    Ok(())
}
