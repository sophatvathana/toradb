use toradb_core::{Batch, ExecCtx};
use toradb_index::RetrievalRuntime;

#[test]
fn tier1_respects_budget() {
    let rt = RetrievalRuntime::new();
    let mut batch = Batch::new();
    let ctx = ExecCtx::new(5, 3, 2);
    rt.run_tier1(&mut batch, &ctx);
    assert!(batch.candidates.len() <= 5);
}
