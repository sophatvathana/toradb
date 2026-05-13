use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

#[test]
fn bm25_sidecar_written_on_flush_and_used_on_reload() {
    let dir = std::env::temp_dir().join("toradb_bm25_sidecar");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla alternating current".into(),
                metadata: Default::default(),
                vector: None,
            }],
        )
        .expect("add");
    }

    let sidecar = dir.join("docs/indexes/bm25.json");
    assert!(sidecar.exists(), "bm25 sidecar should exist after flush");

    let mut dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla alternating current".into();
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
