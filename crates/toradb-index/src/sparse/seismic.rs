use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;
use crate::sparse::bm25::tokenize;

/// Long-token focused lexical search (SEISMIC-style posting filter).
pub fn search(store: &CorpusStore, table: &str, query: &str, k: usize) -> CandidateSet {
    let Some(t) = store.table(table) else {
        return CandidateSet::default();
    };
    let long: Vec<String> = tokenize(query).into_iter().filter(|t| t.len() >= 5).collect();
    if long.is_empty() {
        return t.bm25_search(query, k);
    }
    t.bm25_search(&long.join(" "), k)
}
