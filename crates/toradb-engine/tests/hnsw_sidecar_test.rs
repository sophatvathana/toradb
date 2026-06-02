use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

fn unit_vector(i: u64, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0; dim];
    v[i as usize % dim] = 1.0;
    v
}

#[test]
fn hnsw_sidecar_written_on_flush_and_used_on_reload() {
    let dir = std::env::temp_dir().join("toradb_hnsw_sidecar");
    let _ = std::fs::remove_dir_all(&dir);
    let dim = 8;

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40u64)
            .map(|i| IngestDoc {
                text: format!("doc {i}"),
                metadata: Default::default(),
                vector: Some(unit_vector(i, dim)),
                sparse: None,
            })
            .collect();
        dag.add_documents("embeddings", docs).expect("add");
    }

    let hnsw_sidecar = dir.join("embeddings/indexes/hnsw.bin");
    assert!(
        hnsw_sidecar.exists(),
        "hnsw graph sidecar should exist after flush"
    );

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "embeddings".into();
    batch.query_vector = Some(unit_vector(39, dim));
    batch.tier1_enable_dense = true;
    batch.tier1_enable_sparse = false;
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());
    assert_eq!(batch.candidates.ids[0], 39);

    let _ = std::fs::remove_dir_all(&dir);
}
