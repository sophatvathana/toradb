use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_index::RetrievalRuntime;
use toradb_storage::SegmentManager;
use crate::lowering::{lower_tier1, lower_tier2, lower_tier3};
use crate::scheduler::SegmentScheduler;

/// Single execution path for all queries (SQL and SDK).
#[derive(Debug)]
pub struct DagRunner {
    pub retrieval: RetrievalRuntime,
    pub segments: SegmentManager,
}

impl DagRunner {
    pub fn new() -> Self {
        Self {
            retrieval: RetrievalRuntime::new(),
            segments: SegmentManager::new(),
        }
    }

    pub fn run(&mut self, batch: &mut Batch, ctx: &ExecCtx) -> QueryMetrics {
        let mut metrics = QueryMetrics::default();
        let t1 = lower_tier1();
        metrics.tier1_candidates = t1.execute(batch, ctx) as u32;
        self.retrieval.run_tier1(batch, ctx);

        let t2 = lower_tier2();
        metrics.tier2_candidates = t2.execute(batch, ctx) as u32;

        let scheduler = SegmentScheduler::new(4);
        batch.candidates = scheduler.run_per_segment(&self.segments, |seg| {
            self.retrieval.segment_candidates(seg, ctx)
        });
        SegmentScheduler::local_top_k(&mut batch.candidates, ctx.tier2_budget as usize);

        let t3 = lower_tier3();
        metrics.tier3_candidates = t3.execute(batch, ctx) as u32;
        metrics
    }
}
