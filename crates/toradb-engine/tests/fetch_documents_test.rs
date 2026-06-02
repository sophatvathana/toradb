use toradb_engine::DagRunner;
use toradb_storage::columnar::{IndexMode, TableManifestFile};

#[test]
fn fetch_documents_by_ids_from_segment_only_table() {
    let dir = std::env::temp_dir().join("toradb_fetch_docs");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.begin_bulk_ingest("docs");
        dag.add_documents(
            "docs",
            vec![
                toradb_index::IngestDoc {
                    text: "alpha bravo document".into(),
                    metadata: [("tag".into(), "a".into())].into_iter().collect(),
                    vector: None,
                    sparse: None,
                },
                toradb_index::IngestDoc {
                    text: "charlie delta passage".into(),
                    metadata: Default::default(),
                    vector: None,
                    sparse: None,
                },
            ],
        )
        .expect("add");
        dag.finish_bulk_ingest("docs", false).expect("finish");
    }

    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(&dir, "docs"))
        .expect("manifest");
    assert_eq!(manifest.index_mode, IndexMode::SegmentOnly);

    let mut dag = DagRunner::open_with_reload(&dir, false).expect("reopen");
    let docs = dag.fetch_documents("docs", &[1]).expect("fetch");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].0, 1);
    assert!(docs[0].1.text.contains("charlie"));

    let _ = std::fs::remove_dir_all(&dir);
}
