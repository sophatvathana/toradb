use std::path::Path;

use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_index::RetrievalRuntime;
use toradb_storage::SegmentManager;

use crate::advanced::apply_crag;
use crate::fusion::rrf_merge;
use crate::lowering::{lower_tier1, lower_tier2, lower_tier3};
use crate::persist::{self, DbPath};
use crate::scheduler::SegmentScheduler;

/// Single execution path for all queries (SQL and SDK).
#[derive(Debug)]
pub struct DagRunner {
    pub retrieval: RetrievalRuntime,
    pub segments: SegmentManager,
    db_path: Option<DbPath>,
}

impl DagRunner {
    pub fn new() -> Self {
        Self {
            retrieval: RetrievalRuntime::new(),
            segments: SegmentManager::new(),
            db_path: None,
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let db_path = DbPath::new(path.as_ref());
        std::fs::create_dir_all(db_path.as_path()).map_err(|e| e.to_string())?;
        let mut runner = Self::new();
        runner.db_path = Some(db_path);
        runner.reload_from_disk()?;
        Ok(runner)
    }

    pub fn reload_from_disk(&mut self) -> Result<usize, String> {
        let Some(ref path) = self.db_path else {
            return Ok(0);
        };
        self.retrieval.store = toradb_index::CorpusStore::default();
        persist::load_all(
            path.as_path(),
            &mut self.retrieval.store,
            self.segments.len(),
        )
    }

    pub fn add_documents(
        &mut self,
        table: &str,
        docs: Vec<toradb_index::IngestDoc>,
    ) -> Result<usize, String> {
        if docs.is_empty() {
            return Ok(0);
        }
        self.ensure_table(table);
        let since_id = self.retrieval.store.next_id(table);
        let n = self.segment_parallelism(table);
        let added = self
            .retrieval
            .store
            .add_documents(table, docs, n.max(1));
        if let Some(ref path) = self.db_path {
            persist::flush_new_docs(
                path.as_path(),
                table,
                &mut self.retrieval.store,
                since_id,
                n.max(1),
            )?;
        }
        Ok(added)
    }

    pub fn ensure_table(&mut self, table: &str) {
        self.retrieval.store.ensure_table(table);
    }

    pub fn drop_table(&mut self, table: &str) -> Result<(), String> {
        self.retrieval.store.remove_table(table);
        if let Some(ref path) = self.db_path {
            persist::drop_table(path.as_path(), table)?;
        }
        Ok(())
    }

    pub fn create_index(&mut self, table: &str, using: &str) -> Result<(), String> {
        let doc_count = self
            .retrieval
            .store
            .table(table)
            .map(|t| t.len())
            .unwrap_or(0);
        if doc_count == 0 {
            return Err(format!("table {table} not found or has no documents"));
        }
        match using.to_uppercase().as_str() {
            "BM25" | "SPARSE" | "TEXT" => {
                self.retrieval.store.rebuild_bm25(table);
            }
            "HNSW" | "VECTOR" | "DENSE" | "ANN" => {
                self.retrieval.store.rebuild_hnsw(table);
                self.retrieval
                    .store
                    .rebuild_segment_hnsw(table, self.segment_parallelism(table));
            }
            "DISKANN" => {
                self.retrieval.store.rebuild_diskann(table);
            }
            "HYBRID" => {
                self.retrieval.store.rebuild_bm25(table);
                self.retrieval.store.rebuild_hnsw(table);
                self.retrieval
                    .store
                    .rebuild_segment_hnsw(table, self.segment_parallelism(table));
                self.retrieval.store.rebuild_diskann(table);
            }
            other => return Err(format!("unsupported index type {other}")),
        }
        if let Some(ref path) = self.db_path {
            let base = path.as_path();
            let n = self.segment_parallelism(table);
            persist::save_table_indexes(base, table, &mut self.retrieval.store, n)?;
            let kind = using.to_uppercase();
            let sparse = matches!(kind.as_str(), "BM25" | "SPARSE" | "TEXT" | "HYBRID");
            let vectors = matches!(
                kind.as_str(),
                "HNSW" | "VECTOR" | "DENSE" | "ANN" | "DISKANN" | "HYBRID"
            );
            persist::rebuild_segment_sidecars(base, table, sparse, vectors)?;
        }
        Ok(())
    }

    pub fn vector_dim(&self, table: &str) -> Option<usize> {
        self.retrieval.store.vector_dim(table)
    }

    pub fn table_index_sidecars(&self, table: &str) -> Result<Vec<String>, String> {
        let Some(ref path) = self.db_path else {
            return Ok(Vec::new());
        };
        persist::table_index_sidecars(path.as_path(), table)
    }

    pub fn table_has_diskann_sidecar(&self, table: &str) -> bool {
        self.db_path
            .as_ref()
            .is_some_and(|p| persist::table_has_diskann_sidecar(p.as_path(), table))
    }

    fn segment_parallelism(&self, table: &str) -> u32 {
        if let Some(ref path) = self.db_path {
            persist::table_segment_count(path.as_path(), table)
                .unwrap_or(persist::DEFAULT_SEGMENT_PARALLELISM)
        } else {
            self.segments.len() as u32
        }
        .max(1)
    }

    pub fn db_path(&self) -> Option<&std::path::Path> {
        self.db_path.as_ref().map(|p| p.as_path())
    }

    pub fn list_tables(&self) -> Result<Vec<String>, String> {
        match self.db_path.as_ref() {
            Some(p) => persist::list_tables(p.as_path()),
            None => Ok(Vec::new()),
        }
    }

    pub fn table_documents(
        &self,
        table: &str,
    ) -> Result<Vec<(u64, toradb_index::IngestDoc)>, String> {
        crate::persist::table_documents(
            &self.retrieval.store,
            self.db_path(),
            table,
        )
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
            let run_segments = self
                .db_path
                .as_ref()
                .and_then(|p| persist::table_has_segment_bm25_sidecars(p.as_path(), &table).ok())
                .unwrap_or(false);
            if run_segments {
                let query = batch.query.clone();
                let num_segments = self.segment_parallelism(&table);
                let parallel = batch.distributed_segments;
                let scheduler = SegmentScheduler::new(num_segments as usize);
                metrics.segments_scanned = num_segments;
                metrics.segment_workers = if parallel && num_segments > 1 {
                    scheduler.workers as u32
                } else {
                    1
                };
                let seg_merged = scheduler.run_for_segments(num_segments, parallel, |seg| {
                    self.retrieval
                        .segment_candidates(&table, seg, &query, ctx)
                });
                if !seg_merged.is_empty() {
                    batch.candidates = seg_merged;
                    SegmentScheduler::local_top_k(&mut batch.candidates, ctx.tier2_budget as usize);
                }
            } else if batch.tier1_enable_dense {
                if let Some(ref path) = self.db_path {
                    let run_dense_shards = persist::table_has_segment_hnsw_sidecars(
                        path.as_path(),
                        &table,
                    )
                    .unwrap_or(false);
                    if run_dense_shards {
                        let query_vec = batch.query_vector.clone().unwrap_or_default();
                        if !query_vec.is_empty() {
                            let num_segments = self.segment_parallelism(&table);
                            let parallel = batch.distributed_segments;
                            let scheduler = SegmentScheduler::new(num_segments as usize);
                            let k = ctx.tier2_budget as usize;
                            metrics.segments_scanned = num_segments;
                            metrics.segment_workers = if parallel && num_segments > 1 {
                                scheduler.workers as u32
                            } else {
                                1
                            };
                            let seg_merged = scheduler.run_for_segments(num_segments, parallel, |seg| {
                                self.retrieval.segment_dense_candidates(
                                    &table,
                                    seg,
                                    &query_vec,
                                    k,
                                )
                            });
                            if !seg_merged.is_empty() {
                                batch.candidates = seg_merged;
                                SegmentScheduler::local_top_k(
                                    &mut batch.candidates,
                                    ctx.tier2_budget as usize,
                                );
                            }
                        }
                    }
                }
            }
        }

        let t3 = lower_tier3();
        metrics.tier3_candidates = t3.execute(batch, ctx) as u32;
        metrics
    }
}
