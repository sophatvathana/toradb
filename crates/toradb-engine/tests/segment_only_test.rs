use toradb_engine::DagRunner;
use toradb_storage::columnar::{IndexMode, TableManifestFile};

#[test]
fn bulk_finish_segment_only_skips_merged_bm25() {
    let dir = std::env::temp_dir().join("toradb_segment_only");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.begin_bulk_ingest("docs");
        dag.add_documents(
            "docs",
            vec![
                toradb_index::IngestDoc {
                    text: "Nikola Tesla alternating current motor".into(),
                    metadata: Default::default(),
                    vector: None,
                },
                toradb_index::IngestDoc {
                    text: "wireless power transmission experiments".into(),
                    metadata: Default::default(),
                    vector: None,
                },
            ],
        )
        .expect("add");
        dag.finish_bulk_ingest("docs", false).expect("finish");
    }

    let manifest =
        TableManifestFile::load(&TableManifestFile::path_for_table(&dir, "docs")).expect("manifest");
    assert_eq!(manifest.index_mode, IndexMode::SegmentOnly);
    assert!(
        !dir.join("docs/indexes/bm25.bin").exists(),
        "segment-only should not write merged bm25.bin"
    );
    assert!(
        toradb_engine::persist::table_has_segment_bm25_sidecars(&dir, "docs").expect("check")
    );

    let mut dag2 = DagRunner::open_with_reload(&dir, true).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla motor".into();
    batch.tier1_enable_sparse = true;
    batch.distributed_segments = true;
    // segment_only tables query via distributed segment sidecars when corpus is empty in RAM
    let ctx = toradb_core::ExecCtx::new(20, 10, 10);
    dag2.run(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
