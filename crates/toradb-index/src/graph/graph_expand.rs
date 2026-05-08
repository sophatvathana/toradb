use toradb_core::CandidateSet;

pub fn expand(candidates: &CandidateSet, depth: u32) -> CandidateSet {
    let mut out = candidates.clone();
    for d in 0..depth {
        for id in candidates.ids.iter() {
            out.push(id + (d as u64) + 1, 0.5);
        }
    }
    out
}
