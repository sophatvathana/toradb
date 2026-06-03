use std::collections::HashMap;

use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_index::dense::query_embed::lexical_proxy_vector;
use toradb_sql::ast::{CompareOp, Expr, SelectExpr, SelectStmt, WherePred};

use crate::dag::DagRunner;
use crate::join::apply_metadata_join;
use crate::materialized;
use crate::metadata_filter::filter_candidates_by_where;
use crate::olap::{run_aggregate, SqlAggregateResult};
use crate::persist;

pub(crate) fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn order_by_metadata_sort(
    candidates: &mut toradb_core::CandidateSet,
    ob: &toradb_sql::ast::OrderBy,
    docs: &HashMap<u64, toradb_index::IngestDoc>,
    col_types: &HashMap<String, toradb_core::ColumnType>,
    now_millis: i64,
) {
    let empty_meta: HashMap<String, String> = HashMap::new();
    let keys: Vec<(String, toradb_core::ColumnType)> = candidates
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| match &ob.key {
            Some(expr) => {
                let doc = docs.get(id);
                let row = crate::scalar::Row {
                    id: *id,
                    metadata: doc.map(|d| &d.metadata).unwrap_or(&empty_meta),
                    text: doc.map(|d| d.text.as_str()),
                    score: candidates.scores.get(i).copied(),
                };
                let v = crate::scalar::eval_expr(expr, &row, col_types, now_millis);
                (v.as_string().unwrap_or_default(), v.column_type())
            }
            None => {
                let v = docs
                    .get(id)
                    .and_then(|d| d.metadata.get(&ob.column))
                    .cloned()
                    .unwrap_or_default();
                let ty = col_types
                    .get(&ob.column)
                    .copied()
                    .unwrap_or(toradb_core::ColumnType::Text);
                (v, ty)
            }
        })
        .collect();

    let mut idx: Vec<usize> = (0..candidates.ids.len()).collect();
    idx.sort_by(|&a, &b| {
        let (va, ty) = (&keys[a].0, keys[a].1);
        let vb = &keys[b].0;
        let ord = toradb_core::typed_cmp(ty, va, vb).unwrap_or_else(|| va.cmp(vb));
        if ob.descending {
            ord.reverse()
        } else {
            ord
        }
    });
    candidates.reorder(&idx);
}

fn apply_distinct(
    candidates: &mut toradb_core::CandidateSet,
    sel: &SelectStmt,
    docs: Option<&HashMap<u64, toradb_index::IngestDoc>>,
    col_types: &HashMap<String, toradb_core::ColumnType>,
    now_millis: i64,
) {
    let empty_meta: HashMap<String, String> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut keep = Vec::with_capacity(candidates.ids.len());
    for (i, id) in candidates.ids.iter().enumerate() {
        let doc = docs.and_then(|m| m.get(id));
        let row = crate::scalar::Row {
            id: *id,
            metadata: doc.map(|d| &d.metadata).unwrap_or(&empty_meta),
            text: doc.map(|d| d.text.as_str()),
            score: candidates.scores.get(i).copied(),
        };
        let mut key: Vec<String> = Vec::new();
        if sel.select_items.is_empty() {
            key.push(id.to_string());
        }
        for item in &sel.select_items {
            match item {
                SelectExpr::All => {
                    key.push(id.to_string());
                    key.push(format!(
                        "{:.6}",
                        candidates.scores.get(i).copied().unwrap_or(0.0)
                    ));
                    key.push(doc.map(|d| d.text.clone()).unwrap_or_default());
                }
                SelectExpr::Column { name, .. } => key.push(match name.as_str() {
                    "id" => id.to_string(),
                    "score" => format!("{:.6}", candidates.scores.get(i).copied().unwrap_or(0.0)),
                    "text" => doc.map(|d| d.text.clone()).unwrap_or_default(),
                    meta => doc
                        .and_then(|d| d.metadata.get(meta))
                        .cloned()
                        .unwrap_or_default(),
                }),
                SelectExpr::Func { expr, .. } => key.push(
                    crate::scalar::eval_expr(expr, &row, col_types, now_millis)
                        .as_string()
                        .unwrap_or_default(),
                ),
                SelectExpr::Aggregate { .. } => {}
            }
        }
        if seen.insert(key) {
            keep.push(i);
        }
    }
    candidates.retain_indices(&keep);
}

fn ids_from_delete_pred(pred: &WherePred) -> Result<Vec<u64>, String> {
    fn parse_id(s: &str) -> Result<u64, String> {
        s.trim()
            .parse::<u64>()
            .map_err(|_| format!("DELETE id value must be an integer, got '{s}'"))
    }
    match pred {
        WherePred::Compare { column, op, value } if column == "id" => {
            if !matches!(op, CompareOp::Eq) {
                return Err("DELETE only supports `id = N` or `id IN (...)`".into());
            }
            Ok(vec![parse_id(value)?])
        }
        WherePred::In { column, values } if column == "id" => {
            values.iter().map(|v| parse_id(v)).collect()
        }
        WherePred::Or(parts) => {
            let mut ids = Vec::new();
            for p in parts {
                ids.extend(ids_from_delete_pred(p)?);
            }
            Ok(ids)
        }
        _ => Err("DELETE WHERE supports only `id = N` or `id IN (...)` in this version".into()),
    }
}

pub fn run_delete(
    dag: &mut DagRunner,
    table: &str,
    where_clause: Option<&WherePred>,
) -> Result<usize, String> {
    let Some(pred) = where_clause else {
        return Err("DELETE requires a WHERE clause (use DROP TABLE to remove all rows)".into());
    };
    let ids = ids_from_delete_pred(pred)?;
    dag.delete_by_ids(&table.to_lowercase(), &ids)
}

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
    expanded.group_by_exprs = sel.group_by_exprs.clone();
    expanded.where_clause = sel.where_clause.clone();
    expanded.having_clause = sel.having_clause.clone();
    expanded.facets = sel.facets.clone();
    expanded.bm25_k1 = sel.bm25_k1;
    expanded.bm25_b = sel.bm25_b;
    expanded.field_boosts = sel.field_boosts.clone();
    expanded.decay = sel.decay.clone();
    expanded.highlight = sel.highlight;
    expanded.snippet_len = sel.snippet_len;
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
    pub facets: Vec<crate::olap::FacetResult>,
}

fn collect_columns(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Column(c) => {
            if c != "id" && c != "score" && !out.iter().any(|x| x == c) {
                out.push(c.clone());
            }
        }
        Expr::Literal(_) => {}
        Expr::Func { args, .. } => {
            for a in args {
                collect_columns(a, out);
            }
        }
    }
}

fn retrieval_fetch_columns(sel: &SelectStmt) -> Vec<String> {
    let mut cols = Vec::new();
    for item in &sel.select_items {
        match item {
            SelectExpr::All => {
                if !cols.iter().any(|c| c == "text") {
                    cols.push("text".to_string());
                }
            }
            SelectExpr::Column { name, .. } if name != "id" && name != "score" => {
                if !cols.iter().any(|c| c == name) {
                    cols.push(name.clone());
                }
            }
            SelectExpr::Func { expr, .. } => collect_columns(expr, &mut cols),
            _ => {}
        }
    }
    cols
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
    let fetch_cols = retrieval_fetch_columns(sel);
    let docs_by_id: HashMap<u64, toradb_index::IngestDoc> =
        if needs_document_fetch(&fetch_cols) || sel.highlight {
            dag.fetch_documents(table, ids)?.into_iter().collect()
        } else {
            HashMap::new()
        };

    let has_func = sel
        .select_items
        .iter()
        .any(|i| matches!(i, SelectExpr::Func { .. }));
    let (col_types, now) = if has_func {
        (dag.column_types_for(table), now_millis())
    } else {
        (HashMap::new(), 0)
    };
    let empty_meta: HashMap<String, String> = HashMap::new();
    let str_col = |key: &str| {
        SqlProjectedColumn::Str(
            ids.iter()
                .map(|id| {
                    docs_by_id
                        .get(id)
                        .and_then(|d| d.metadata.get(key))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect(),
        )
    };
    let text_col = || {
        SqlProjectedColumn::Str(
            ids.iter()
                .map(|id| {
                    docs_by_id
                        .get(id)
                        .map(|d| d.text.clone())
                        .unwrap_or_default()
                })
                .collect(),
        )
    };

    let mut out: Vec<(String, SqlProjectedColumn)> = Vec::new();
    let push_unique =
        |out: &mut Vec<(String, SqlProjectedColumn)>, name: String, data: SqlProjectedColumn| {
            if !out.iter().any(|(c, _)| *c == name) {
                out.push((name, data));
            }
        };
    if sel.select_items.is_empty() {
        push_unique(&mut out, "id".into(), SqlProjectedColumn::U64(ids.to_vec()));
        push_unique(
            &mut out,
            "score".into(),
            SqlProjectedColumn::F32(scores.to_vec()),
        );
    }
    for item in &sel.select_items {
        match item {
            SelectExpr::All => {
                push_unique(&mut out, "id".into(), SqlProjectedColumn::U64(ids.to_vec()));
                push_unique(
                    &mut out,
                    "score".into(),
                    SqlProjectedColumn::F32(scores.to_vec()),
                );
                push_unique(&mut out, "text".into(), text_col());
            }
            SelectExpr::Column { name, .. } => {
                let data = match name.as_str() {
                    "id" => SqlProjectedColumn::U64(ids.to_vec()),
                    "score" => SqlProjectedColumn::F32(scores.to_vec()),
                    "text" => text_col(),
                    key => str_col(key),
                };
                let out_name = item.output_name().expect("column has output name");
                push_unique(&mut out, out_name, data);
            }
            SelectExpr::Func { expr, .. } => {
                let values: Vec<String> = ids
                    .iter()
                    .enumerate()
                    .map(|(idx, id)| {
                        let doc = docs_by_id.get(id);
                        let row = crate::scalar::Row {
                            id: *id,
                            metadata: doc.map(|d| &d.metadata).unwrap_or(&empty_meta),
                            text: doc.map(|d| d.text.as_str()),
                            score: scores.get(idx).copied(),
                        };
                        crate::scalar::eval_expr(expr, &row, &col_types, now)
                            .as_string()
                            .unwrap_or_default()
                    })
                    .collect();
                let out_name = item.output_name().expect("func has output name");
                push_unique(&mut out, out_name, SqlProjectedColumn::Str(values));
            }
            SelectExpr::Aggregate { .. } => {
                return Err("aggregates require GROUP BY (analytics path)".into());
            }
        }
    }
    if sel.highlight {
        let max_chars = sel.snippet_len.unwrap_or(160) as usize;
        let qtokens =
            crate::snippets::snippet_query_tokens(sel.sparse_query.as_deref().unwrap_or_default());
        let snippets: Vec<String> = ids
            .iter()
            .map(|id| {
                docs_by_id
                    .get(id)
                    .map(|d| {
                        crate::snippets::generate_snippet(
                            &d.text, &qtokens, max_chars, "<em>", "</em>",
                        )
                    })
                    .unwrap_or_default()
            })
            .collect();
        out.push(("snippet".to_string(), SqlProjectedColumn::Str(snippets)));
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
        let text = explain_plan(dag, &sel)?;
        return Ok(SqlSelectResult::Search(SqlSearchResult {
            ids: Vec::new(),
            scores: Vec::new(),
            projected: Vec::new(),
            metrics: QueryMetrics::default(),
            explain_text: Some(text),
            facets: Vec::new(),
        }));
    }
    if let Some(base) = dag.db_path().map(|p| p.to_path_buf()) {
        if materialized::is_materialized_view(&base, &sel.table) {
            return Ok(SqlSelectResult::Search(
                materialized::query_materialized_view(dag, &base, &sel.table, &sel)?,
            ));
        }
    }
    if is_analytics_select(&sel) {
        return Ok(SqlSelectResult::Aggregate(run_aggregate(dag, &sel)?));
    }
    if !has_sparse(&sel) && !has_vector(&sel) {
        return Ok(SqlSelectResult::Search(run_scan(dag, &sel)?));
    }
    Ok(SqlSelectResult::Search(run_search(dag, &sel)?))
}

pub(crate) fn run_scan(dag: &mut DagRunner, sel: &SelectStmt) -> Result<SqlSearchResult, String> {
    dag.ensure_table(&sel.table);

    let mut ids: Vec<u64> = match sel.where_clause.as_ref().and_then(direct_id_lookup) {
        Some(direct) => direct,
        None => {
            let mut acc = Vec::new();
            dag.scan_table_id_metadata(&sel.table, |id, _meta| {
                acc.push(id);
                Ok(())
            })?;
            acc
        }
    };
    ids.sort_unstable();

    let mut candidates = toradb_core::CandidateSet {
        ids,
        scores: Vec::new(),
    };
    candidates.scores = vec![0.0; candidates.ids.len()];

    let now = now_millis();
    if let Some(ref pred) = sel.where_clause {
        filter_candidates_by_where(dag, &sel.table, pred, &mut candidates, now)?;
    }
    if let Some(ref join) = sel.join {
        apply_metadata_join(dag, &sel.table, join, &mut candidates)?;
    }

    let order_by_metadata = sel.order_by.as_ref().filter(|ob| ob.column != "score");
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
    if let Some(ob) = order_by_metadata {
        let docs = shared_docs.as_ref().expect("metadata fetched for ORDER BY");
        let col_types = dag.column_types_for(&sel.table);
        order_by_metadata_sort(&mut candidates, ob, docs, &col_types, now);
    }

    if sel.distinct {
        let col_types = dag.column_types_for(&sel.table);
        apply_distinct(&mut candidates, sel, shared_docs.as_ref(), &col_types, now);
    }

    let limit = sel.limit.max(1) as usize;
    let offset = sel.offset as usize;
    let page = candidates.slice_range(offset, limit);
    let projected = project_retrieval_columns(dag, &sel.table, sel, &page.ids, &page.scores)?;
    Ok(SqlSearchResult {
        ids: page.ids,
        scores: page.scores,
        projected,
        metrics: QueryMetrics::default(),
        explain_text: None,
        facets: Vec::new(),
    })
}

fn direct_id_lookup(pred: &WherePred) -> Option<Vec<u64>> {
    match pred {
        WherePred::Compare {
            column,
            op: CompareOp::Eq,
            value,
        } if column == "id" => value.trim().parse::<u64>().ok().map(|id| vec![id]),
        WherePred::In { column, values } if column == "id" => values
            .iter()
            .map(|v| v.trim().parse::<u64>().ok())
            .collect::<Option<Vec<u64>>>(),
        WherePred::Or(parts) => {
            let mut out = Vec::new();
            for p in parts {
                out.extend(direct_id_lookup(p)?);
            }
            Some(out)
        }
        _ => None,
    }
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
        let by_id = sel
            .where_clause
            .as_ref()
            .map(|p| direct_id_lookup(p).is_some())
            .unwrap_or(false);
        let mode = if by_id { "id_lookup" } else { "full" };
        return Ok(format!(
            "TableScan(table={} mode={mode} where={} order_by={:?} distinct={} limit={} offset={})",
            sel.table,
            sel.where_clause.is_some(),
            sel.order_by,
            sel.distinct,
            sel.limit.max(1),
            sel.offset
        ));
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
    let segment_scan = dag
        .db_path()
        .map(|base| {
            if sparse && persist::table_has_segment_bm25_sidecars(base, &sel.table).unwrap_or(false)
            {
                "bm25_shards"
            } else if vector
                && persist::table_has_segment_hnsw_sidecars(base, &sel.table).unwrap_or(false)
            {
                "hnsw_shards"
            } else {
                "table_level"
            }
        })
        .unwrap_or("n/a");

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
    let facets = if sel.facets.is_empty() {
        String::new()
    } else {
        format!(" facets=[{}]", sel.facets.join(","))
    };

    Ok(format!(
        "RetrievalScan(table={table} sparse={sparse} vector={vector} dense_backend={dense_backend} distributed={distributed} hyde={hyde} crag={crag} graph_expand={graph_expand} fusion_k={fusion_k} segment_scan={segment_scan} segments={segments} segment_workers={workers} indexes=[{indexes}] limit={limit} offset={offset}{join}{order}{facets})",
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
        facets = facets,
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
    batch.bm25_params = match (sel.bm25_k1, sel.bm25_b) {
        (None, None) => None,
        (k1, b) => Some((k1.unwrap_or(1.2), b.unwrap_or(0.75))),
    };
    batch.field_boosts = sel.field_boosts.clone();
    batch.decay = sel
        .decay
        .clone()
        .map(|(field, half_life_days)| toradb_core::DecaySpec {
            field,
            half_life_days,
        });
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
    // Facets must count the full matched set, so widen the retrieval window the same way
    // ORDER BY / DISTINCT do — otherwise a small LIMIT would also shrink the facet counts.
    let needs_window = sel.order_by.is_some()
        || sel.distinct
        || !sel.facets.is_empty()
        || crate::rerank::knobs_active(&batch.field_boosts, &batch.decay);
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
    let now = now_millis();
    let mut candidates = batch.candidates;
    if let Some(ref pred) = sel.where_clause {
        filter_candidates_by_where(dag, &sel.table, pred, &mut candidates, now)?;
    }
    if let Some(ref join) = sel.join {
        apply_metadata_join(dag, &sel.table, join, &mut candidates)?;
    }
    let knobs = crate::rerank::knobs_active(&batch.field_boosts, &batch.decay);
    let order_by_metadata = sel.order_by.as_ref().filter(|ob| ob.column != "score");
    let want_docs = knobs || order_by_metadata.is_some() || sel.distinct || !sel.facets.is_empty();
    let shared_docs: Option<HashMap<u64, toradb_index::IngestDoc>> = if want_docs {
        Some(
            dag.fetch_documents(&sel.table, &candidates.ids)?
                .into_iter()
                .collect(),
        )
    } else {
        None
    };

    if knobs {
        let docs = shared_docs
            .as_ref()
            .expect("docs fetched when knobs active");
        crate::rerank::apply_ranking_knobs_with_docs(
            &mut candidates,
            docs,
            &batch.field_boosts,
            &batch.decay,
            now,
            None,
        );
    }

    match sel.order_by.as_ref() {
        Some(ob) if ob.column == "score" => candidates.sort_by_score(ob.descending),
        Some(ob) => {
            let docs = shared_docs.as_ref().expect("metadata fetched for ORDER BY");
            let col_types = dag.column_types_for(&sel.table);
            order_by_metadata_sort(&mut candidates, ob, docs, &col_types, now);
        }
        None => candidates.sort_by_score(true),
    }

    if sel.distinct {
        let col_types = dag.column_types_for(&sel.table);
        apply_distinct(&mut candidates, sel, shared_docs.as_ref(), &col_types, now);
    }

    let facets = match (&shared_docs, sel.facets.is_empty()) {
        (Some(docs), false) => crate::olap::count_facets_for_ids(
            &sel.facets,
            docs,
            &candidates.ids,
            crate::olap::DEFAULT_FACET_TOP_N,
        ),
        _ => Vec::new(),
    };

    let page = candidates.slice_range(offset as usize, limit as usize);
    let projected = project_retrieval_columns(dag, &sel.table, sel, &page.ids, &page.scores)?;

    Ok(SqlSearchResult {
        ids: page.ids,
        scores: page.scores,
        projected,
        metrics,
        explain_text: None,
        facets,
    })
}
