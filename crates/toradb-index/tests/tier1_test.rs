//! Tier-1 budget tests with a Nikola Tesla themed corpus.

use toradb_core::{Batch, ExecCtx, IngestOptions};
use toradb_index::{IngestDoc, RetrievalRuntime};

#[test]
fn tier1_respects_budget() {
    let mut rt = RetrievalRuntime::new();
    for i in 0..20 {
        rt.store.add_documents(
            "t",
            vec![IngestDoc {
                text: format!(
                    "Document {i} about Nikola Tesla wireless power and Wardenclyffe experiments"
                ),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            }],
            4,
            IngestOptions::default(),
        );
    }
    let mut batch = Batch::new();
    batch.table = "t".into();
    batch.query = "Nikola Tesla wireless".into();
    let ctx = ExecCtx::new(5, 3, 2);
    rt.run_tier1(&mut batch, &ctx);
    assert!(batch.candidates.len() <= 5);
}
