use toradb_core::{Batch, ExecCtx, ProvenanceCollector, ProvenanceRecord, QueryMetrics};
use toradb_index::dense::query_embed::lexical_proxy_vector;
use toradb_storage::columnar::IndexMode;

use crate::adaptive::tune_ctx;
use crate::persist;
use crate::DagRunner;

#[derive(Debug, Clone)]
pub struct TableSearchOptions {
    pub table: String,
    pub query: String,
    pub top_k: Option<u32>,
    pub offset: Option<u32>,
    pub strategy: Option<String>,
    pub explain: bool,
    pub graph_expand: Option<bool>,
    pub depth: Option<u32>,
    pub query_vector: Option<Vec<f32>>,
}

#[derive(Debug, Clone)]
pub struct TableSearchResult {
    pub ids: Vec<u64>,
    pub scores: Vec<f32>,
    pub metrics: QueryMetrics,
    pub explain_text: Option<String>,
    /// Strategy actually applied (after segment-only / auto defaults).
    pub strategy_used: Option<String>,
    /// Structured provenance DAG (populated when `explain=true`).
    pub provenance: Option<ProvenanceRecord>,
}

fn exec_ctx(top_k: Option<u32>, offset: Option<u32>) -> ExecCtx {
    let k = top_k.unwrap_or(20);
    let off = offset.unwrap_or(0);
    let fetch = off.saturating_add(k).min(1000);
    ExecCtx::new(
        fetch.saturating_mul(50).min(1000),
        fetch.saturating_mul(5).min(100),
        fetch,
    )
}

fn is_segment_only_table(dag: &DagRunner, table: &str) -> bool {
    dag.db_path()
        .and_then(|p| persist::table_index_mode(p, table).ok())
        .map(|m| m == IndexMode::SegmentOnly)
        .unwrap_or(false)
}

fn configure_batch(dag: &DagRunner, opts: &TableSearchOptions) -> (Batch, ExecCtx, Option<String>) {
    // Match demo API: large segment-sharded tables default to BM25-only unless caller picks a strategy.
    let mut strategy = opts.strategy.clone();
    if strategy.is_none() && is_segment_only_table(dag, &opts.table) {
        strategy = Some("sparse".into());
    }
    let strategy = strategy.as_deref();

    let mut batch = Batch::new();
    batch.table = opts.table.clone();
    batch.query = opts.query.clone();
    batch.query_vector = opts.query_vector.clone();
    batch.tier1_enable_sparse = !matches!(
        strategy,
        Some("dense") | Some("vector") | Some("hnsw") | Some("diskann") | Some("ann")
    );
    batch.tier1_enable_dense = !matches!(strategy, Some("sparse") | Some("bm25") | Some("text"));
    batch.tier1_use_diskann = matches!(strategy, Some("diskann"));
    batch.tier1_use_ivf = matches!(strategy, Some("ivf"));
    batch.sparse_backend = match strategy {
        Some("splade") => "splade".into(),
        Some("seismic") => "seismic".into(),
        _ => "bm25".into(),
    };
    if !batch.tier1_use_diskann
        && batch.tier1_enable_dense
        && !batch.tier1_enable_sparse
        && dag.table_has_diskann_sidecar(&opts.table)
    {
        batch.tier1_use_diskann = true;
    }
    if batch.tier1_enable_dense && batch.query_vector.is_none() {
        if let Some(dim) = dag.vector_dim(&opts.table) {
            batch.query_vector = Some(lexical_proxy_vector(&opts.query, dim));
        } else {
            // No embeddings on disk — skip dense tier (avoids useless HNSW work on text-only tables).
            batch.tier1_enable_dense = false;
        }
    }
    batch.enable_hyde = matches!(strategy, Some("hyde"));
    batch.enable_crag = matches!(strategy, Some("crag"));
    batch.graph_expand =
        opts.graph_expand.unwrap_or(false) || matches!(strategy, Some("graph") | Some("hybrid"));
    batch.graph_depth = opts.depth.unwrap_or(2);
    batch.distributed_segments = matches!(strategy, Some("distributed"));
    let strategy_used = strategy.map(str::to_string);
    if opts.explain {
        batch.provenance = Some(ProvenanceCollector::new(
            true,
            opts.query.clone(),
            strategy_used.clone(),
        ));
    }
    let ctx = tune_ctx(exec_ctx(opts.top_k, opts.offset), &opts.query, strategy);
    (batch, ctx, strategy_used)
}

fn format_explain(
    table: &str,
    strategy: Option<&str>,
    batch: &Batch,
    metrics: &QueryMetrics,
    configured_workers: u32,
) -> String {
    let dense_backend = if batch.tier1_use_diskann {
        "diskann"
    } else if batch.tier1_enable_dense {
        "hnsw"
    } else {
        "none"
    };
    let segment_workers = if batch.distributed_segments && metrics.segment_workers == 0 {
        configured_workers
    } else {
        metrics.segment_workers
    };
    format!(
        "table={table} strategy={strategy:?} dense_backend={dense_backend} sparse={} dense={} graph_expand={} depth={} hyde={} crag={} distributed={} segment_workers={} segments_scanned={} tier1={} tier2={} tier3={}",
        batch.tier1_enable_sparse,
        batch.tier1_enable_dense,
        batch.graph_expand,
        batch.graph_depth,
        batch.enable_hyde,
        batch.enable_crag,
        batch.distributed_segments,
        segment_workers,
        metrics.segments_scanned,
        metrics.tier1_candidates,
        metrics.tier2_candidates,
        metrics.tier3_candidates
    )
}

/// Run native retrieval for a table (same path as `toradb.Table.search`).
pub fn run_table_search(
    dag: &mut DagRunner,
    opts: TableSearchOptions,
) -> Result<TableSearchResult, String> {
    let (mut batch, ctx, strategy_used) = configure_batch(dag, &opts);
    let strategy = strategy_used.as_deref();
    if !batch.table.is_empty() {
        dag.ensure_table_queryable(&batch.table)?;
    }
    let t0 = std::time::Instant::now();
    let metrics = dag.run(&mut batch, &ctx);
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let top_k = opts.top_k.unwrap_or(20) as usize;
    let offset = opts.offset.unwrap_or(0) as usize;
    let page = batch.candidates.slice_range(offset, top_k);

    let explain_text = if opts.explain {
        let configured_workers = dag
            .db_path()
            .and_then(|p| persist::table_segment_workers(p, &opts.table).ok())
            .unwrap_or(1);
        Some(format_explain(
            &opts.table,
            strategy,
            &batch,
            &metrics,
            configured_workers,
        ))
    } else {
        None
    };

    let provenance = batch.provenance.take().and_then(|mut prov| {
        prov.set_final(&page.ids);
        prov.set_total_latency_ms(elapsed_ms);
        prov.finish()
    });

    // Persist provenance to per-table search log when available.
    if let (Some(ref prov), Some(db_path)) = (&provenance, dag.db_path()) {
        persist::append_search_log(db_path, &opts.table, prov);
    }

    Ok(TableSearchResult {
        ids: page.ids,
        scores: page.scores,
        metrics,
        explain_text,
        strategy_used,
        provenance,
    })
}
