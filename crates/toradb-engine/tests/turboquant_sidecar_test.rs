use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;

fn make_vec(seed: u64) -> Vec<f32> {
    (0..64)
        .map(|i| ((seed.wrapping_mul(31).wrapping_add(i as u64)) as f32 * 0.013).sin())
        .collect()
}

// Single test: `std::env::set_var` is process-global, so we can't reliably run
// "on" and "off" cases as parallel tests. Sequence both flows in one process.
#[test]
fn turboquant_sidecar_respects_env_flag() {
    // --- default off ---
    std::env::remove_var("TORADB_VECTOR_CODEC");
    std::env::remove_var("TORADB_TURBOQUANT_BITS");
    let dir_off = tempfile::tempdir().unwrap();
    let mut dag = DagRunner::open(dir_off.path()).unwrap();
    dag.add_documents(
        "emb_default",
        vec![IngestDoc {
            text: "vec doc".into(),
            metadata: Default::default(),
            vector: Some(vec![0.5, 1.0, 1.5, 2.0]),
        }],
    )
    .unwrap();
    assert!(
        !persist::table_has_turboquant_sidecars(dir_off.path(), "emb_default").unwrap(),
        "TQ sidecar should be off when env var is unset"
    );

    // --- enabled ---
    std::env::set_var("TORADB_VECTOR_CODEC", "turboquant_ip");
    std::env::set_var("TORADB_TURBOQUANT_BITS", "3");
    let dir_on = tempfile::tempdir().unwrap();
    let mut dag = DagRunner::open(dir_on.path()).unwrap();
    let docs: Vec<IngestDoc> = (0..16)
        .map(|i| IngestDoc {
            text: format!("vec doc {i}"),
            metadata: Default::default(),
            vector: Some(make_vec(i as u64)),
        })
        .collect();
    dag.add_documents("emb_tq", docs).unwrap();
    assert!(
        persist::table_has_turboquant_sidecars(dir_on.path(), "emb_tq").unwrap(),
        "expected at least one .vectors.tq.bin sidecar"
    );
    let snaps = persist::load_turboquant_sidecars(dir_on.path(), "emb_tq").unwrap();
    assert!(!snaps.is_empty(), "no TQ snapshots loaded");
    let total: usize = snaps.iter().map(|s| s.len()).sum();
    assert_eq!(total, 16, "expected 16 vectors across TQ sidecars");

    std::env::remove_var("TORADB_VECTOR_CODEC");
    std::env::remove_var("TORADB_TURBOQUANT_BITS");
}
