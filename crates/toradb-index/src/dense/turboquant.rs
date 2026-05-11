use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;
use crate::dense::search;

/// Reduced-dimension vector probe (8-d prefix) before full re-rank in tier 3.
pub fn search(store: &CorpusStore, table: &str, query: &[f32], k: usize) -> CandidateSet {
    if query.len() <= 8 {
        return search::search(store, table, query, k);
    }
    search::search(store, table, &query[..8], k)
}
