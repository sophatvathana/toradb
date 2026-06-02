use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

fn make_vec(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed.wrapping_mul(31).wrapping_add(i as u64)) as f32 * 0.013).sin())
        .collect()
}

#[test]
fn search_uses_turboquant_segments_after_reopen() {
    // Sequenced because std::env::set_var is process-global.
    std::env::set_var("TORADB_VECTOR_CODEC", "turboquant_ip");
    std::env::set_var("TORADB_TURBOQUANT_BITS", "4");
    // Use a wide HNSW search so the codec's accuracy isn't masked by graph misses.
    std::env::set_var("TORADB_HNSW_EF_SEARCH", "256");

    let dir = tempfile::tempdir().unwrap();

    // Ingest a corpus with enough vectors that HNSW activates.
    {
        let mut dag = DagRunner::open(dir.path()).unwrap();
        let docs: Vec<IngestDoc> = (0..64)
            .map(|i| IngestDoc {
                text: format!("vec doc {i}"),
                metadata: Default::default(),
                vector: Some(make_vec(i, 64)),
                sparse: None,
            })
            .collect();
        dag.add_documents("vecs", docs).unwrap();
    }

    assert!(
        persist::table_has_turboquant_sidecars(dir.path(), "vecs").unwrap(),
        "TQ sidecar should be written"
    );

    // Reopen — the engine should auto-load .vectors.tq.bin into the corpus.
    let dag = DagRunner::open(dir.path()).unwrap();
    let table = dag
        .retrieval
        .store
        .table("vecs")
        .expect("table after reopen");
    assert!(
        table.has_turboquant_segments(),
        "TQ segments should be installed on reopen"
    );

    // Use a query that isn't in the corpus to avoid self-IP ambiguity.
    let query: Vec<f32> = (0..64).map(|j| ((j as f32) * 0.041).cos()).collect();
    let candidates = table.vector_search(&query, 5);
    assert!(
        !candidates.ids.is_empty(),
        "expected non-empty search result"
    );

    // Brute-force ground truth from the same in-memory corpus.
    let mut truth: Vec<(u64, f32)> = (0..64u64)
        .map(|i| {
            let v = make_vec(i, 64);
            let dot: f32 = query.iter().zip(v.iter()).map(|(a, b)| a * b).sum();
            (i, dot)
        })
        .collect();
    truth.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let truth_top5: Vec<u64> = truth.iter().take(5).map(|(id, _)| *id).collect();

    // With EF_SEARCH=256 + full-precision re-rank, the codec should recover the
    // true top-1.
    assert_eq!(
        candidates.ids[0], truth_top5[0],
        "TQ search top-1 should match brute-force IP top-1; got {:?}, truth {:?}",
        candidates.ids, truth_top5
    );

    std::env::remove_var("TORADB_VECTOR_CODEC");
    std::env::remove_var("TORADB_TURBOQUANT_BITS");
    std::env::remove_var("TORADB_HNSW_EF_SEARCH");
}
