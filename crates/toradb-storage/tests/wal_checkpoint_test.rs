use std::path::Path;

use toradb_storage::columnar::{write_segment, ColumnarDoc, TableManifestFile};
use toradb_storage::wal::{append_flush, checkpoint_after_manifest, read_checkpoint, read_flushes};

#[test]
fn checkpoint_trims_flush_log_when_manifest_matches() {
    let dir = std::env::temp_dir().join("toradb_wal_checkpoint");
    let _ = std::fs::remove_dir_all(&dir);
    let base = Path::new(&dir);
    let table = "docs";
    let seg_name = "seg_00001.parquet";
    let seg_dir = TableManifestFile::segments_dir(base, table);
    std::fs::create_dir_all(&seg_dir).expect("seg dir");
    let seg_path = seg_dir.join(seg_name);
    write_segment(
        &seg_path,
        &[ColumnarDoc {
            id: 0,
            text: "hello".into(),
            metadata: Default::default(),
            embedding: None,
        }],
    )
    .expect("write");
    append_flush(base, table, seg_name, 0, 1, true).expect("wal");

    let mut manifest = TableManifestFile::default();
    manifest.push_segment(seg_name.to_string());
    manifest
        .save(&TableManifestFile::path_for_table(base, table))
        .expect("manifest");

    let trimmed =
        checkpoint_after_manifest(base, table, &manifest.segments, &seg_dir).expect("checkpoint");
    assert!(trimmed);
    assert!(read_flushes(base, table).expect("read").is_empty());
    let cp = read_checkpoint(base, table)
        .expect("cp")
        .expect("checkpoint file");
    assert_eq!(cp.last_segment, seg_name);

    let _ = std::fs::remove_dir_all(&dir);
}
