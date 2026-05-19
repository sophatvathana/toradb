use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

#[test]
fn vector_sidecar_written_on_flush_and_used_on_reload() {
    let dir = std::env::temp_dir().join("toradb_vector_sidecar");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "papers",
            vec![
                IngestDoc {
                    text: "alpha".into(),
                    metadata: Default::default(),
                    vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
                },
                IngestDoc {
                    text: "beta".into(),
                    metadata: Default::default(),
                    vector: Some(vec![0.0, 1.0, 0.0, 0.0]),
                },
            ],
        )
        .expect("add");
    }

    let table_sidecar = dir.join("papers/indexes/vectors.bin");
    let segment_sidecar = dir.join("papers/indexes/seg_00001.vectors.bin");
    assert!(
        table_sidecar.exists(),
        "table vector sidecar should exist after flush"
    );
    assert!(
        segment_sidecar.exists(),
        "per-segment vector sidecar should exist after flush"
    );
    assert!(persist::table_has_segment_vector_sidecars(&dir, "papers").expect("check"));

    let mut dag2 = DagRunner::open(&dir).expect("reopen");
    let out = dag2
        .table_documents("papers")
        .expect("docs");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].1.vector.as_deref(), Some([1.0, 0.0, 0.0, 0.0].as_slice()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reload_uses_per_segment_vectors_when_table_sidecar_missing() {
    let dir = std::env::temp_dir().join("toradb_vector_seg_reload");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: "gamma".into(),
                metadata: Default::default(),
                vector: Some(vec![0.5, 0.5]),
            }],
        )
        .expect("add");
    }

    std::fs::remove_file(dir.join("docs/indexes/vectors.bin")).expect("remove table sidecar");

    let mut dag2 = DagRunner::open(&dir).expect("reopen");
    let docs = dag2.table_documents("docs").expect("docs");
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].1.vector.as_deref(), Some([0.5_f32, 0.5].as_slice()));

    let _ = std::fs::remove_dir_all(&dir);
}
