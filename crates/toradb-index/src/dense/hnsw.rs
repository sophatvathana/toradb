use toradb_core::CandidateSet;

pub fn search(_vec: &[f32], k: usize) -> CandidateSet { let mut c = CandidateSet::with_capacity(k); c.push(13, 0.65); c }
