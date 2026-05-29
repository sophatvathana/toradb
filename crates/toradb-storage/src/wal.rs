use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalFlushRecord {
    pub segment: String,
    pub since_id: u64,
    pub doc_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalCheckpoint {
    pub last_segment: String,
    pub committed_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalCompactionRecord {
    pub removed: Vec<String>,
    pub added: Vec<String>,
    #[serde(default)]
    pub added_tiers: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Wal {
    pub entries: u64,
}

pub fn flush_log_path(base: &Path, table: &str) -> PathBuf {
    base.join(table).join("wal").join("flush.jsonl")
}

pub fn checkpoint_path(base: &Path, table: &str) -> PathBuf {
    base.join(table).join("wal").join("checkpoint.json")
}

pub fn compaction_log_path(base: &Path, table: &str) -> PathBuf {
    base.join(table).join("wal").join("compaction.jsonl")
}

/// Append one compaction record and fsync.
pub fn append_compaction(
    base: &Path,
    table: &str,
    removed: &[String],
    added: &[String],
    added_tiers: &[u8],
) -> Result<(), String> {
    if removed.is_empty() && added.is_empty() {
        return Ok(());
    }
    let path = compaction_log_path(base, table);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let record = WalCompactionRecord {
        removed: removed.to_vec(),
        added: added.to_vec(),
        added_tiers: added_tiers.to_vec(),
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

pub fn read_compactions(base: &Path, table: &str) -> Result<Vec<WalCompactionRecord>, String> {
    let path = compaction_log_path(base, table);
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

pub fn truncate_compactions(base: &Path, table: &str) -> Result<(), String> {
    let path = compaction_log_path(base, table);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Append one flush record. When `sync` is true, fsync the log file.
pub fn append_flush(
    base: &Path,
    table: &str,
    segment: &str,
    since_id: u64,
    doc_count: usize,
    sync: bool,
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
    if sync {
        file.sync_all().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Fsync the flush WAL after buffered bulk appends.
pub fn sync_flush_log(base: &Path, table: &str) -> Result<(), String> {
    let path = flush_log_path(base, table);
    if !path.exists() {
        return Ok(());
    }
    let file = OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
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

pub fn read_checkpoint(base: &Path, table: &str) -> Result<Option<WalCheckpoint>, String> {
    let path = checkpoint_path(base, table);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string()).map(Some)
}

fn write_checkpoint(base: &Path, table: &str, last_segment: &str) -> Result<(), String> {
    let path = checkpoint_path(base, table);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let committed_unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let checkpoint = WalCheckpoint {
        last_segment: last_segment.to_string(),
        committed_unix_secs,
    };
    let bytes = serde_json::to_vec_pretty(&checkpoint).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// Remove the flush log when all records are reflected in the manifest.
pub fn truncate_flushes(base: &Path, table: &str) -> Result<(), String> {
    let path = flush_log_path(base, table);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// After manifest commit, checkpoint and trim WAL when every flush record is durable.
pub fn checkpoint_after_manifest(
    base: &Path,
    table: &str,
    manifest_segments: &[String],
    segment_dir: &Path,
) -> Result<bool, String> {
    let records = read_flushes(base, table)?;
    if records.is_empty() {
        return Ok(false);
    }
    let all_reconciled = records.iter().all(|r| {
        manifest_segments.contains(&r.segment) && segment_dir.join(&r.segment).exists()
    });
    if !all_reconciled {
        return Ok(false);
    }
    let last_segment = records
        .last()
        .map(|r| r.segment.as_str())
        .unwrap_or("");
    write_checkpoint(base, table, last_segment)?;
    truncate_flushes(base, table)?;
    Ok(true)
}
