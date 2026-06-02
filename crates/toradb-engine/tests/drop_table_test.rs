use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

#[test]
fn drop_table_removes_disk_and_corpus() {
    let dir = std::env::temp_dir().join("toradb_drop_table");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "tmp",
            vec![IngestDoc {
                text: "to delete".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
        )
        .expect("add");
        assert!(dir.join("tmp/manifest.json").exists());
        dag.drop_table("tmp").expect("drop");
    }

    assert!(!dir.join("tmp").exists());
    let dag2 = DagRunner::open(&dir).expect("reopen");
    assert!(dag2.list_tables().expect("list").is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
