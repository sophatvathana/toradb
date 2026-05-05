use toradb_core::{Catalog, TableManifest};

#[derive(Debug, Default)]
pub struct ManifestStore {
    catalog: Catalog,
    active_snapshot: u64,
}

impl ManifestStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot_id(&self) -> u64 {
        self.active_snapshot
    }

    pub fn swap_manifest(&mut self, table: TableManifest) {
        self.catalog.register(table);
        self.active_snapshot = self.active_snapshot.wrapping_add(1);
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
}
