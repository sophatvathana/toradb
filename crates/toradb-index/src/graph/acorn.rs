use toradb_core::CandidateSet;

/// Filtered approximate nearest neighbor refinement (stub).
pub fn refine(candidates: &CandidateSet, query: &str, cap: usize) -> CandidateSet {
    let mut out = CandidateSet::with_capacity(cap.min(candidates.len()));
    for (i, id) in candidates.ids.iter().enumerate() {
        if out.len() >= cap {
            break;
        }
        let score = candidates.scores[i];
        if query.is_empty() || score > 0.0 {
            out.push(*id, score);
        }
    }
    out
}
