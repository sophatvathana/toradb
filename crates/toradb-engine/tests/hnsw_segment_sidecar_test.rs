use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

fn unit_vector(i: u64, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0; dim];
    v[i as usize % dim] = 1.0;
    v
}

#[test]
fn per_segment_hnsw_shards_persist_and_search_without_table_graph() {
    let dir = std::env::temp_dir().join("toradb_hnsw_segment_shards");
    let _ = std::fs::remove_dir_all(&dir);
    let dim = 8;

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40u64)
            .map(|i| IngestDoc {
                text: format!("doc {i}"),
                metadata: Default::default(),
                vector: Some(unit_vector(i, dim)),
            })
            .collect();
        dag.add_documents("embeddings", docs).expect("add");
    }

    assert!(
        persist::table_has_segment_hnsw_sidecars(&dir, "embeddings").expect("check"),
        "per-segment hnsw shards should exist after flush"
    );
    assert!(dir.join("embeddings/indexes/hnsw.bin").exists());

    std::fs::remove_file(dir.join("embeddings/indexes/hnsw.bin")).expect("drop table graph");

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "embeddings".into();
    batch.query_vector = Some(unit_vector(39, dim));
    batch.tier1_enable_dense = true;
    batch.tier1_enable_sparse = false;
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());
    assert!(
        batch.candidates.ids.iter().take(5).any(|&id| id == 39),
        "expected doc 39 in top-5 dense hits, got {:?}",
        &batch.candidates.ids[..batch.candidates.len().min(5)]
    );

    let _ = std::fs::remove_dir_all(&dir);
}
