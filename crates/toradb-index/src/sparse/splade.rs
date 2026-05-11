use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;
use crate::sparse::bm25::tokenize;

/// SPLADE-style expansion: search with extra terms from the query bigrams.
pub fn search(store: &CorpusStore, table: &str, query: &str, k: usize) -> CandidateSet {
    let Some(t) = store.table(table) else {
        return CandidateSet::default();
    };
    let terms = tokenize(query);
    let mut expanded = terms.clone();
    for i in 0..terms.len().saturating_sub(1) {
        expanded.push(format!("{}_{}", terms[i], terms[i + 1]));
    }
    let q = expanded.join(" ");
    t.bm25_search(&q, k)
}
