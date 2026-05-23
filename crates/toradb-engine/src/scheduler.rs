use rayon::prelude::*;

use toradb_core::{Batch, CandidateSet, DocId, ExecCtx};
use toradb_storage::SegmentManager;

/// Coordinator dispatches per-segment work to workers (sync thread pool, not Tokio hot path).
#[derive(Debug)]
pub struct SegmentScheduler {
    pub workers: usize,
}

impl SegmentScheduler {
    pub fn new(workers: usize) -> Self {
        Self {
            workers: workers.max(1),
        }
    }

    fn merge_local(into: &mut CandidateSet, local: CandidateSet) {
        for (i, id) in local.ids.iter().enumerate() {
            into.push(*id, local.scores[i]);
        }
    }

    pub fn run_for_segments<F>(&self, num_segments: u32, parallel: bool, f: F) -> CandidateSet
    where
        F: Fn(u32) -> CandidateSet + Sync,
    {
        let n = num_segments.max(1);
        let use_parallel = parallel && self.workers > 1 && n > 1;
        if use_parallel {
            let locals: Vec<CandidateSet> = (0..n).into_par_iter().map(|seg| f(seg)).collect();
            let mut merged = CandidateSet::with_capacity(1024);
            for local in locals {
                Self::merge_local(&mut merged, local);
            }
            return merged;
        }
        let mut merged = CandidateSet::with_capacity(1024);
        for seg in 0..n {
            Self::merge_local(&mut merged, f(seg));
        }
        merged
    }

    pub fn run_per_segment<F>(&self, segments: &SegmentManager, mut f: F) -> CandidateSet
    where
        F: FnMut(u32) -> CandidateSet,
    {
        let n = segments.len().max(1) as u32;
        let mut merged = CandidateSet::with_capacity(1024);
        for seg in 0..n {
            Self::merge_local(&mut merged, f(seg));
        }
        merged
    }

    pub fn local_top_k(candidates: &mut CandidateSet, k: usize) {
        if candidates.len() <= k {
            return;
        }
        let mut idx: Vec<usize> = (0..candidates.len()).collect();
        idx.sort_by(|&a, &b| {
            candidates.scores[b]
                .partial_cmp(&candidates.scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        idx.truncate(k);
        let mut new_ids = Vec::with_capacity(k);
        let mut new_scores = Vec::with_capacity(k);
        for i in idx {
            new_ids.push(candidates.ids[i]);
            new_scores.push(candidates.scores[i]);
        }
        candidates.ids = new_ids;
        candidates.scores = new_scores;
    }

    pub fn global_merge(locals: Vec<CandidateSet>, k: usize) -> CandidateSet {
        let mut merged = CandidateSet::with_capacity(k);
        for mut local in locals {
            Self::local_top_k(&mut local, k);
            for (i, id) in local.ids.iter().enumerate() {
                merged.push(*id, local.scores[i]);
            }
        }
        Self::local_top_k(&mut merged, k);
        merged
    }
}

pub fn execute_query(batch: &mut Batch, ctx: &ExecCtx, segments: &SegmentManager) {
    let scheduler = SegmentScheduler::new(4);
    let merged = scheduler.run_per_segment(segments, |_seg| {
        let mut c = CandidateSet::with_capacity(ctx.tier1_budget as usize);
        c.push(1 as DocId, 0.9);
        c
    });
    batch.candidates = merged;
}
