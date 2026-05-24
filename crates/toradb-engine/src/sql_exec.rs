use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_index::dense::query_embed::lexical_proxy_vector;
use toradb_sql::ast::SelectStmt;

use crate::dag::DagRunner;
use crate::join::apply_metadata_join;
use crate::materialized;
use crate::olap::{run_aggregate, SqlAggregateResult};
use crate::persist;

pub struct SqlSearchResult {
    pub ids: Vec<u64>,
    pub scores: Vec<f32>,
    pub metrics: QueryMetrics,
    pub explain_text: Option<String>,
}

pub enum SqlSelectResult {
    Search(SqlSearchResult),
    Aggregate(SqlAggregateResult),
}

pub fn run_select(dag: &mut DagRunner, sel: &SelectStmt) -> Result<SqlSelectResult, String> {
    if sel.explain {
        if sel.stream {
            return Err("EXPLAIN does not support STREAM".into());
        }
        let text = explain_plan(dag, sel)?;
        return Ok(SqlSelectResult::Search(SqlSearchResult {
            ids: Vec::new(),
            scores: Vec::new(),
            metrics: QueryMetrics::default(),
            explain_text: Some(text),
        }));
    }
    if let Some(base) = dag.db_path() {
        if materialized::is_materialized_view(base, &sel.table) {
            return Ok(SqlSelectResult::Search(materialized::query_materialized_view(
                base, &sel.table, sel,
            )?));
        }
    }
    if sel.group_by.is_some() {
        return Ok(SqlSelectResult::Aggregate(run_aggregate(dag, sel)?));
    }
    Ok(SqlSelectResult::Search(run_search(dag, sel)?))
}

pub fn explain_plan(dag: &DagRunner, sel: &SelectStmt) -> Result<String, String> {
    if let Some(base) = dag.db_path() {
        if materialized::is_materialized_view(base, &sel.table) {
            let rows = materialized::load_view_row_count(base, &sel.table)?;
            return Ok(format!(
                "MaterializedViewScan(view={} cached_rows={} limit={} offset={})",
                sel.table,
                rows,
                sel.limit.max(1),
                sel.offset
            ));
        }
    }
    if sel.group_by.is_some() {
        return Ok(format!(
            "AggregateScan(table={} group_by={:?} limit={} offset={})",
            sel.table,
            sel.group_by,
            sel.limit.max(1),
            sel.offset
        ));
    }

    let sparse = has_sparse(sel);
    let vector = has_vector(sel);
    if !sparse && !vector {
        return Err(
            "SELECT retrieval requires SPARSE SEARCH ... BM25('query') and/or VECTOR SEARCH ... ANN([...])"
                .into(),
        );
    }

    let dense_backend = if vector && !sparse && dag.table_has_diskann_sidecar(&sel.table) {
        "diskann"
    } else if vector {
        "hnsw"
    } else {
        "none"
    };
    let segments = dag
        .db_path()
        .and_then(|p| persist::table_segment_count(p, &sel.table).ok())
        .unwrap_or(0);
    let workers = dag
        .db_path()
        .and_then(|p| persist::table_segment_workers(p, &sel.table).ok())
        .unwrap_or(1);
    let indexes = dag
        .table_index_sidecars(&sel.table)
        .unwrap_or_default()
        .join(",");
    let segment_scan = dag.db_path().map(|base| {
        if sparse
            && persist::table_has_segment_bm25_sidecars(base, &sel.table).unwrap_or(false)
        {
            "bm25_shards"
        } else if vector
            && persist::table_has_segment_hnsw_sidecars(base, &sel.table).unwrap_or(false)
        {
            "hnsw_shards"
        } else {
            "table_level"
        }
    }).unwrap_or("n/a");

    let join = sel
        .join
        .as_ref()
        .map(|j| format!(" join {} on {}={}", j.right_table, j.left_key, j.right_key))
        .unwrap_or_default();
    let order = match sel.order_by_score_desc {
        Some(true) => " order_by_score_desc",
        Some(false) => " order_by_score_asc",
        None => "",
    };

    Ok(format!(
        "RetrievalScan(table={table} sparse={sparse} vector={vector} dense_backend={dense_backend} distributed={distributed} segment_scan={segment_scan} segments={segments} segment_workers={workers} indexes=[{indexes}] limit={limit} offset={offset}{join}{order})",
        table = sel.table,
        sparse = sparse,
        vector = vector,
        dense_backend = dense_backend,
        distributed = sel.distributed,
        segment_scan = segment_scan,
        segments = segments,
        workers = workers,
        indexes = if indexes.is_empty() { "none" } else { &indexes },
        limit = sel.limit.max(1),
        offset = sel.offset,
        join = join,
        order = order,
    ))
}

fn has_sparse(sel: &SelectStmt) -> bool {
    sel.sparse_query.as_ref().is_some_and(|q| !q.is_empty())
}

fn has_vector(sel: &SelectStmt) -> bool {
    sel.vector || sel.vector_query.is_some() || sel.vector_text.is_some()
}

pub(crate) fn run_search(dag: &mut DagRunner, sel: &SelectStmt) -> Result<SqlSearchResult, String> {
    let sparse = has_sparse(sel);
    let vector = has_vector(sel);
    if !sparse && !vector {
        return Err(
            "SELECT retrieval requires SPARSE SEARCH ... BM25('query') and/or VECTOR SEARCH ... ANN([...])"
                .into(),
        );
    }

    dag.ensure_table(&sel.table);
    let mut batch = Batch::new();
    batch.table = sel.table.clone();
    batch.query = sel
        .sparse_query
        .clone()
        .or_else(|| sel.vector_text.clone())
        .unwrap_or_default();
    batch.tier1_enable_sparse = sparse;
    batch.tier1_enable_dense = vector;
    batch.distributed_segments = sel.distributed;
    if vector && !sparse && dag.table_has_diskann_sidecar(&sel.table) {
        batch.tier1_use_diskann = true;
    }

    if let Some(ref v) = sel.vector_query {
        batch.query_vector = Some(v.clone());
    } else if vector {
        let dim = dag
            .vector_dim(&sel.table)
            .ok_or("VECTOR SEARCH requires a table with embeddings")?;
        let text = sel
            .vector_text
            .as_deref()
            .or(sel.sparse_query.as_deref())
            .unwrap_or(batch.query.as_str());
        batch.query_vector = Some(lexical_proxy_vector(text, dim));
    }

    let limit = sel.limit.max(1);
    let offset = sel.offset;
    let base_page = offset.saturating_add(limit).max(1);
    let page_size = if sel.order_by_score_desc.is_some() {
        base_page.saturating_mul(20).min(1000).max(base_page)
    } else {
        base_page
    };
    let ctx = ExecCtx::new(
        page_size.saturating_mul(50).min(1000),
        page_size.saturating_mul(5).min(100),
        page_size,
    );
    let metrics = dag.run(&mut batch, &ctx);
    let mut candidates = batch.candidates;
    if let Some(ref join) = sel.join {
        apply_metadata_join(dag, &sel.table, join, &mut candidates)?;
    }
    if let Some(desc) = sel.order_by_score_desc {
        candidates.sort_by_score(desc);
    }
    let page = candidates.slice_range(offset as usize, limit as usize);

    Ok(SqlSearchResult {
        ids: page.ids,
        scores: page.scores,
        metrics,
        explain_text: None,
    })
}
