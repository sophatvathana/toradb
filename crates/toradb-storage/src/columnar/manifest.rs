use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableManifestFile {
    pub schema_version: u32,
    pub segments: Vec<String>,
    /// Cap on rayon threads for distributed segment scans (default 4 when unset).
    #[serde(default)]
    pub segment_workers: Option<u32>,
}

impl Default for TableManifestFile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            segments: Vec::new(),
            segment_workers: None,
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
