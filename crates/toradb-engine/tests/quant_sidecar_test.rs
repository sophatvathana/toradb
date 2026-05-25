use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

#[test]
fn quant_sidecar_written_on_flush() {
    let dir = tempfile::tempdir().unwrap();
    let mut dag = DagRunner::open(dir.path()).unwrap();
    dag.add_documents(
        "emb",
        vec![IngestDoc {
            text: "vec doc".into(),
            metadata: Default::default(),
            vector: Some(vec![0.5, 1.0, 1.5]),
        }],
    )
    .unwrap();
    assert!(persist::table_has_quant_sidecars(dir.path(), "emb").unwrap());
}
