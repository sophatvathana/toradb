use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;

use toradb_core::{Batch, CandidateSet, DocId, ExecCtx, IngestOptions, QueryMetrics};
use toradb_index::RetrievalRuntime;
use toradb_storage::columnar::IndexMode;
use toradb_storage::SegmentManager;
use toradb_storage::StorageCaches;

use toradb_distributed::ClusterClient;

use crate::advanced::apply_crag;
use crate::fusion::rrf_merge;
use crate::lowering::{lower_tier1, lower_tier2, lower_tier3, tier3_rescore};
use crate::persist::{self, DbPath};
use crate::scheduler::SegmentScheduler;

#[derive(Debug, Clone, Copy)]
struct BulkDiskState {
    next_id: DocId,
}

/// Single execution path for all queries (SQL and SDK).
#[derive(Debug)]
pub struct DagRunner {
    pub retrieval: RetrievalRuntime,
    pub segments: SegmentManager,
    pub caches: StorageCaches,
    db_path: Option<DbPath>,
    bulk_tables: HashSet<String>,
    bulk_disk_state: HashMap<String, BulkDiskState>,
    /// When set, `DISTRIBUTED` uses multi-node segment RPC instead of local rayon only.
    cluster: Option<ClusterClient>,
}

fn segment_bm25_per_segment_k(ctx: &ExecCtx) -> usize {
    let fetch = ctx.tier3_budget as usize;
    fetch.saturating_mul(2).max(fetch).min(ctx.tier1_budget as usize)
}

impl DagRunner {
    pub fn new() -> Self {
        Self {
            retrieval: RetrievalRuntime::new(),
            segments: SegmentManager::new(),
            caches: StorageCaches::default_from_env(),
            db_path: None,
            bulk_tables: HashSet::new(),
            bulk_disk_state: HashMap::new(),
            cluster: ClusterClient::from_env(),
        }
    }

    pub fn set_cluster(&mut self, cluster: Option<ClusterClient>) {
        self.cluster = cluster;
    }

    pub fn cluster(&self) -> Option<&ClusterClient> {
        self.cluster.as_ref()
    }

    fn ingest_options_for(&self, table: &str) -> IngestOptions {
        if self.bulk_tables.contains(table) {
            IngestOptions::bulk()
        } else {
            IngestOptions::default()
        }
    }

    /// Defer per-batch index rebuilds and table index writes until [`Self::finish_bulk_ingest`].
    pub fn begin_bulk_ingest(&mut self, table: &str) {
        self.ensure_table(table);
        let next_id = self.retrieval.store.next_id(table);
        self.bulk_tables.insert(table.to_string());
        self.bulk_disk_state
            .insert(table.to_string(), BulkDiskState { next_id });
        if let Some(ref path) = self.db_path {
            let _ = crate::index_build_status::clear_index_build_status(path.as_path(), table);
            let _ = persist::mark_table_segment_only(path.as_path(), table);
        }
    }

    pub fn bulk_ingest_active(&self, table: &str) -> bool {
        self.bulk_tables.contains(table)
    }

    pub fn ensure_table_queryable(&self, table: &str) -> Result<(), String> {
        if self.bulk_ingest_active(table) {
            return Err(format!("bulk ingest in progress for table {table}"));
        }
        if let Some(ref path) = self.db_path {
            if let Some(status) = persist::read_table_index_build_status(path.as_path(), table) {
                if status.state == crate::index_build_status::IndexBuildState::Building {
                    return Err(format!("index build in progress for table {table}"));
                }
            }
        }
        Ok(())
    }

    /// Finalize indexes after bulk load (table sidecars + optional compaction).
    pub fn finish_bulk_ingest(&mut self, table: &str, compact: bool) -> Result<(), String> {
        if !self.bulk_tables.remove(table) {
            return Err(format!("table {table} is not in bulk ingest mode"));
        }
        let _had_bulk_disk = self.bulk_disk_state.remove(table).is_some();
        let Some(ref path) = self.db_path else {
            return Err("finish_bulk_ingest requires a local on-disk database".into());
        };
        let reload_texts = _had_bulk_disk
            && persist::table_index_mode(path.as_path(), table)? != IndexMode::SegmentOnly;
        let num_segments = self.segment_parallelism(table);
        persist::finalize_bulk_table_indexes(
            path.as_path(),
            table,
            &mut self.retrieval.store,
            num_segments,
            Some(&mut self.caches),
            reload_texts,
        )?;
        if compact {
            let _ = persist::maybe_compact_after_flush(
                path.as_path(),
                table,
                &mut self.retrieval.store,
                Some(&mut self.caches),
            )?;
        }
        Ok(())
    }

    /// Build or resume table indexes without an active bulk session.
    pub fn resume_index_build(&mut self, table: &str, compact: bool) -> Result<(), String> {
        let Some(ref path) = self.db_path else {
            return Err("resume_index_build requires a local on-disk database".into());
        };
        let num_segments = self.segment_parallelism(table);
        let _had_disk = self.bulk_disk_state.remove(table).is_some();
        let reload_texts =
            persist::table_index_mode(path.as_path(), table)? != IndexMode::SegmentOnly;
        persist::resume_table_indexes(
            path.as_path(),
            table,
            &mut self.retrieval.store,
            num_segments,
            Some(&mut self.caches),
            reload_texts,
        )?;
        if compact {
            let _ = persist::maybe_compact_after_flush(
                path.as_path(),
                table,
                &mut self.retrieval.store,
                Some(&mut self.caches),
            )?;
        }
        Ok(())
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_with_reload(path, true)
    }

    /// Open a database directory. When `reload` is false, skip loading tables into memory
    /// (use after removing a table on disk to avoid reloading millions of rows).
    pub fn open_with_reload(path: impl AsRef<Path>, reload: bool) -> Result<Self, String> {
        let db_path = DbPath::new(path.as_ref());
        std::fs::create_dir_all(db_path.as_path()).map_err(|e| e.to_string())?;
        let mut runner = Self::new();
        runner.db_path = Some(db_path.clone());
        if reload {
            runner.reload_from_disk()?;
        }
        runner.maybe_auto_resume_indexes()?;
        Ok(runner)
    }

    /// When `TORADB_AUTO_RESUME_INDEX=1`, resume any table stuck in `building` state.
    pub fn maybe_auto_resume_indexes(&mut self) -> Result<(), String> {
        let auto = std::env::var("TORADB_AUTO_RESUME_INDEX")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        if !auto {
            return Ok(());
        }
        let Some(ref path) = self.db_path else {
            return Ok(());
        };
        let base = path.as_path().to_path_buf();
        for table in persist::list_tables(&base)? {
            if let Some(status) = persist::read_table_index_build_status(&base, &table) {
                if status.state == crate::index_build_status::IndexBuildState::Building {
                    let _ = self.resume_index_build(&table, false);
                }
            }
        }
        Ok(())
    }

    /// Run index finalization on a background thread (opens a fresh [`DagRunner`] on the same path).
    pub fn finish_bulk_ingest_background(
        &mut self,
        table: &str,
        compact: bool,
    ) -> Result<std::thread::JoinHandle<Result<(), String>>, String> {
        if !self.bulk_tables.contains(table) {
            return Err(format!("table {table} is not in bulk ingest mode"));
        }
        let table = table.to_string();
        let path = self
            .db_path
            .as_ref()
            .ok_or("finish_bulk_ingest requires a local on-disk database")?
            .as_path()
            .to_path_buf();
        self.bulk_tables.remove(&table);
        let _ = self.bulk_disk_state.remove(&table);
        let handle = std::thread::spawn(move || {
            let mut dag = DagRunner::open_with_reload(&path, false)?;
            dag.resume_index_build(&table, compact)
        });
        Ok(handle)
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
            Some(&mut self.caches),
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
        let opts = self.ingest_options_for(table);
        let since_id = self.retrieval.store.next_id(table);
        let n = self.segment_parallelism(table);
        let num_segments = n.max(1);
        if opts.defer_table_indexes {
            if let Some(ref path) = self.db_path {
                let since_id = self
                    .bulk_disk_state
                    .get(table)
                    .map(|s| s.next_id)
                    .unwrap_or(since_id);
                let added = persist::flush_ingest_batch_disk_only(
                    path.as_path(),
                    table,
                    since_id,
                    &docs,
                    &opts,
                )?;
                if let Some(state) = self.bulk_disk_state.get_mut(table) {
                    state.next_id = state.next_id.saturating_add(added as u64);
                }
                return Ok(added);
            }
        }
        let added = self
            .retrieval
            .store
            .add_documents(table, docs, num_segments, opts);
        if let Some(ref path) = self.db_path {
            persist::flush_new_docs(
                path.as_path(),
                table,
                &mut self.retrieval.store,
                since_id,
                num_segments,
                Some(&mut self.caches),
                opts,
            )?;
        }
        Ok(added)
    }

    /// Ingest an Arrow record batch on the bulk disk-only path (no `IngestDoc` vec).
    pub fn ingest_record_batch(
        &mut self,
        table: &str,
        batch: &arrow::record_batch::RecordBatch,
    ) -> Result<usize, String> {
        if batch.num_rows() == 0 {
            return Ok(0);
        }
        self.ensure_table(table);
        let opts = self.ingest_options_for(table);
        let Some(ref path) = self.db_path else {
            return Err("ingest_record_batch requires a local on-disk database".into());
        };
        if !opts.defer_table_indexes {
            return Err("ingest_record_batch requires bulk ingest mode".into());
        }
        let since_id = self
            .bulk_disk_state
            .get(table)
            .map(|s| s.next_id)
            .unwrap_or_else(|| self.retrieval.store.next_id(table));
        let added = persist::flush_arrow_batch_disk_only(
            path.as_path(),
            table,
            since_id,
            batch,
            &opts,
        )?;
        if let Some(state) = self.bulk_disk_state.get_mut(table) {
            state.next_id = state.next_id.saturating_add(added as u64);
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

    fn segment_worker_count(&self, table: &str) -> u32 {
        if let Some(ref path) = self.db_path {
            persist::table_segment_workers(path.as_path(), table)
                .unwrap_or(persist::DEFAULT_SEGMENT_PARALLELISM)
        } else {
            persist::DEFAULT_SEGMENT_PARALLELISM
        }
        .max(1)
    }

    pub fn set_segment_workers(&self, table: &str, workers: u32) -> Result<(), String> {
        let Some(ref path) = self.db_path else {
            return Err("segment_workers requires a local on-disk database".into());
        };
        persist::set_table_segment_workers(path.as_path(), table, workers)
    }

    pub fn compact_table(&mut self, table: &str, full: bool) -> Result<toradb_storage::compaction::CompactReport, String> {
        let Some(ref path) = self.db_path else {
            return Err("COMPACT TABLE requires a local on-disk database".into());
        };
        let mode = if full {
            toradb_storage::compaction::CompactMode::Full
        } else {
            toradb_storage::compaction::CompactMode::Normal
        };
        let policy = toradb_storage::compaction::CompactPolicy::from_env();
        persist::compact_table(
            path.as_path(),
            table,
            Some(&mut self.retrieval.store),
            mode,
            &policy,
            Some(&mut self.caches),
        )
    }

    pub fn cache_stats(&self) -> toradb_storage::CacheHierarchy {
        self.caches.combined_stats()
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
        &mut self,
        table: &str,
    ) -> Result<Vec<(u64, toradb_index::IngestDoc)>, String> {
        let base = self.db_path().map(|p| p.to_path_buf());
        crate::persist::table_documents(
            &self.retrieval.store,
            base.as_deref(),
            table,
            Some(&mut self.caches),
        )
    }

    pub fn scan_table_id_metadata(
        &self,
        table: &str,
        f: impl FnMut(u64, &std::collections::HashMap<String, String>) -> Result<(), String>,
    ) -> Result<(), String> {
        let base = self.db_path().map(|p| p.to_path_buf());
        crate::persist::scan_table_id_metadata(&self.retrieval.store, base.as_deref(), table, f)
    }

    /// Row count without loading full Parquet when the table is segment-only on disk.
    pub fn table_row_count(&self, table: &str) -> Result<usize, String> {
        if let Some(t) = self.retrieval.store.table(table) {
            let n = t.len();
            if n > 0 {
                return Ok(n);
            }
        }
        if let Some(ref path) = self.db_path {
            return crate::persist::table_row_count_on_disk(
                &self.retrieval.store,
                path.as_path(),
                table,
            );
        }
        Ok(0)
    }

    /// Fetch stored text/metadata for specific doc ids (lazy Parquet read when not in RAM).
    pub fn fetch_documents(
        &mut self,
        table: &str,
        ids: &[u64],
    ) -> Result<Vec<(u64, toradb_index::IngestDoc)>, String> {
        let base = self.db_path().map(|p| p.to_path_buf());
        crate::persist::fetch_documents_by_ids(
            &self.retrieval.store,
            base.as_deref(),
            table,
            ids,
            Some(&mut self.caches),
        )
    }

    pub fn run(&mut self, batch: &mut Batch, ctx: &ExecCtx) -> QueryMetrics {
        let mut metrics = QueryMetrics::default();

        if !batch.table.is_empty() {
            if let Err(e) = self.ensure_table_queryable(&batch.table) {
                eprintln!("toradb: {e}");
                return metrics;
            }
        }

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
        let fusion_k = batch.fusion_k.max(1);
        batch.candidates = rrf_merge(&pre_tier2, &batch.candidates, fusion_k);
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
            let segment_only = self.db_path.as_ref().is_some_and(|path| {
                persist::table_index_mode(path.as_path(), &table)
                    .map(|m| m == IndexMode::SegmentOnly)
                    .unwrap_or(false)
            });
            let skip_segment_bm25 = !segment_only
                && persist::table_has_merged_bm25_in_memory(&self.retrieval.store, &table);
            if run_segments && !skip_segment_bm25 {
                let query = batch.query.clone();
                let num_segments = self.segment_parallelism(&table);
                let workers = self.segment_worker_count(&table);
                let parallel = batch.distributed_segments
                    || (segment_only && num_segments > 1 && workers > 1);
                let scheduler =
                    SegmentScheduler::new_with_numa(workers as usize, self.caches.numa);
                metrics.segment_workers = if parallel && num_segments > 1 && workers > 1 {
                    workers
                } else {
                    1
                };
                let use_disk_segments = segment_only;
                let seg_k = segment_bm25_per_segment_k(ctx);
                let caches = &self.caches;
                let seg_bins: Arc<Vec<Option<PathBuf>>> =
                    if use_disk_segments {
                        self.db_path
                            .as_ref()
                            .map(|p| {
                                persist::list_segment_bm25_bins(p.as_path(), &table)
                                    .unwrap_or_default()
                            })
                            .map(Arc::new)
                            .unwrap_or_else(|| Arc::new(Vec::new()))
                    } else {
                        Arc::new(Vec::new())
                    };
                let active_segments: Vec<u32> = if use_disk_segments {
                    self.db_path
                        .as_ref()
                        .map(|p| {
                            persist::filter_bm25_segment_indices(
                                p.as_path(),
                                &table,
                                num_segments,
                                &query,
                            )
                            .unwrap_or_else(|_| (0..num_segments).collect())
                        })
                        .unwrap_or_else(|| (0..num_segments).collect())
                } else {
                    (0..num_segments).collect()
                };
                metrics.segments_scanned = active_segments.len() as u32;
                let use_cluster = batch.distributed_segments
                    && self.cluster.is_some()
                    && use_disk_segments;
                let seg_merged = if use_cluster {
                    let cluster = self.cluster.as_ref().expect("cluster");
                    cluster
                        .segment_bm25_search(&table, &query, seg_k, &active_segments)
                        .unwrap_or_default()
                } else if parallel && workers > 1 && active_segments.len() > 1 {
                    let bins = Arc::clone(&seg_bins);
                    let table_c = table.clone();
                    let query_c = query.clone();
                    let scan = || -> Vec<CandidateSet> {
                        active_segments
                            .par_iter()
                            .map(|&seg| {
                                if use_disk_segments {
                                    if let Some(bin_path) =
                                        bins.get(seg as usize).and_then(|p| p.as_ref())
                                    {
                                        return persist::search_segment_bm25_at_path(
                                            bin_path,
                                            &query_c,
                                            seg_k,
                                            Some(caches),
                                        )
                                        .unwrap_or_default();
                                    }
                                    return CandidateSet::default();
                                }
                                self.retrieval.segment_candidates(
                                    &table_c, seg, &query_c, ctx,
                                )
                            })
                            .collect()
                    };
                    let locals = match rayon::ThreadPoolBuilder::new()
                        .num_threads(workers as usize)
                        .build()
                    {
                        Ok(pool) => pool.install(scan),
                        Err(_) => scan(),
                    };
                    let mut merged = CandidateSet::with_capacity(1024);
                    for local in locals {
                        SegmentScheduler::merge_local(&mut merged, local);
                    }
                    merged
                } else {
                    let mut merged = CandidateSet::with_capacity(1024);
                    for seg in active_segments {
                        let local = if use_disk_segments {
                            if let Some(bin_path) =
                                seg_bins.get(seg as usize).and_then(|p| p.as_ref())
                            {
                                persist::search_segment_bm25_at_path(
                                    bin_path,
                                    &query,
                                    seg_k,
                                    Some(caches),
                                )
                                .unwrap_or_default()
                            } else {
                                CandidateSet::default()
                            }
                        } else {
                            self.retrieval
                                .segment_candidates(&table, seg, &query, ctx)
                        };
                        SegmentScheduler::merge_local(&mut merged, local);
                    }
                    merged
                };
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
                            let workers = self.segment_worker_count(&table);
                            let parallel = batch.distributed_segments;
                            let scheduler =
                    SegmentScheduler::new_with_numa(workers as usize, self.caches.numa);
                            let k = ctx.tier2_budget as usize;
                            metrics.segments_scanned = num_segments;
                            metrics.segment_workers = if parallel && num_segments > 1 && workers > 1 {
                                workers
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

        if let Some(ref path) = self.db_path {
            tier3_rescore(
                path.as_path(),
                batch,
                ctx,
                &mut self.caches,
                &self.retrieval.store,
            );
        }
        let t3 = lower_tier3();
        metrics.tier3_candidates = t3.execute(batch, ctx) as u32;
        if let Some(ref path) = self.db_path {
            if !batch.table.is_empty()
                && persist::table_has_quant_sidecars(path.as_path(), &batch.table)
                    .unwrap_or(false)
            {
                metrics.decompressions = metrics.tier3_candidates;
            }
        }
        metrics
    }
}
