use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

#[test]
fn create_index_hnsw_persists_sidecar() {
    let dir = std::env::temp_dir().join("toradb_create_index_hnsw");
    let _ = std::fs::remove_dir_all(&dir);
    let dim = 4;

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40u64)
            .map(|i| {
                let mut v = vec![0.0; dim];
                v[i as usize % dim] = 1.0;
                IngestDoc {
                    text: format!("doc {i}"),
                    metadata: Default::default(),
                    vector: Some(v),
                }
            })
            .collect();
        dag.add_documents("papers", docs).expect("add");
        std::fs::remove_file(dir.join("papers/indexes/hnsw.bin")).ok();
        dag.create_index("papers", "HNSW").expect("create index");
    }

    assert!(dir.join("papers/indexes/hnsw.bin").exists());

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "papers".into();
    let mut q = vec![0.0; dim];
    q[39 % dim] = 1.0;
    batch.query_vector = Some(q);
    batch.tier1_enable_dense = true;
    batch.tier1_enable_sparse = false;
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());
    assert!(
        batch.candidates.ids.contains(&39),
        "expected doc 39 in dense hits: {:?}",
        batch.candidates.ids
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_index_bm25_rebuilds_sparse() {
    let dir = std::env::temp_dir().join("toradb_create_index_bm25");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: Default::default(),
                vector: None,
            }],
        )
        .expect("add");
        dag.create_index("docs", "BM25").expect("create index");
    }

    assert!(dir.join("docs/indexes/bm25.bin").exists());

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla motor".into();
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
