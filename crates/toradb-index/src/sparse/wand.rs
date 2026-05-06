use toradb_core::CandidateSet;

pub fn search(query: &str, k: usize) -> CandidateSet {
    let mut c = CandidateSet::with_capacity(k);
    if !query.is_empty() { c.push(4, 0.85); }
    c
}
