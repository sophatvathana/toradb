use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_index::RetrievalRuntime;
use toradb_storage::SegmentManager;
use crate::advanced::apply_crag;
use crate::fusion::rrf_merge;
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

    pub fn add_documents(
        &mut self,
        table: &str,
        docs: Vec<toradb_index::IngestDoc>,
    ) -> usize {
        let n = self.segments.len() as u32;
        self.retrieval
            .store
            .add_documents(table, docs, n.max(1))
    }

    pub fn ensure_table(&mut self, table: &str) {
        self.retrieval.store.ensure_table(table);
    }

    pub fn run(&mut self, batch: &mut Batch, ctx: &ExecCtx) -> QueryMetrics {
        let mut metrics = QueryMetrics::default();

        if batch.enable_hyde && !batch.table.is_empty() {
            batch.query = self
                .retrieval
                .store
                .expand_query_terms(&batch.table, &batch.query);
        }

        self.retrieval.run_tier1(batch, ctx);
        metrics.tier1_candidates = batch.candidates.len() as u32;
        let t1 = lower_tier1();
        let _ = t1.execute(batch, ctx);

        let pre_tier2 = batch.candidates.clone();
        let t2 = lower_tier2();
        self.retrieval.run_tier2(batch, ctx);
        batch.candidates = rrf_merge(&pre_tier2, &batch.candidates, 60);
        if batch.enable_crag {
            apply_crag(batch);
        }
        metrics.tier2_candidates = t2.execute(batch, ctx) as u32;

        if !batch.table.is_empty() {
            let table = batch.table.clone();
            let query = batch.query.clone();
            let scheduler = SegmentScheduler::new(4);
            batch.candidates = scheduler.run_per_segment(&self.segments, |seg| {
                self.retrieval
                    .segment_candidates(&table, seg, &query, ctx)
            });
            SegmentScheduler::local_top_k(&mut batch.candidates, ctx.tier2_budget as usize);
        }

        let t3 = lower_tier3();
        metrics.tier3_candidates = t3.execute(batch, ctx) as u32;
        metrics
    }
}
