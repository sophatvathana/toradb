use toradb_core::CandidateSet;

pub fn filter(_field: &str, k: usize) -> CandidateSet {
    let mut c = CandidateSet::with_capacity(k);
    c.push(101, 1.0);
    c
}
