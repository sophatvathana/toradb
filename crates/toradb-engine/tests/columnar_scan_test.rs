use toradb_engine::{persist, DagRunner};
use toradb_index::{CorpusStore, IngestDoc};

#[test]
fn table_documents_reads_parquet_when_corpus_empty() {
    let dir = std::env::temp_dir().join("toradb_columnar_scan");
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

    let empty = CorpusStore::default();
    let docs = persist::table_documents(&empty, Some(&dir), "docs", None).expect("scan");
    assert_eq!(docs.len(), 1);
    assert!(docs[0].1.text.contains("Tesla"));

    let _ = std::fs::remove_dir_all(&dir);
}
