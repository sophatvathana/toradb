use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

fn unit_vector(i: u64, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0; dim];
    v[i as usize % dim] = 1.0;
    v
}

#[test]
fn diskann_sidecar_written_on_flush_and_used_on_reload() {
    let dir = std::env::temp_dir().join("toradb_diskann_sidecar");
    let _ = std::fs::remove_dir_all(&dir);
    let dim = 8;

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40u64)
            .map(|i| IngestDoc {
                text: format!("doc {i}"),
                metadata: Default::default(),
                vector: Some(unit_vector(i, dim)),
            })
            .collect();
        dag.add_documents("embeddings", docs).expect("add");
    }

    assert!(
        dir.join("embeddings/indexes/diskann.bin").exists(),
        "diskann graph sidecar should exist after flush"
    );

    std::fs::remove_file(dir.join("embeddings/indexes/hnsw.bin")).ok();
    for entry in std::fs::read_dir(dir.join("embeddings/indexes")).expect("read indexes") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("shard_") && name.ends_with(".hnsw.bin") {
            std::fs::remove_file(entry.path()).ok();
        }
    }

    let dag2 = DagRunner::open(&dir).expect("reopen");
    let mut batch = toradb_core::Batch::new();
    batch.table = "embeddings".into();
    batch.query_vector = Some(unit_vector(39, dim));
    batch.tier1_enable_dense = true;
    batch.tier1_enable_sparse = false;
    batch.tier1_use_diskann = true;
    let ctx = toradb_core::ExecCtx::new(10, 10, 5);
    dag2.retrieval.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());
    assert_eq!(batch.candidates.ids[0], 39);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_index_diskann_persists_sidecar() {
    let dir = std::env::temp_dir().join("toradb_create_index_diskann");
    let _ = std::fs::remove_dir_all(&dir);
    let dim = 8;

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs: Vec<IngestDoc> = (0..40u64)
            .map(|i| IngestDoc {
                text: format!("doc {i}"),
                metadata: Default::default(),
                vector: Some(unit_vector(i, dim)),
            })
            .collect();
        dag.add_documents("papers", docs).expect("add");
        std::fs::remove_file(dir.join("papers/indexes/diskann.bin")).ok();
        dag.create_index("papers", "DISKANN").expect("create index");
    }

    assert!(dir.join("papers/indexes/diskann.bin").exists());

    let _ = std::fs::remove_dir_all(&dir);
}
