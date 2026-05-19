use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

#[test]
fn list_tables_finds_manifest_dirs() {
    let dir = std::env::temp_dir().join("toradb_list_tables");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "articles",
            vec![IngestDoc {
                text: "Nikola Tesla coil".into(),
                metadata: Default::default(),
                vector: None,
            }],
        )
        .expect("add");
    }

    let names = persist::list_tables(&dir).expect("list");
    assert!(names.iter().any(|n| n == "articles"));

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let tables = dag2.list_tables().expect("list");
    assert!(tables.iter().any(|n| n == "articles"));

    let _ = std::fs::remove_dir_all(&dir);
}
