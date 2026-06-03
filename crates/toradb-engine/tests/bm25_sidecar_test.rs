use toradb_engine::index_build_status::{
    segment_bm25_path, segment_sparse_up_to_date, IndexBuildManifest,
};
use toradb_engine::{persist, DagRunner};
use toradb_index::sparse::bm25_tbm3::Bm25Tbm3View;
use toradb_index::IngestDoc;

#[test]
fn bm25_sidecar_written_on_flush_and_used_on_reload() {
    let dir = std::env::temp_dir().join("toradb_bm25_sidecar");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla alternating current".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
        )
        .expect("add");
    }

    let table_sidecar = dir.join("docs/indexes/bm25.bin");
    let segment_sidecar = dir.join("docs/indexes/seg_00001.bm25.bin");
    assert!(
        table_sidecar.exists(),
        "table bm25 binary sidecar should exist after flush"
    );
    assert!(
        segment_sidecar.exists(),
        "per-segment bm25 binary sidecar should exist after flush"
    );

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla alternating current".into();
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn segment_sparse_up_to_date_without_build_manifest_entry() {
    let dir = std::env::temp_dir().join("toradb_sparse_skip_disk");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla coil".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
        )
        .expect("add");
    }

    let seg = "seg_00001.parquet";
    let parquet_path = dir.join("docs/segments").join(seg);
    let bm25_path = segment_bm25_path(&dir, "docs", seg);
    assert!(bm25_path.exists());

    let empty_manifest = IndexBuildManifest::default();
    assert!(segment_sparse_up_to_date(
        &dir,
        "docs",
        seg,
        &parquet_path,
        &empty_manifest
    ));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tbm3_sidecar_search_after_flush() {
    let dir = std::env::temp_dir().join("toradb_tbm3_sidecar");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
        )
        .expect("add");
    }

    let bin = dir.join("docs/indexes/seg_00001.bm25.bin");
    assert!(bin.exists(), "TBM3 BM25 sidecar should exist after flush");
    let bytes = std::fs::read(&bin).expect("read");
    let view = Bm25Tbm3View::open(&bytes).expect("parse");
    let hits = view.search("Nikola motor", 5);
    assert!(!hits.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn table_has_segment_bm25_sidecars_after_flush() {
    let dir = std::env::temp_dir().join("toradb_seg_sidecar_flag");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla coil".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
        )
        .expect("add");
    }

    assert!(persist::table_has_segment_bm25_sidecars(&dir, "docs").expect("check"));
    assert!(!persist::table_has_segment_bm25_sidecars(&dir, "missing").expect("check"));
    assert!(persist::table_segment_count(&dir, "docs").expect("count") >= 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_uses_per_segment_bm25_when_table_sidecar_missing() {
    let dir = std::env::temp_dir().join("toradb_seg_bm25_reload");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "Nikola Tesla wireless transmission".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
        )
        .expect("add");
    }

    std::fs::remove_file(dir.join("docs/indexes/bm25.bin")).expect("remove table sidecar");
    assert!(dir.join("docs/indexes/seg_00001.bm25.bin").exists());

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "docs".into();
    batch.query = "Nikola Tesla wireless".into();
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
