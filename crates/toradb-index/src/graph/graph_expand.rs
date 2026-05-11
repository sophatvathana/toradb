use toradb_core::CandidateSet;

use crate::corpus::CorpusStore;

pub fn expand(
    store: &CorpusStore,
    table: &str,
    candidates: &CandidateSet,
    depth: u32,
) -> CandidateSet {
    let Some(t) = store.table(table) else {
        return candidates.clone();
    };
    let mut out = candidates.clone();
    for &id in &candidates.ids {
        let n = t.neighbors(id, depth);
        for (i, nid) in n.ids.iter().enumerate() {
            if out.len() < 256 {
                out.push(*nid, n.scores[i]);
            }
        }
    }
    out
}
