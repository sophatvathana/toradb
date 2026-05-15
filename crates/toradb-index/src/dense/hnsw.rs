use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;

pub fn search(store: &CorpusStore, table: &str, query: &[f32], k: usize) -> CandidateSet {
    store
        .table(table)
        .map(|t| t.vector_search(query, k))
        .unwrap_or_default()
}
