use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How sparse indexes are stored and queried for this table.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IndexMode {
    /// Merged `bm25.bin` loaded into memory at open.
    #[default]
    Merged,
    /// Per-segment `*.bm25.bin` only; query fans out without merging postings into RAM.
    SegmentOnly,
}

/// How sparse queries choose segments at runtime.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QueryMode {
    /// Scan every segment BM25 sidecar (legacy fan-out).
    #[default]
    SegmentFanout,
    /// Use `indexes/bm25.route.bin` to skip segments with no query-term overlap.
    Routed,
}

/// Tables with at least this many segments default to `QueryMode::Routed` after index build.
pub const ROUTED_QUERY_MIN_SEGMENTS: u32 = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentIdRange {
    pub file: String,
    pub min_id: u64,
    pub max_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TableManifestFile {
    pub schema_version: u32,
    pub segments: Vec<String>,
    /// Cap on rayon threads for distributed segment scans (default 4 when unset).
    #[serde(default)]
    pub segment_workers: Option<u32>,
    /// Parquet compression for new segments (optional).
    #[serde(default)]
    pub compression: Option<toradb_core::CompressionConfig>,
    #[serde(default)]
    pub index_mode: IndexMode,
    #[serde(default)]
    pub segment_id_ranges: Vec<SegmentIdRange>,
    #[serde(default)]
    pub query_mode: QueryMode,
}

impl Default for TableManifestFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            segments: Vec::new(),
            segment_workers: None,
            compression: None,
            index_mode: IndexMode::Merged,
            segment_id_ranges: Vec::new(),
            query_mode: QueryMode::default(),
        }
    }
}

impl TableManifestFile {
    pub fn set_index_mode(&mut self, mode: IndexMode) {
        self.index_mode = mode;
    }

    pub fn record_segment_id_range(&mut self, file: &str, min_id: u64, max_id: u64) {
        if let Some(r) = self.segment_id_ranges.iter_mut().find(|r| r.file == file) {
            r.min_id = min_id;
            r.max_id = max_id;
            return;
        }
        self.segment_id_ranges.push(SegmentIdRange {
            file: file.to_string(),
            min_id,
            max_id,
        });
    }

    pub fn id_range_for_segment(&self, file: &str) -> Option<(u64, u64)> {
        self.segment_id_ranges
            .iter()
            .find(|r| r.file == file)
            .map(|r| (r.min_id, r.max_id))
    }

    pub fn segments_for_ids<'a>(&'a self, ids: &[u64]) -> Vec<&'a str> {
        if self.segment_id_ranges.is_empty() {
            return self.segments.iter().map(|s| s.as_str()).collect();
        }
        let mut out: Vec<&str> = Vec::new();
        for id in ids {
            for r in &self.segment_id_ranges {
                if *id >= r.min_id && *id <= r.max_id {
                    let name = r.file.as_str();
                    if !out.contains(&name) {
                        out.push(name);
                    }
                }
            }
        }
        if out.is_empty() {
            self.segments.iter().map(|s| s.as_str()).collect()
        } else {
            out
        }
    }
}

impl TableManifestFile {
    pub fn path_for_table(base: &Path, table: &str) -> PathBuf {
        base.join(table).join("manifest.json")
    }

    pub fn segments_dir(base: &Path, table: &str) -> PathBuf {
        base.join(table).join("segments")
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&data).map_err(|e| e.to_string())
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn push_segment(&mut self, segment_name: String) {
        self.segments.push(segment_name);
    }
}
