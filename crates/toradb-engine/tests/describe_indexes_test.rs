use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

fn unit_vector(i: u64, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0; dim];
    v[i as usize % dim] = 1.0;
    v
}

#[test]
fn table_index_sidecars_lists_diskann_after_vector_flush() {
    let dir = std::env::temp_dir().join("toradb_describe_indexes");
    let _ = std::fs::remove_dir_all(&dir);
    let dim = 8;

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40u64)
            .map(|i| IngestDoc {
                text: format!("doc {i}"),
                metadata: Default::default(),
                vector: Some(unit_vector(i, dim)),
                sparse: None,
            })
            .collect();
        dag.add_documents("emb", docs).expect("add");
    }

    let sidecars = persist::table_index_sidecars(&dir, "emb").expect("sidecars");
    assert!(sidecars.iter().any(|s| s == "diskann"));
    assert!(sidecars.iter().any(|s| s == "vectors"));

    let _ = std::fs::remove_dir_all(&dir);
}
