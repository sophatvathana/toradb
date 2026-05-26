use std::path::Path;

use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

fn tesla_docs(n: usize, offset: usize) -> Vec<IngestDoc> {
    (0..n)
        .map(|i| IngestDoc {
            text: format!(
                "Nikola Tesla document {} alternating current motor wireless",
                offset + i
            ),
            metadata: Default::default(),
            vector: None,
        })
        .collect()
}

fn segment_bm25_files(dir: &Path, table: &str) -> Vec<std::path::PathBuf> {
    let indexes = dir.join(table).join("indexes");
    if !indexes.is_dir() {
        return Vec::new();
    }
    std::fs::read_dir(&indexes)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".bm25.bin") && n.starts_with("seg_"))
        })
        .collect()
}

#[test]
fn bulk_ingest_finish_then_search_after_reopen() {
    let dir = std::env::temp_dir().join("toradb_bulk_ingest");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.begin_bulk_ingest("docs");
        for batch in 0..5 {
            dag.add_documents("docs", tesla_docs(1000, batch * 1000))
                .expect("add");
        }
        assert!(
            segment_bm25_files(&dir, "docs").is_empty(),
            "bulk ingest should defer per-segment BM25 until finish"
        );
        dag.finish_bulk_ingest("docs", false).expect("finish");
        assert!(
            !segment_bm25_files(&dir, "docs").is_empty(),
            "finish should build segment BM25 sidecars"
        );
    }

    let mut dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla alternating current motor".into();
    batch.tier1_enable_sparse = true;
    let ctx = toradb_core::ExecCtx::new(20, 10, 10);
    dag2.run(&mut batch, &ctx);
    assert!(
        !batch.candidates.is_empty(),
        "search should work after finish without manual create_index"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finish_bulk_ingest_requires_active_session() {
    let dir = std::env::temp_dir().join("toradb_bulk_ingest_err");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.ensure_table("docs");
    let err = dag.finish_bulk_ingest("docs", false).unwrap_err();
    assert!(err.contains("not in bulk ingest"));
    let _ = std::fs::remove_dir_all(&dir);
}
