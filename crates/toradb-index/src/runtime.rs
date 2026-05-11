use toradb_core::{Batch, CandidateSet, DocId, ExecCtx};
use crate::dense::{diskann, hnsw, ivf, matryoshka, muvera, turboquant};
use crate::sparse::{bm25, seismic, splade, wand};
use crate::filter::{bitmap, metadata};
use crate::graph::{acorn, graph_expand};

#[derive(Debug, Default)]
pub struct RetrievalRuntime;

impl RetrievalRuntime {
    pub fn new() -> Self {
        Self
    }

    pub fn run_tier1(&self, batch: &mut Batch, ctx: &ExecCtx) {
        let cap = ctx.tier1_budget as usize;
        let mut merged = CandidateSet::with_capacity(cap);
        let q = if batch.query.is_empty() { "query" } else { batch.query.as_str() };
        for mut c in [
            bm25::search(q, cap),
            splade::search(q, cap),
            seismic::search(q, cap),
            wand::search(q, cap),
            turboquant::search(&[0.0; 8], cap),
            muvera::search(&[0.0; 8], cap),
            matryoshka::search(&[0.0; 8], cap),
            hnsw::search(&[0.0; 8], cap),
            ivf::search(&[0.0; 8], cap),
            diskann::search(&[0.0; 8], cap),
            bitmap::filter(cap),
            metadata::filter("year", cap),
        ] {
            for (i, id) in c.ids.iter().enumerate() {
                if merged.len() < cap {
                    merged.push(*id, c.scores[i]);
                }
            }
        }
        batch.candidates = merged;
    }

    pub fn segment_candidates(&self, seg: u32, ctx: &ExecCtx) -> CandidateSet {
        let mut c = CandidateSet::with_capacity(ctx.tier1_budget as usize);
        c.push(seg as DocId + 1, 1.0);
        c
    }

    pub fn run_tier2(&self, batch: &mut Batch, ctx: &ExecCtx) {
        let depth = if batch.graph_expand {
            batch.graph_depth.max(1)
        } else {
            1
        };
        let expanded = graph_expand::expand(&batch.candidates, depth);
        let refined = acorn::refine(&batch.candidates, &batch.query, ctx.tier2_budget as usize);
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
