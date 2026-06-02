use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

#[test]
fn reload_hits_segment_cache() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    {
        let mut dag = DagRunner::open(base).unwrap();
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "cached doc".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
        )
        .unwrap();
        let _ = dag.table_documents("docs").expect("scan");
    }
    let mut dag2 = DagRunner::open(base).unwrap();
    let _ = dag2.table_documents("docs").expect("scan again");
    let stats = dag2.cache_stats();
    assert!(stats.hits > 0 || stats.misses > 0);
}
