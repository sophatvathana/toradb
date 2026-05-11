use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;

/// WAND-style max-score retrieval; for in-memory tables this delegates to BM25 scoring.
pub fn search(store: &CorpusStore, table: &str, query: &str, k: usize) -> CandidateSet {
    store
        .table(table)
        .map(|t| t.bm25_search(query, k))
        .unwrap_or_default()
}
