use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalFlushRecord {
    pub segment: String,
    pub since_id: u64,
    pub doc_count: usize,
}

#[derive(Debug, Default)]
pub struct Wal {
    pub entries: u64,
}

pub fn flush_log_path(base: &Path, table: &str) -> PathBuf {
    base.join(table).join("wal").join("flush.jsonl")
}

/// Append one flush record and fsync the log file.
pub fn append_flush(
    base: &Path,
    table: &str,
    segment: &str,
    since_id: u64,
    doc_count: usize,
) -> Result<(), String> {
    if doc_count == 0 {
        return Ok(());
    }
    let path = flush_log_path(base, table);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let record = WalFlushRecord {
        segment: segment.to_string(),
        since_id,
        doc_count,
    };
    let line = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

/// Read all flush records in append order.
pub fn read_flushes(base: &Path, table: &str) -> Result<Vec<WalFlushRecord>, String> {
    let path = flush_log_path(base, table);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).map_err(|e| e.to_string())?);
    }
    Ok(out)
}
