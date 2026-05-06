use toradb_core::{CompressionConfig, IndexMode, Schema, TableManifest};
use toradb_storage::ManifestStore;

#[test]
fn manifest_swap_increments_snapshot() {
    let mut store = ManifestStore::new();
    assert_eq!(store.snapshot_id(), 0);
    store.swap_manifest(TableManifest {
        name: "docs".into(),
        schema: Schema::default(),
        index_mode: IndexMode::Text,
        vector_dim: None,
        sparse_enabled: true,
        graph_enabled: false,
        compression: CompressionConfig::default(),
        segments: vec![],
    });
    assert_eq!(store.snapshot_id(), 1);
    assert!(store.catalog().get("docs").is_some());
}
