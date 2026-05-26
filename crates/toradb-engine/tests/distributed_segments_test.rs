use toradb_engine::persist;
use toradb_engine::DagRunner;
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn distributed_sql_sets_parallel_segment_scan() {
    let dir = std::env::temp_dir().join("toradb_distributed_segments");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40)
            .map(|i| IngestDoc {
                text: format!("Nikola Tesla document {i} motor"),
                metadata: Default::default(),
                vector: None,
            })
            .collect();
        dag.add_documents("docs", docs).expect("add");
    }
    persist::mark_table_segment_only(&dir, "docs").expect("segment_only");
    persist::rebuild_segment_sidecars(&dir, "docs", true, false).expect("sidecars");

    let stmts = parse(
        "SELECT id FROM docs DISTRIBUTED SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.distributed);

    let mut dag = DagRunner::open_with_reload(&dir, false).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla motor".into();
    batch.tier1_enable_sparse = true;
    batch.distributed_segments = true;
    let ctx = toradb_core::ExecCtx::new(200, 50, 20);
    let metrics = dag.run(&mut batch, &ctx);
    assert!(metrics.segments_scanned >= 1);
    assert!(metrics.segment_workers > 1);
    assert!(!batch.candidates.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
