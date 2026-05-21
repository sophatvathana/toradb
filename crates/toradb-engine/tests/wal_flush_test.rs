use toradb_engine::DagRunner;
use toradb_index::IngestDoc;
use toradb_storage::wal::read_flushes;

#[test]
fn ingest_appends_wal_flush_record() {
    let dir = std::env::temp_dir().join("toradb_engine_wal");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla motor".into(),
                metadata: Default::default(),
                vector: None,
            }],
        )
        .expect("add");
    }

    let records = read_flushes(&dir, "docs").expect("read wal");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].doc_count, 1);
    assert!(records[0].segment.ends_with(".parquet"));

    let _ = std::fs::remove_dir_all(&dir);
}
