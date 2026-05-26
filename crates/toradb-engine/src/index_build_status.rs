//! On-disk index build progress for ingest/finish 

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use toradb_storage::columnar::TableManifestFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexBuildState {
    Building,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexBuildPhase {
    SegmentBm25,
    MergeBm25,
    TableIndexes,
    ReloadTexts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexBuildStatus {
    pub state: IndexBuildState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<IndexBuildPhase>,
    #[serde(default)]
    pub segments_done: u32,
    #[serde(default)]
    pub segments_total: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub updated_unix_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SegmentBuildRecord {
    pub segment: String,
    pub sparse_done: bool,
    #[serde(default)]
    pub parquet_mtime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexBuildManifest {
    pub segments: Vec<SegmentBuildRecord>,
}

pub fn build_status_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("build_status.json")
}

pub fn build_manifest_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("build_manifest.json")
}

fn indexes_dir(base: &Path, table: &str) -> PathBuf {
    base.join(table).join("indexes")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn read_index_build_status(base: &Path, table: &str) -> Option<IndexBuildStatus> {
    let path = build_status_path(base, table);
    if !path.exists() {
        return None;
    }
    let data = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn write_index_build_status(
    base: &Path,
    table: &str,
    status: &IndexBuildStatus,
) -> Result<(), String> {
    let dir = indexes_dir(base, table);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = build_status_path(base, table);
    let data = serde_json::to_string_pretty(status).map_err(|e| e.to_string())?;
    std::fs::write(&path, data).map_err(|e| e.to_string())
}

pub fn mark_index_building(
    base: &Path,
    table: &str,
    phase: IndexBuildPhase,
    segments_done: u32,
    segments_total: u32,
) -> Result<(), String> {
    write_index_build_status(
        base,
        table,
        &IndexBuildStatus {
            state: IndexBuildState::Building,
            phase: Some(phase),
            segments_done,
            segments_total,
            message: None,
            updated_unix_secs: now_secs(),
        },
    )
}

pub fn mark_index_ready(base: &Path, table: &str) -> Result<(), String> {
    write_index_build_status(
        base,
        table,
        &IndexBuildStatus {
            state: IndexBuildState::Ready,
            phase: None,
            segments_done: 0,
            segments_total: 0,
            message: None,
            updated_unix_secs: now_secs(),
        },
    )
}

pub fn mark_index_failed(base: &Path, table: &str, message: impl Into<String>) -> Result<(), String> {
    write_index_build_status(
        base,
        table,
        &IndexBuildStatus {
            state: IndexBuildState::Failed,
            phase: None,
            segments_done: 0,
            segments_total: 0,
            message: Some(message.into()),
            updated_unix_secs: now_secs(),
        },
    )
}

pub fn clear_index_build_status(base: &Path, table: &str) -> Result<(), String> {
    let path = build_status_path(base, table);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Tables with `state == building` in `indexes/build_status.json`.
pub fn scan_indexing_tables(base: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(base) else {
        return out;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(status) = read_index_build_status(base, &name) {
            if status.state == IndexBuildState::Building {
                out.push(name);
            }
        }
    }
    out
}

/// List table names that have a manifest.json (no toradb load).
pub fn list_tables_on_disk(base: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(base) else {
        return out;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if TableManifestFile::path_for_table(base, &name).exists() {
            out.push(name);
        }
    }
    out.sort();
    out
}

pub fn read_build_manifest(base: &Path, table: &str) -> IndexBuildManifest {
    let path = build_manifest_path(base, table);
    if !path.exists() {
        return IndexBuildManifest::default();
    }
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn write_build_manifest(
    base: &Path,
    table: &str,
    manifest: &IndexBuildManifest,
) -> Result<(), String> {
    let dir = indexes_dir(base, table);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = build_manifest_path(base, table);
    let data = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    std::fs::write(&path, data).map_err(|e| e.to_string())
}

pub fn parquet_mtime_secs(path: &Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn segment_sparse_up_to_date(
    base: &Path,
    table: &str,
    segment: &str,
    parquet_path: &Path,
    manifest: &IndexBuildManifest,
) -> bool {
    let bm25_path = segment_bm25_path(base, table, segment);
    let v2 = base
        .join(table)
        .join("indexes")
        .join(format!(
            "{}.bm25.v2.bin",
            segment.strip_suffix(".parquet").unwrap_or(segment)
        ));
    if !bm25_path.exists() && !v2.exists() {
        return false;
    }
    let pq_mtime = parquet_mtime_secs(parquet_path);
    if manifest
        .segments
        .iter()
        .find(|r| r.segment == segment)
        .is_some_and(|r| r.sparse_done && r.parquet_mtime_secs == pq_mtime)
    {
        return true;
    }
    // Sidecar on disk but manifest incomplete (interrupted resume) — trust mtime ordering.
    parquet_mtime_secs(&bm25_path) >= pq_mtime
}

pub fn segment_bm25_path(base: &Path, table: &str, segment: &str) -> PathBuf {
    base.join(table).join("indexes").join(format!(
        "{}.bm25.bin",
        segment.strip_suffix(".parquet").unwrap_or(segment)
    ))
}
