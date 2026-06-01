use std::cmp::Ordering;

use crate::schema::DocId;

/// Bounded candidate set for tiered retrieval (hot path: no heap growth in steady state).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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

    pub fn sort_by_score(&mut self, descending: bool) {
        let mut pairs: Vec<(DocId, f32)> = self
            .ids
            .iter()
            .copied()
            .zip(self.scores.iter().copied())
            .collect();
        let cmp = |a: f32, b: f32| a.partial_cmp(&b).unwrap_or(Ordering::Equal);
        if descending {
            pairs.sort_by(|a, b| cmp(b.1, a.1));
        } else {
            pairs.sort_by(|a, b| cmp(a.1, b.1));
        }
        self.ids = pairs.iter().map(|(id, _)| *id).collect();
        self.scores = pairs.iter().map(|(_, s)| *s).collect();
    }

    pub fn reorder(&mut self, order: &[usize]) {
        let mut new_ids = Vec::with_capacity(order.len());
        let mut new_scores = Vec::with_capacity(order.len());
        for &idx in order {
            if idx < self.ids.len() {
                new_ids.push(self.ids[idx]);
                new_scores.push(self.scores[idx]);
            }
        }
        self.ids = new_ids;
        self.scores = new_scores;
    }

    pub fn retain_indices(&mut self, keep: &[usize]) {
        self.reorder(keep);
    }

    pub fn remove_ids(&mut self, deleted: &std::collections::HashSet<DocId>) {
        if deleted.is_empty() || self.ids.is_empty() {
            return;
        }
        let mut new_ids = Vec::with_capacity(self.ids.len());
        let mut new_scores = Vec::with_capacity(self.ids.len());
        for (i, &id) in self.ids.iter().enumerate() {
            if !deleted.contains(&id) {
                new_ids.push(id);
                new_scores.push(self.scores[i]);
            }
        }
        self.ids = new_ids;
        self.scores = new_scores;
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
