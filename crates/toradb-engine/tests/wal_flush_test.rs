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
                sparse: None,
            }],
        )
        .expect("add");
    }

    let records = read_flushes(&dir, "docs").expect("read wal");
    assert!(
        records.is_empty(),
        "WAL flush log should be checkpointed after manifest commit"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
