use toradb_core::{Batch, CandidateSet, ExecCtx};

use crate::corpus::CorpusStore;
use crate::dense::{diskann, hnsw, ivf, matryoshka, muvera, turboquant};
use crate::filter::{bitmap, metadata};
use crate::graph::{acorn, graph_expand};
use crate::sparse::{seismic, splade, wand};

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

        push_cap(
            &mut merged,
            self.store
                .table(table)
                .map(|t| t.bm25_search(q, cap))
                .unwrap_or_default(),
        );
        push_cap(&mut merged, splade::search(&self.store, table, q, cap));
        push_cap(&mut merged, seismic::search(&self.store, table, q, cap));
        push_cap(&mut merged, wand::search(&self.store, table, q, cap));

        if !query_vec.is_empty() {
            push_cap(&mut merged, hnsw::search(&self.store, table, query_vec, cap));
            push_cap(&mut merged, ivf::search(&self.store, table, query_vec, cap));
            push_cap(&mut merged, diskann::search(&self.store, table, query_vec, cap));
            push_cap(&mut merged, turboquant::search(&self.store, table, query_vec, cap));
            push_cap(&mut merged, muvera::search(&self.store, table, query_vec, cap));
            push_cap(&mut merged, matryoshka::search(&self.store, table, query_vec, cap));
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

    pub fn run_tier2(&self, batch: &mut Batch, ctx: &ExecCtx) {
        let table = &batch.table;
        if table.is_empty() {
            return;
        }
        let depth = if batch.graph_expand {
            batch.graph_depth.max(1)
        } else {
            1
        };
        let expanded = graph_expand::expand(&self.store, table, &batch.candidates, depth);
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
