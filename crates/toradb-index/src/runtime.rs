use toradb_core::{Batch, CandidateSet, DocId, ExecCtx};
use crate::dense::{diskann, hnsw, ivf, matryoshka, muvera, turboquant};
use crate::sparse::{bm25, seismic, splade, wand};
use crate::filter::{bitmap, metadata};
use crate::graph::{csr, graph_expand};

#[derive(Debug, Default)]
pub struct RetrievalRuntime;

impl RetrievalRuntime {
    pub fn new() -> Self {
        Self
    }

    pub fn run_tier1(&self, batch: &mut Batch, ctx: &ExecCtx) {
        let cap = ctx.tier1_budget as usize;
        let mut merged = CandidateSet::with_capacity(cap);
        for mut c in [
            bm25::search("query", cap),
            splade::search("query", cap),
            seismic::search("query", cap),
            wand::search("query", cap),
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
        let _ = graph_expand::expand(&batch.candidates, 2);
        batch.candidates.truncate(ctx.tier2_budget as usize);
    }
}
