//! In-memory HNSW-style graph for small/medium vector corpora

use std::cmp::Ordering;

use toradb_core::{CandidateSet, DocId};
use toradb_simd::dot_f32;

const M: usize = 16;
const EF_SEARCH: usize = 64;
const HNSW_MIN_DOCS: usize = 32;
/// Minimum vectors per logical segment shard before building a segment-local graph.
pub const SEGMENT_HNSW_MIN_DOCS: usize = 8;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct HnswIndex {
    dim: usize,
    ids: Vec<DocId>,
    vectors: Vec<Vec<f32>>,
    neighbors: Vec<Vec<usize>>,
    entry: usize,
}

impl HnswIndex {
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn build(ids: Vec<DocId>, vectors: Vec<Vec<f32>>) -> Option<Self> {
        if ids.is_empty() || ids.len() != vectors.len() {
            return None;
        }
        let dim = vectors[0].len();
        if dim == 0 || vectors.iter().any(|v| v.len() != dim) {
            return None;
        }
        let mut index = Self {
            dim,
            ids,
            vectors,
            neighbors: Vec::new(),
            entry: 0,
        };
        for i in 0..index.vectors.len() {
            index.insert_node(i);
        }
        Some(index)
    }

    fn insert_node(&mut self, idx: usize) {
        if idx == 0 {
            self.neighbors.push(Vec::new());
            self.entry = 0;
            return;
        }
        let mut scored: Vec<(usize, f32)> = (0..idx)
            .map(|j| (j, dot_f32(&self.vectors[idx], &self.vectors[j])))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        let nn: Vec<usize> = scored.into_iter().take(M).map(|(j, _)| j).collect();
        self.neighbors.push(nn.clone());
        for j in nn {
            let slot = &mut self.neighbors[j];
            if !slot.contains(&idx) {
                slot.push(idx);
                if slot.len() > M {
                    slot.sort_by(|a, b| {
                        let da = dot_f32(&self.vectors[idx], &self.vectors[*a]);
                        let db = dot_f32(&self.vectors[idx], &self.vectors[*b]);
                        db.partial_cmp(&da).unwrap_or(Ordering::Equal)
                    });
                    slot.truncate(M);
                }
            }
        }
        let q = &self.vectors[idx];
        if dot_f32(q, &self.vectors[self.entry]) > dot_f32(&self.vectors[self.entry], &self.vectors[self.entry]) {
            self.entry = idx;
        }
    }

    pub fn search(&self, query: &[f32], k: usize) -> CandidateSet {
        if self.ids.is_empty() || query.len() != self.dim {
            return CandidateSet::default();
        }
        let mut visited = vec![false; self.ids.len()];
        let mut candidates = vec![(self.entry, dot_f32(query, &self.vectors[self.entry]))];
        visited[self.entry] = true;
        let mut best: Vec<(usize, f32)> = Vec::new();

        for _ in 0..EF_SEARCH {
            if candidates.is_empty() {
                break;
            }
            candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
            let (cur, cur_score) = candidates.remove(0);
            best.push((cur, cur_score));
            for &nb in &self.neighbors[cur] {
                if visited[nb] {
                    continue;
                }
                visited[nb] = true;
                let s = dot_f32(query, &self.vectors[nb]);
                candidates.push((nb, s));
            }
        }

        best.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        best.truncate(k);
        let mut out = CandidateSet::with_capacity(best.len());
        for (idx, score) in best {
            out.push(self.ids[idx], score);
        }
        out
    }
}

pub fn should_use_hnsw(doc_count: usize) -> bool {
    doc_count >= HNSW_MIN_DOCS
}

pub fn should_use_segment_hnsw(doc_count: usize) -> bool {
    doc_count >= SEGMENT_HNSW_MIN_DOCS
}
