use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

#[test]
fn resume_index_build_is_idempotent_after_partial_finish() {
    let dir = std::env::temp_dir().join("toradb_resume_mid");
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.begin_bulk_ingest("docs");
        let docs: Vec<IngestDoc> = (0..100u64)
            .map(|i| IngestDoc {
                text: format!("document {i} keyword"),
                metadata: Default::default(),
                vector: None,
            })
            .collect();
        dag.add_documents("docs", docs).expect("add");
    }
    {
        let mut dag = DagRunner::open_with_reload(&dir, false).expect("reopen");
        dag.resume_index_build("docs", false).expect("resume1");
    }
    {
        let mut dag = DagRunner::open_with_reload(&dir, false).expect("reopen");
        dag.resume_index_build("docs", false).expect("resume2");
    }
    assert!(persist::table_has_segment_bm25_sidecars(&dir, "docs").unwrap_or(false));
    let _ = std::fs::remove_dir_all(&dir);
}
