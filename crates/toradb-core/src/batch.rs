use crate::candidate::CandidateSet;

#[derive(Debug, Clone, Default)]
pub struct Batch {
    pub candidates: CandidateSet,
}

impl Batch {
    pub fn new() -> Self {
        Self::default()
    }
}
