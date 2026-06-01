use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Scan every segment BM25 sidecar.
    #[default]
    SegmentFanout,
    /// Use `indexes/bm25.route.bin` to skip segments with no query-term overlap.
    Routed,
}

/// Tables with at least this many segments default to `QueryMode::Routed` after index build.
pub const ROUTED_QUERY_MIN_SEGMENTS: u32 = 8;

pub const TIER_BYTE_BOUNDS: [u64; 4] = [
    4 * 1024 * 1024,
    16 * 1024 * 1024,
    64 * 1024 * 1024,
    u64::MAX,
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentIdRange {
    pub file: String,
    pub min_id: u64,
    pub max_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SegmentMeta {
    pub file: String,
    pub min_id: u64,
    pub max_id: u64,
    #[serde(default)]
    pub tier: u8,
    #[serde(default)]
    pub generation: u32,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub byte_size: u64,
    #[serde(default)]
    pub row_count: u64,
    #[serde(default)]
    pub deleted_count: u64,
}

impl SegmentMeta {
    pub fn tier_for_bytes(bytes: u64) -> u8 {
        for (i, &bound) in TIER_BYTE_BOUNDS.iter().enumerate() {
            if bytes < bound {
                return i as u8;
            }
        }
        3
    }
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
    #[serde(default)]
    pub segment_meta: Vec<SegmentMeta>,
    #[serde(default)]
    pub column_types: Vec<(String, toradb_core::ColumnTypeSpec)>,
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
            segment_meta: Vec::new(),
            column_types: Vec::new(),
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
        } else {
            self.segment_id_ranges.push(SegmentIdRange {
                file: file.to_string(),
                min_id,
                max_id,
            });
        }
        if let Some(m) = self.segment_meta.iter_mut().find(|m| m.file == file) {
            m.min_id = min_id;
            m.max_id = max_id;
        }
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

    pub fn push_segment_meta(&mut self, meta: SegmentMeta) {
        if !self.segments.contains(&meta.file) {
            self.segments.push(meta.file.clone());
        }
        if let Some(r) = self.segment_id_ranges.iter_mut().find(|r| r.file == meta.file) {
            r.min_id = meta.min_id;
            r.max_id = meta.max_id;
        } else {
            self.segment_id_ranges.push(SegmentIdRange {
                file: meta.file.clone(),
                min_id: meta.min_id,
                max_id: meta.max_id,
            });
        }
        if let Some(m) = self.segment_meta.iter_mut().find(|m| m.file == meta.file) {
            *m = meta;
        } else {
            self.segment_meta.push(meta);
        }
    }

    pub fn remove_segment(&mut self, file: &str) {
        self.segments.retain(|s| s != file);
        self.segment_id_ranges.retain(|r| r.file != file);
        self.segment_meta.retain(|m| m.file != file);
    }

    pub fn next_generation(&self) -> u32 {
        self.segment_meta
            .iter()
            .map(|m| m.generation)
            .max()
            .unwrap_or(0)
            + 1
    }

    pub fn migrate_to_segment_meta(&mut self, seg_dir: &Path) -> bool {
        if !self.segment_meta.is_empty() {
            return false;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for name in &self.segments {
            let range = self.segment_id_ranges.iter().find(|r| r.file == *name);
            let (min_id, max_id) = range.map(|r| (r.min_id, r.max_id)).unwrap_or((0, 0));
            let byte_size = seg_dir.join(name).metadata().map(|m| m.len()).unwrap_or(0);
            self.segment_meta.push(SegmentMeta {
                file: name.clone(),
                min_id,
                max_id,
                tier: 0,
                generation: 0,
                created_at: now,
                byte_size,
                row_count: 0,
                deleted_count: 0,
            });
        }
        true
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
        let mut manifest: Self = serde_json::from_str(&data).map_err(|e| e.to_string())?;
        if manifest.segment_meta.is_empty() && !manifest.segments.is_empty() {
            let seg_dir = path
                .parent()
                .map(|p| p.join("segments"))
                .unwrap_or_default();
            manifest.migrate_to_segment_meta(&seg_dir);
        }
        Ok(manifest)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut m = self.clone();
        if !m.segment_meta.is_empty() {
            m.schema_version = 2;
            // Keep legacy fields in sync so schema_version=1 readers still work.
            m.segments = m.segment_meta.iter().map(|s| s.file.clone()).collect();
            m.segment_id_ranges = m
                .segment_meta
                .iter()
                .map(|s| SegmentIdRange {
                    file: s.file.clone(),
                    min_id: s.min_id,
                    max_id: s.max_id,
                })
                .collect();
        }
        if !m.column_types.is_empty() {
            m.schema_version = m.schema_version.max(4);
        }
        let data = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn push_segment(&mut self, segment_name: String) {
        if !self.segments.contains(&segment_name) {
            self.segments.push(segment_name.clone());
        }
        if !self.segment_meta.iter().any(|m| m.file == segment_name) {
            self.segment_meta.push(SegmentMeta {
                file: segment_name,
                min_id: 0,
                max_id: 0,
                tier: 0,
                generation: 0,
                created_at: 0,
                byte_size: 0,
                row_count: 0,
                deleted_count: 0,
            });
        }
    }

    pub fn set_column_types(&mut self, types: Vec<(String, toradb_core::ColumnTypeSpec)>) {
        self.column_types = types;
    }

    pub fn column_type(&self, name: &str) -> Option<toradb_core::ColumnTypeSpec> {
        self.column_types
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, ty)| *ty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_meta_round_trip() {
        let meta = SegmentMeta {
            file: "seg_00001.parquet".to_string(),
            min_id: 0,
            max_id: 99,
            tier: 1,
            generation: 3,
            created_at: 1_700_000_000,
            byte_size: 5 * 1024 * 1024,
            row_count: 100,
            deleted_count: 0,
        };
        let manifest = TableManifestFile {
            schema_version: 2,
            segments: vec!["seg_00001.parquet".to_string()],
            segment_meta: vec![meta.clone()],
            segment_id_ranges: vec![SegmentIdRange {
                file: "seg_00001.parquet".to_string(),
                min_id: 0,
                max_id: 99,
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: TableManifestFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.segment_meta[0], meta);
    }

    #[test]
    fn migration_from_v1_manifest() {
        let json = r#"{
            "schema_version": 1,
            "segments": ["seg_00001.parquet", "seg_00002.parquet"],
            "segment_id_ranges": [
                {"file": "seg_00001.parquet", "min_id": 0, "max_id": 49},
                {"file": "seg_00002.parquet", "min_id": 50, "max_id": 99}
            ]
        }"#;
        let mut manifest: TableManifestFile = serde_json::from_str(json).unwrap();
        let migrated = manifest.migrate_to_segment_meta(Path::new("/nonexistent"));
        assert!(migrated);
        assert_eq!(manifest.segment_meta.len(), 2);
        assert_eq!(manifest.segment_meta[0].file, "seg_00001.parquet");
        assert_eq!(manifest.segment_meta[0].tier, 0);
        assert_eq!(manifest.segment_meta[0].min_id, 0);
        assert_eq!(manifest.segment_meta[0].max_id, 49);
        assert_eq!(manifest.segment_meta[1].file, "seg_00002.parquet");
        assert_eq!(manifest.segment_meta[1].min_id, 50);
        assert_eq!(manifest.segment_meta[1].max_id, 99);
    }

    #[test]
    fn column_types_round_trip_and_bump_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let mut manifest = TableManifestFile::default();
        manifest.set_column_types(vec![
            ("published".to_string(), toradb_core::ColumnTypeSpec::new(toradb_core::ColumnType::Date)),
            ("rank".to_string(), toradb_core::ColumnTypeSpec::new(toradb_core::ColumnType::Int)),
        ]);
        manifest.save(&path).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["schema_version"], 4);
        let parsed = TableManifestFile::load(&path).unwrap();
        assert_eq!(parsed.column_type("PUBLISHED"), Some(toradb_core::ColumnTypeSpec::new(toradb_core::ColumnType::Date)));
        assert_eq!(parsed.column_type("rank"), Some(toradb_core::ColumnTypeSpec::new(toradb_core::ColumnType::Int)));
        assert_eq!(parsed.column_type("missing"), None);
    }

    #[test]
    fn vector_dim_column_types_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let mut manifest = TableManifestFile::default();
        manifest.set_column_types(vec![(
            "embedding".to_string(),
            toradb_core::ColumnTypeSpec::parse("vector(384)"),
        )]);
        manifest.save(&path).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["column_types"][0][1], "vector:384");
        let parsed = TableManifestFile::load(&path).unwrap();
        let ty = parsed.column_type("embedding").unwrap();
        assert_eq!(ty.vector_dim, Some(384));
    }

    #[test]
    fn old_manifest_without_column_types_still_loads() {
        let json = r#"{
            "schema_version": 2,
            "segments": ["seg_00001.parquet"],
            "segment_id_ranges": [{"file": "seg_00001.parquet", "min_id": 0, "max_id": 9}],
            "segment_meta": [{"file":"seg_00001.parquet","min_id":0,"max_id":9,"tier":0,"generation":1,"created_at":1,"byte_size":1,"row_count":10,"deleted_count":0}]
        }"#;
        let manifest: TableManifestFile = serde_json::from_str(json).unwrap();
        assert!(manifest.column_types.is_empty());
        assert_eq!(manifest.column_type("anything"), None);
    }

    #[test]
    fn save_syncs_legacy_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let mut manifest = TableManifestFile::default();
        manifest.push_segment_meta(SegmentMeta {
            file: "seg_00001.parquet".to_string(),
            min_id: 0,
            max_id: 10,
            tier: 0,
            generation: 1,
            created_at: 100,
            byte_size: 1024,
            row_count: 11,
            deleted_count: 0,
        });
        manifest.save(&path).unwrap();
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(raw["segments"].as_array().unwrap().len() == 1);
        assert!(raw["segment_id_ranges"].as_array().unwrap().len() == 1);
        assert_eq!(raw["schema_version"], 2);
    }

    #[test]
    fn push_segment_meta_updates_all_fields() {
        let mut manifest = TableManifestFile::default();
        let meta = SegmentMeta {
            file: "seg_00001.parquet".to_string(),
            min_id: 5,
            max_id: 15,
            tier: 1,
            generation: 2,
            created_at: 999,
            byte_size: 8192,
            row_count: 11,
            deleted_count: 0,
        };
        manifest.push_segment_meta(meta.clone());
        assert_eq!(manifest.segments, vec!["seg_00001.parquet"]);
        assert_eq!(manifest.segment_id_ranges[0].min_id, 5);
        assert_eq!(manifest.segment_meta[0], meta);
    }
}
