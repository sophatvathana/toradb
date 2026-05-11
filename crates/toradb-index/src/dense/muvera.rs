use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;
use crate::dense::search;

pub fn search(store: &CorpusStore, table: &str, query: &[f32], k: usize) -> CandidateSet {
    search::search(store, table, query, k)
}
