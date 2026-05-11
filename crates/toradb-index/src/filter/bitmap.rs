use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;

pub fn filter(store: &CorpusStore, table: &str, tag: &str, k: usize) -> CandidateSet {
    let Some(t) = store.table(table) else {
        return CandidateSet::default();
    };
    let mut out = CandidateSet::with_capacity(k);
    for (&id, doc) in &t.docs {
        if out.len() >= k {
            break;
        }
        if doc.metadata.contains_key(tag) {
            out.push(id, 1.0);
        }
    }
    out
}
