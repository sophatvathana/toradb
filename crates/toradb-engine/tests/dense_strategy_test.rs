use toradb_core::{Batch, ExecCtx};
use toradb_engine::DagRunner;
use toradb_index::IngestDoc;

#[test]
fn dense_strategy_skips_bm25_tier1() {
    let mut dag = DagRunner::new();
    dag.add_documents(
        "v",
        vec![
            IngestDoc {
                text: "alpha beta".into(),
                metadata: Default::default(),
                vector: Some(vec![1.0, 0.0]),
            },
            IngestDoc {
                text: "gamma delta".into(),
                metadata: Default::default(),
                vector: Some(vec![0.0, 1.0]),
            },
        ],
    )
    .expect("add");

    let mut batch = Batch::new();
    batch.table = "v".into();
    batch.query = "gamma delta".into();
    batch.query_vector = Some(vec![0.0, 1.0]);
    batch.tier1_enable_sparse = false;
    batch.tier1_enable_dense = true;
    batch.graph_expand = false;

    let ctx = ExecCtx::new(100, 20, 1);
    dag.run(&mut batch, &ctx);
    assert_eq!(batch.candidates.ids.first().copied(), Some(1));
}
