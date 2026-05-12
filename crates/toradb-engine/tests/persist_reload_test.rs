use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

#[test]
fn reload_restores_corpus_from_parquet() {
    let dir = std::env::temp_dir().join("toradb_persist_reload");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "papers",
            vec![IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: Default::default(),
                vector: None,
            }],
        )
        .expect("add");
    }

    let mut dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "papers".into();
    batch.query = "Nikola Tesla alternating current".into();
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());
    assert_eq!(batch.candidates.ids[0], 0);

    let _ = std::fs::remove_dir_all(&dir);
}
