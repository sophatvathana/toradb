use crate::schema::DocId;

/// Bounded candidate set for tiered retrieval (hot path: no heap growth in steady state).
#[derive(Debug, Clone, Default)]
pub struct CandidateSet {
    pub ids: Vec<DocId>,
    pub scores: Vec<f32>,
}

impl CandidateSet {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            ids: Vec::with_capacity(cap),
            scores: Vec::with_capacity(cap),
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn push(&mut self, id: DocId, score: f32) {
        self.ids.push(id);
        self.scores.push(score);
    }

    pub fn truncate(&mut self, max: usize) {
        self.ids.truncate(max);
        self.scores.truncate(max);
    }

    pub fn slice_range(&self, offset: usize, limit: usize) -> Self {
        if offset >= self.len() {
            return Self::default();
        }
        let end = (offset + limit).min(self.len());
        Self {
            ids: self.ids[offset..end].to_vec(),
            scores: self.scores[offset..end].to_vec(),
        }
    }
}
