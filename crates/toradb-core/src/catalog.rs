use crate::compression::{CompressionConfig, IndexMode};
use crate::schema::{Schema, SegmentId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TableManifest {
    pub name: String,
    pub schema: Schema,
    pub index_mode: IndexMode,
    pub vector_dim: Option<u32>,
    pub sparse_enabled: bool,
    pub graph_enabled: bool,
    pub compression: CompressionConfig,
    pub segments: Vec<SegmentId>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    tables: std::collections::HashMap<String, TableManifest>,
}

impl Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, manifest: TableManifest) {
        self.tables.insert(manifest.name.clone(), manifest);
    }

    pub fn get(&self, name: &str) -> Option<&TableManifest> {
        self.tables.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut TableManifest> {
        self.tables.get_mut(name)
    }

    pub fn list_tables(&self) -> Vec<&str> {
        self.tables.keys().map(|s| s.as_str()).collect()
    }
}
