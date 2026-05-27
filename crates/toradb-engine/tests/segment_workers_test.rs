use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn manifest_segment_workers_cap_parallel_scan() {
    let dir = std::env::temp_dir().join("toradb_segment_workers");
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

    let base = dir.as_path();
    persist::mark_table_segment_only(base, "docs").expect("segment_only");
    persist::rebuild_segment_sidecars(base, "docs", true, false).expect("sidecars");
    persist::set_table_segment_workers(base, "docs", 2).expect("set workers");
    assert_eq!(persist::table_segment_workers(base, "docs").expect("read"), 2);

    let mut dag = DagRunner::open_with_reload(&dir, false).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla motor".into();
    batch.tier1_enable_sparse = true;
    batch.distributed_segments = true;
    let metrics = dag.run(&mut batch, &toradb_core::ExecCtx::new(200, 50, 20));
    assert!(metrics.segments_scanned >= 1);
    assert_eq!(metrics.segment_workers, 2);
    assert!(!batch.candidates.is_empty());

    let alter = parse("ALTER TABLE docs SET SEGMENT_WORKERS = 6").unwrap();
    let toradb_sql::ast::Stmt::AlterTableSetSegmentWorkers { workers, .. } = &alter[0] else {
        panic!("alter");
    };
    assert_eq!(*workers, 6);
    dag.set_segment_workers("docs", *workers).expect("apply");
    assert_eq!(persist::table_segment_workers(base, "docs").expect("read"), 6);

    let _ = std::fs::remove_dir_all(&dir);
}
