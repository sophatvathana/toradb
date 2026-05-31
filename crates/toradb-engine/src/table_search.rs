use toradb_core::{Batch, ExecCtx, ProvenanceCollector, ProvenanceRecord, QueryMetrics};
use toradb_core::provenance::DropStage;
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

fn configure_batch(
    dag: &DagRunner,
    opts: &TableSearchOptions,
) -> (Batch, ExecCtx, Option<String>) {
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
    let ctx = tune_ctx(exec_ctx(opts.top_k, opts.offset), &opts.query, strategy);
    (batch, ctx, strategy.map(str::to_string))
}

fn format_explain(
    table: &str,
    strategy: Option<&str>,
    batch: &Batch,
    metrics: &QueryMetrics,
) -> String {
    let dense_backend = if batch.tier1_use_diskann {
        "diskann"
    } else if batch.tier1_enable_dense {
        "hnsw"
    } else {
        "none"
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
        metrics.segment_workers,
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
        Some(format_explain(&opts.table, strategy, &batch, &metrics))
    } else {
        None
    };

    let provenance = if opts.explain {
        let mut prov = ProvenanceCollector::new(
            true,
            opts.query.clone(),
            strategy_used.clone(),
        );

        // Tier 1: record all BM25 candidates (if sparse was enabled)
        if batch.tier1_enable_sparse {
            for (&id, &score) in batch.candidates.ids.iter().zip(batch.candidates.scores.iter()) {
                prov.record_bm25(id, score);
            }
        }
        // Tier 1: record all HNSW candidates (if dense was enabled)
        if batch.tier1_enable_dense {
            for (&id, &score) in batch.candidates.ids.iter().zip(batch.candidates.scores.iter()) {
                prov.record_hnsw(id, score);
            }
        }

        // Tier 2: record RRF-merged candidates
        for (&id, &score) in batch.candidates.ids.iter().zip(batch.candidates.scores.iter()) {
            prov.record_rrf(id, score);
        }

        // Record tier2 budget cuts: candidates after tier2 that didn't reach tier3
        let tier2_count = metrics.tier2_candidates as usize;
        let tier3_count = metrics.tier3_candidates as usize;
        if tier2_count > tier3_count {
            let cuts = tier2_count.saturating_sub(tier3_count);
            for i in tier3_count..(tier3_count + cuts).min(batch.candidates.len()) {
                let id = batch.candidates.ids[i];
                prov.record_drop(id, DropStage::Tier2BudgetCut, format!("tier2 budget cut at rank {i}"));
            }
        }

        // Record CRAG drops
        if batch.enable_crag {
            let final_count = top_k.min(batch.candidates.len());
            for i in final_count..batch.candidates.len() {
                let id = batch.candidates.ids[i];
                prov.record_drop(id, DropStage::CragFilter, "crag median filter".to_string());
            }
        }

        prov.set_final(&page.ids);
        prov.set_total_latency_ms(elapsed_ms);
        prov.finish()
    } else {
        None
    };

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
