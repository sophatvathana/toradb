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
    pub facets: Vec<String>,
    pub facet_top_n: Option<usize>,
    pub query_sparse: Option<std::collections::HashMap<String, f32>>,
    pub bm25_params: Option<(f32, f32)>,
    pub field_boosts: std::collections::HashMap<String, f32>,
    pub decay: Option<(String, f32)>,
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
    pub facets: Vec<crate::olap::FacetResult>,
}

fn exec_ctx(top_k: Option<u32>, offset: Option<u32>, widen: bool) -> ExecCtx {
    let k = top_k.unwrap_or(20);
    let off = offset.unwrap_or(0);
    let mut fetch = off.saturating_add(k).min(1000);
    if widen {
        fetch = fetch.saturating_mul(20).min(1000).max(fetch);
    }
    ExecCtx::new(
        fetch.saturating_mul(50).min(1000),
        fetch.saturating_mul(5).min(100),
        fetch,
    )
}

fn now_unix_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
    batch.query_sparse = opts.query_sparse.clone();
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
    batch.bm25_params = opts.bm25_params;
    batch.field_boosts = opts.field_boosts.clone();
    batch.decay = opts
        .decay
        .clone()
        .map(|(field, half_life_days)| toradb_core::DecaySpec {
            field,
            half_life_days,
        });
    let strategy_used = strategy.map(str::to_string);
    if opts.explain {
        batch.provenance = Some(ProvenanceCollector::new(
            true,
            opts.query.clone(),
            strategy_used.clone(),
        ));
    }
    let knobs = crate::rerank::knobs_active(&batch.field_boosts, &batch.decay);
    let widen = !opts.facets.is_empty() || knobs;
    let ctx = tune_ctx(
        exec_ctx(opts.top_k, opts.offset, widen),
        &opts.query,
        strategy,
    );
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
    let sparse_backend = match batch.sparse_backend.as_str() {
        b @ ("splade" | "seismic") => {
            if batch.query_sparse.as_ref().is_some_and(|m| !m.is_empty()) {
                b.to_string()
            } else {
                format!("{b}(fallback=bm25)")
            }
        }
        other => other.to_string(),
    };
    format!(
        "table={table} strategy={strategy:?} dense_backend={dense_backend} sparse_backend={sparse_backend} sparse={} dense={} graph_expand={} depth={} hyde={} crag={} distributed={} segment_workers={} segments_scanned={} tier1={} tier2={} tier3={}",
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

    if crate::rerank::knobs_active(&batch.field_boosts, &batch.decay) {
        let now = now_unix_millis();
        let mut candidates = std::mem::take(&mut batch.candidates);
        let mut prov = batch.provenance.take();
        crate::rerank::apply_ranking_knobs(
            dag,
            &opts.table,
            &mut candidates,
            &batch.field_boosts,
            &batch.decay,
            now,
            prov.as_mut(),
        )?;
        batch.candidates = candidates;
        batch.provenance = prov;
    }
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

    let facets = if opts.facets.is_empty() {
        Vec::new()
    } else {
        let candidate_ids: std::collections::HashSet<u64> =
            batch.candidates.ids.iter().copied().collect();
        let top_n = opts.facet_top_n.unwrap_or(crate::olap::DEFAULT_FACET_TOP_N);
        crate::olap::count_facets(dag, &opts.table, &opts.facets, &candidate_ids, top_n)?
    };

    let provenance = batch.provenance.take().and_then(|mut prov| {
        prov.set_final(&page.ids);
        prov.set_total_latency_ms(elapsed_ms);
        prov.set_facets(facets.iter().map(|f| {
            (
                f.field.clone(),
                f.values
                    .iter()
                    .map(|v| (v.value.clone(), v.count))
                    .collect::<Vec<_>>(),
            )
        }));
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
        facets,
    })
}
