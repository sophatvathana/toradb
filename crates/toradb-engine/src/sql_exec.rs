use std::collections::HashMap;

use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_index::dense::query_embed::lexical_proxy_vector;
use toradb_sql::ast::{SelectExpr, SelectStmt};

use crate::dag::DagRunner;
use crate::join::apply_metadata_join;
use crate::materialized;
use crate::metadata_filter::filter_candidates_by_where;
use crate::olap::{run_aggregate, SqlAggregateResult};
use crate::persist;
use crate::table_search::{run_table_search, TableSearchOptions};

fn expand_cte_select(sel: &SelectStmt) -> Result<SelectStmt, String> {
    expand_cte_select_depth(sel, 2)
}

fn expand_cte_select_depth(sel: &SelectStmt, max_depth: u32) -> Result<SelectStmt, String> {
    if sel.ctes.is_empty() {
        return Ok(sel.clone());
    }
    if max_depth == 0 {
        return Err("nested WITH depth exceeded".into());
    }
    let cte = sel
        .ctes
        .iter()
        .find(|cte| cte.name == sel.table)
        .ok_or("WITH currently supports selecting from a defined CTE table name only")?;
    let mut expanded = if cte.query.ctes.is_empty() {
        (*cte.query).clone()
    } else {
        expand_cte_select_depth(&cte.query, max_depth - 1)?
    };
    expanded.select_items = sel.select_items.clone();
    expanded.group_by = sel.group_by.clone();
    expanded.where_clause = sel.where_clause.clone();
    expanded.having_clause = sel.having_clause.clone();
    expanded.limit = sel.limit;
    expanded.offset = sel.offset;
    expanded.order_by = sel.order_by.clone();
    expanded.distinct |= sel.distinct;
    expanded.stream |= sel.stream;
    expanded.explain |= sel.explain;
    expanded.distributed |= sel.distributed;
    expanded.hyde |= sel.hyde;
    expanded.crag |= sel.crag;
    expanded.join = sel.join.clone();
    if sel.sparse_query.is_some() {
        expanded.sparse_query = sel.sparse_query.clone();
    }
    if sel.sparse.is_some() {
        expanded.sparse = sel.sparse.clone();
    }
    if sel.vector || sel.vector_query.is_some() || sel.vector_text.is_some() {
        expanded.vector = sel.vector;
        expanded.vector_query = sel.vector_query.clone();
        expanded.vector_text = sel.vector_text.clone();
    }
    expanded.ctes.clear();
    Ok(expanded)
}

#[derive(Clone, Debug)]
pub enum SqlProjectedColumn {
    U64(Vec<u64>),
    F32(Vec<f32>),
    Str(Vec<String>),
}

pub struct SqlSearchResult {
    pub ids: Vec<u64>,
    pub scores: Vec<f32>,
    /// Columns in SELECT order (`id`, `score`, `text`, or metadata keys).
    pub projected: Vec<(String, SqlProjectedColumn)>,
    pub metrics: QueryMetrics,
    pub explain_text: Option<String>,
}

fn resolve_retrieval_columns(sel: &SelectStmt) -> Result<Vec<String>, String> {
    if sel.select_items.is_empty() {
        return Ok(vec!["id".into(), "score".into()]);
    }
    let mut cols = Vec::new();
    for item in &sel.select_items {
        match item {
            SelectExpr::All => {
                for name in ["id", "score", "text"] {
                    if !cols.iter().any(|c| c == name) {
                        cols.push(name.into());
                    }
                }
            }
            SelectExpr::Column(name) => {
                if !cols.iter().any(|c| c == name) {
                    cols.push(name.clone());
                }
            }
            SelectExpr::Aggregate { .. } => {
                return Err(
                    "aggregates in SELECT run as analytics (use COUNT(*) for total rows or GROUP BY for grouped results)"
                        .into(),
                );
            }
        }
    }
    Ok(cols)
}

fn needs_document_fetch(columns: &[String]) -> bool {
    columns.iter().any(|c| c != "id" && c != "score")
}

pub fn project_retrieval_columns(
    dag: &mut DagRunner,
    table: &str,
    sel: &SelectStmt,
    ids: &[u64],
    scores: &[f32],
) -> Result<Vec<(String, SqlProjectedColumn)>, String> {
    let columns = resolve_retrieval_columns(sel)?;
    let docs_by_id: HashMap<u64, toradb_index::IngestDoc> = if needs_document_fetch(&columns) {
        dag.fetch_documents(table, ids)?
            .into_iter()
            .collect()
    } else {
        HashMap::new()
    };

    let mut out = Vec::with_capacity(columns.len());
    for col in columns {
        let data = match col.as_str() {
            "id" => SqlProjectedColumn::U64(ids.to_vec()),
            "score" => SqlProjectedColumn::F32(scores.to_vec()),
            "text" => SqlProjectedColumn::Str(
                ids.iter()
                    .map(|id| {
                        docs_by_id
                            .get(id)
                            .map(|d| d.text.clone())
                            .unwrap_or_default()
                    })
                    .collect(),
            ),
            meta_key => SqlProjectedColumn::Str(
                ids.iter()
                    .map(|id| {
                        docs_by_id
                            .get(id)
                            .and_then(|d| d.metadata.get(meta_key))
                            .cloned()
                            .unwrap_or_default()
                    })
                    .collect(),
            ),
        };
        out.push((col, data));
    }
    Ok(out)
}

pub enum SqlSelectResult {
    Search(SqlSearchResult),
    Aggregate(SqlAggregateResult),
}

pub fn run_select(dag: &mut DagRunner, sel: &SelectStmt) -> Result<SqlSelectResult, String> {
    let sel = expand_cte_select(sel)?;
    if sel.explain {
        if sel.stream {
            return Err("EXPLAIN does not support STREAM".into());
        }
        if has_sparse(&sel) || has_vector(&sel) {
            let query = sel
                .sparse_query
                .clone()
                .or_else(|| sel.vector_text.clone())
                .unwrap_or_default();
            let strategy = sel.sparse.clone().or_else(|| {
                if sel.vector && !has_sparse(&sel) {
                    Some("dense".into())
                } else if has_sparse(&sel) && sel.vector {
                    Some("hybrid".into())
                } else {
                    None
                }
            });
            let opts = TableSearchOptions {
                table: sel.table.clone(),
                query,
                top_k: Some(sel.limit.max(1)),
                offset: Some(sel.offset),
                strategy,
                explain: true,
                graph_expand: Some(sel.graph_expand),
                depth: Some(sel.graph_depth),
                query_vector: sel.vector_query.clone(),
            };
            let result = run_table_search(dag, opts)?;
            let provenance_text = result
                .provenance
                .as_ref()
                .and_then(|p| serde_json::to_string_pretty(p).ok())
                .or(result.explain_text);
            return Ok(SqlSelectResult::Search(SqlSearchResult {
                ids: result.ids,
                scores: result.scores,
                projected: Vec::new(),
                metrics: result.metrics,
                explain_text: provenance_text,
            }));
        }
        let text = explain_plan(dag, &sel)?;
        return Ok(SqlSelectResult::Search(SqlSearchResult {
            ids: Vec::new(),
            scores: Vec::new(),
            projected: Vec::new(),
            metrics: QueryMetrics::default(),
            explain_text: Some(text),
        }));
    }
    if let Some(base) = dag.db_path().map(|p| p.to_path_buf()) {
        if materialized::is_materialized_view(&base, &sel.table) {
            return Ok(SqlSelectResult::Search(materialized::query_materialized_view(
                dag, &base, &sel.table, &sel,
            )?));
        }
    }
    if is_analytics_select(&sel) {
        return Ok(SqlSelectResult::Aggregate(run_aggregate(dag, &sel)?));
    }
    Ok(SqlSelectResult::Search(run_search(dag, &sel)?))
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
    if is_analytics_select(sel) {
        if sel.group_by.is_empty()
            && sel.where_clause.is_none()
            && !has_sparse(sel)
            && !has_vector(sel)
        {
            let rows = dag.table_row_count(&sel.table)?;
            return Ok(format!("TrivialCountScan(table={} rows={rows})", sel.table));
        }
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
    let order = match sel.order_by {
        Some(ref ob) => format!(
            " order_by={}_{}",
            ob.column,
            if ob.descending { "desc" } else { "asc" }
        ),
        None => String::new(),
    };

    Ok(format!(
        "RetrievalScan(table={table} sparse={sparse} vector={vector} dense_backend={dense_backend} distributed={distributed} hyde={hyde} crag={crag} graph_expand={graph_expand} fusion_k={fusion_k} segment_scan={segment_scan} segments={segments} segment_workers={workers} indexes=[{indexes}] limit={limit} offset={offset}{join}{order})",
        table = sel.table,
        sparse = sparse,
        vector = vector,
        dense_backend = dense_backend,
        distributed = sel.distributed,
        hyde = sel.hyde,
        crag = sel.crag,
        graph_expand = sel.graph_expand,
        fusion_k = sel.fusion_k,
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

fn is_analytics_select(sel: &SelectStmt) -> bool {
    sel.select_items
        .iter()
        .any(|item| matches!(item, SelectExpr::Aggregate { .. }))
        || !sel.group_by.is_empty()
        || sel.having_clause.is_some()
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
    batch.enable_hyde = sel.hyde;
    batch.enable_crag = sel.crag;
    batch.graph_expand = sel.graph_expand;
    batch.graph_depth = if sel.graph_expand {
        sel.graph_depth.max(1)
    } else {
        0
    };
    batch.fusion_k = sel.fusion_k.max(1);
    batch.sparse_backend = sel
        .sparse
        .clone()
        .unwrap_or_else(|| "bm25".into())
        .to_lowercase();
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
    let needs_window = sel.order_by.is_some() || sel.distinct;
    let page_size = if needs_window {
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
    if let Some(ref pred) = sel.where_clause {
        filter_candidates_by_where(dag, &sel.table, pred, &mut candidates)?;
    }
    if let Some(ref join) = sel.join {
        apply_metadata_join(dag, &sel.table, join, &mut candidates)?;
    }

    let order_by_metadata = sel
        .order_by
        .as_ref()
        .filter(|ob| ob.column != "score");
    let shared_docs: Option<HashMap<u64, toradb_index::IngestDoc>> =
        if order_by_metadata.is_some() || sel.distinct {
            Some(
                dag.fetch_documents(&sel.table, &candidates.ids)?
                    .into_iter()
                    .collect(),
            )
        } else {
            None
        };

    match sel.order_by.as_ref() {
        Some(ob) if ob.column == "score" => candidates.sort_by_score(ob.descending),
        Some(ob) => {
            let docs = shared_docs.as_ref().expect("metadata fetched for ORDER BY");
            let col_types = dag.column_types_for(&sel.table);
            let ty = col_types
                .get(&ob.column)
                .copied()
                .unwrap_or(toradb_core::ColumnType::Text);
            let mut idx: Vec<usize> = (0..candidates.ids.len()).collect();
            idx.sort_by(|&a, &b| {
                let va = docs
                    .get(&candidates.ids[a])
                    .and_then(|d| d.metadata.get(&ob.column))
                    .map(String::as_str)
                    .unwrap_or("");
                let vb = docs
                    .get(&candidates.ids[b])
                    .and_then(|d| d.metadata.get(&ob.column))
                    .map(String::as_str)
                    .unwrap_or("");
                let ord = toradb_core::typed_cmp(ty, va, vb)
                    .unwrap_or_else(|| va.cmp(vb));
                if ob.descending { ord.reverse() } else { ord }
            });
            candidates.reorder(&idx);
        }
        None => {}
    }

    if sel.distinct {
        let docs = shared_docs.as_ref();
        let projected_cols = resolve_retrieval_columns(sel)?;
        let mut seen = std::collections::HashSet::new();
        let mut keep = Vec::with_capacity(candidates.ids.len());
        for (i, id) in candidates.ids.iter().enumerate() {
            let key: Vec<String> = projected_cols
                .iter()
                .map(|c| match c.as_str() {
                    "id" => id.to_string(),
                    "score" => format!("{:.6}", candidates.scores[i]),
                    "text" => docs
                        .and_then(|m| m.get(id))
                        .map(|d| d.text.clone())
                        .unwrap_or_default(),
                    meta => docs
                        .and_then(|m| m.get(id))
                        .and_then(|d| d.metadata.get(meta))
                        .cloned()
                        .unwrap_or_default(),
                })
                .collect();
            if seen.insert(key) {
                keep.push(i);
            }
        }
        candidates.retain_indices(&keep);
    }

    let page = candidates.slice_range(offset as usize, limit as usize);
    let projected =
        project_retrieval_columns(dag, &sel.table, sel, &page.ids, &page.scores)?;

    Ok(SqlSearchResult {
        ids: page.ids,
        scores: page.scores,
        projected,
        metrics,
        explain_text: None,
    })
}
