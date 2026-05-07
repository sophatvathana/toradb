use toradb_core::CandidateSet;

pub fn filter(k: usize) -> CandidateSet {
    let mut c = CandidateSet::with_capacity(k);
    c.push(100, 1.0);
    c
}
