//! BM25 ranking tests with English documents about Nikola Tesla.

use toradb_core::{Batch, ExecCtx, IngestOptions};
use toradb_index::{IngestDoc, RetrievalRuntime};

#[test]
fn bm25_ranks_matching_document_first() {
    let mut rt = RetrievalRuntime::new();
    rt.store.add_documents(
        "papers",
        vec![
            IngestDoc {
                text: "Nikola Tesla invented the alternating current induction motor and polyphase AC systems".into(),
                metadata: Default::default(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie studied radioactivity and won two Nobel prizes in physics and chemistry".into(),
                metadata: Default::default(),
                vector: None,
            },
        ],
        4,
        IngestOptions::default(),
    );
    let mut batch = Batch::new();
    batch.table = "papers".into();
    batch.query = "Nikola Tesla alternating current motor".into();
    let ctx = ExecCtx::new(10, 10, 5);
    rt.run_tier1(&mut batch, &ctx);
    assert!(!batch.candidates.is_empty());
    assert_eq!(batch.candidates.ids[0], 0);
}
