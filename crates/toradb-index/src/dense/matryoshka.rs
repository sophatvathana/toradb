use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;

/// Matryoshka-style search: use a prefix of the query vector when dimensions differ.
pub fn search(store: &CorpusStore, table: &str, query: &[f32], k: usize) -> CandidateSet {
    let Some(table) = store.table(table) else {
        return CandidateSet::default();
    };
    let dim = query.len().min(64);
    let q = &query[..dim];
    let mut scored = Vec::new();
    for (&id, doc) in &table.docs {
        let Some(ref v) = doc.vector else { continue };
        if v.len() < dim {
            continue;
        }
        let score: f32 = q.iter().zip(v[..dim].iter()).map(|(a, b)| a * b).sum();
        scored.push((id, score));
    }
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    let mut out = CandidateSet::with_capacity(scored.len());
    for (id, score) in scored {
        out.push(id, score);
    }
    out
}
