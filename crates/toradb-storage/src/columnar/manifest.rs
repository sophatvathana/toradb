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
}

impl Default for TableManifestFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            segments: Vec::new(),
            segment_workers: None,
            compression: None,
            index_mode: IndexMode::Merged,
        }
    }
}

impl TableManifestFile {
    pub fn set_index_mode(&mut self, mode: IndexMode) {
        self.index_mode = mode;
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
