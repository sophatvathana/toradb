use toradb_engine::DagRunner;
use toradb_index::IngestDoc;
use toradb_storage::wal::{read_checkpoint, read_flushes};

#[test]
fn flush_batch_checkpoints_wal_after_manifest() {
    let dir = std::env::temp_dir().join("toradb_wal_checkpoint_flush");
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

    assert!(read_flushes(&dir, "docs").expect("read").is_empty());
    assert!(read_checkpoint(&dir, "docs").expect("cp").is_some());

    let _ = std::fs::remove_dir_all(&dir);
}
