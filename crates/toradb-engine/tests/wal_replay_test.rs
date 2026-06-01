use toradb_engine::persist::{load_table, replay_flush_wal};
use toradb_index::CorpusStore;
use toradb_storage::columnar::{write_segment, ColumnarDoc, TableManifestFile};
use toradb_storage::wal::{append_flush, flush_log_path, read_flushes};

#[test]
fn replay_commits_wal_segment_missing_from_manifest() {
    let dir = std::env::temp_dir().join("toradb_wal_replay");
    let _ = std::fs::remove_dir_all(&dir);
    let base = &dir;
    let table = "papers";

    let seg_dir = TableManifestFile::segments_dir(base, table);
    std::fs::create_dir_all(&seg_dir).expect("segments dir");
    let seg_name = "seg_00001.parquet";
    write_segment(
        &seg_dir.join(seg_name),
        &[ColumnarDoc {
            id: 0,
            text: "Nikola Tesla motor".into(),
            metadata: Default::default(),
            embedding: None,
        }],
    )
    .expect("write segment");
    append_flush(base, table, seg_name, 0, 1, true).expect("wal");

    let recovered = replay_flush_wal(base, table).expect("replay");
    assert_eq!(recovered, 1);

    let manifest =
        TableManifestFile::load(&TableManifestFile::path_for_table(base, table)).expect("manifest");
    assert_eq!(manifest.segments, vec![seg_name.to_string()]);
    assert!(!flush_log_path(base, table).exists());

    let mut store = CorpusStore::default();
    let n = load_table(base, table, &mut store, 4, None).expect("load");
    assert_eq!(n, 1);
    assert!(!store.docs_with_ids_since(table, 0).is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_after_crash_before_manifest_recovers_docs() {
    let dir = std::env::temp_dir().join("toradb_wal_crash_sim");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = toradb_engine::DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![toradb_index::IngestDoc {
                text: "alternating current".into(),
                metadata: Default::default(),
                vector: None,
            }],
        )
        .expect("add");
        // Simulate crash after WAL but before indexes: manifest should still be updated
        // in our flush path; instead simulate orphan WAL by stripping manifest entry.
        let manifest_path = TableManifestFile::path_for_table(&dir, "docs");
        let mut manifest = TableManifestFile::load(&manifest_path).expect("manifest");
        manifest.segments.clear();
        manifest.save(&manifest_path).expect("strip manifest");
        append_flush(&dir, "docs", "seg_00001.parquet", 0, 1, true).expect("wal");
    }

    let mut dag2 = toradb_engine::DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "alternating current".into();
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());

    let records = read_flushes(&dir, "docs").expect("wal");
    assert!(records.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
