use toradb_core::{Batch, CandidateSet, ExecCtx};

use crate::corpus::CorpusStore;
use crate::dense;
use crate::filter::{bitmap, metadata};
use crate::graph::{acorn, graph_expand};

#[derive(Debug, Default)]
pub struct RetrievalRuntime {
    pub store: CorpusStore,
}

impl RetrievalRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run_tier1(&self, batch: &mut Batch, ctx: &ExecCtx) {
        let table = &batch.table;
        if table.is_empty() || self.store.table(table).map(|t| t.len()).unwrap_or(0) == 0 {
            batch.candidates = CandidateSet::default();
            return;
        }
        let cap = ctx.tier1_budget as usize;
        let q = batch.query.as_str();
        let query_vec = batch.query_vector.as_deref().unwrap_or(&[]);
        let mut merged = CandidateSet::with_capacity(cap);

        let push_cap = |merged: &mut CandidateSet, c: CandidateSet| {
            for (i, id) in c.ids.iter().enumerate() {
                if merged.len() >= cap {
                    break;
                }
                merged.push(*id, c.scores[i]);
            }
        };

        if batch.tier1_enable_sparse {
            push_cap(
                &mut merged,
                self.store
                    .table(table)
                    .map(|t| t.bm25_search(q, cap))
                    .unwrap_or_default(),
            );
        }

        if batch.tier1_enable_dense && !query_vec.is_empty() {
            let dense = if batch.tier1_use_diskann {
                dense::diskann::search(&self.store, table, query_vec, cap)
            } else {
                dense::hnsw::search(&self.store, table, query_vec, cap)
            };
            push_cap(&mut merged, dense);
        }

        if metadata::parse_field_filter(q).is_some() {
            push_cap(&mut merged, metadata::filter(&self.store, table, q, cap));
        }
        if let Some(tag) = q.split_whitespace().find(|w| w.starts_with('#')) {
            push_cap(&mut merged, bitmap::filter(&self.store, table, &tag[1..], cap));
        }

        batch.candidates = merged;
    }

    pub fn segment_candidates(
        &self,
        table: &str,
        seg: u32,
        query: &str,
        ctx: &ExecCtx,
    ) -> CandidateSet {
        self.store
            .table(table)
            .map(|t| t.segment_bm25(query, seg, ctx.tier1_budget as usize))
            .unwrap_or_default()
    }

    pub fn segment_dense_candidates(
        &self,
        table: &str,
        seg: u32,
        query: &[f32],
        k: usize,
    ) -> CandidateSet {
        self.store.segment_vector_search(table, query, seg, k)
    }

    pub fn run_tier2(&self, batch: &mut Batch, ctx: &ExecCtx) {
        let table = &batch.table;
        if table.is_empty() {
            return;
        }
        let expanded = if batch.graph_expand {
            let depth = batch.graph_depth.max(1);
            graph_expand::expand(&self.store, table, &batch.candidates, depth)
        } else {
            CandidateSet::default()
        };
        let refined = acorn::refine(
            &self.store,
            table,
            &batch.candidates,
            &batch.query,
            ctx.tier2_budget as usize,
        );
        let cap = ctx.tier2_budget as usize;
        let mut merged = CandidateSet::with_capacity(cap);
        for c in [&refined, &expanded] {
            for (i, id) in c.ids.iter().enumerate() {
                if merged.len() < cap {
                    merged.push(*id, c.scores[i]);
                }
            }
        }
        batch.candidates = merged;
    }
}
