use std::collections::{HashMap, HashSet};

use toradb_sql::ast::{AggFunc, CompareOp, Expr, SelectExpr, SelectStmt, WherePred};

use toradb_index::IngestDoc;

use crate::dag::DagRunner;
use crate::metadata_filter::metadata_matches;
use crate::scalar::{eval_expr, Row};
use crate::sql_exec::run_search;

#[derive(Debug, Clone)]
pub struct SqlAggregateResult {
    pub group_by_columns: Vec<String>,
    pub group_keys: Vec<String>,
    pub value_columns: Vec<String>,
    pub value_rows: Vec<Vec<f64>>,
}

fn parse_numeric_metadata(value: &str) -> Option<f64> {
    value.trim().parse().ok()
}

fn having_matches(
    pred: &WherePred,
    group_key: &str,
    values: &[f64],
    value_col_lookup: &HashMap<String, usize>,
    group_cols: &[String],
) -> bool {
    match pred {
        WherePred::And(parts) => parts
            .iter()
            .all(|p| having_matches(p, group_key, values, value_col_lookup, group_cols)),
        WherePred::Or(parts) => parts
            .iter()
            .any(|p| having_matches(p, group_key, values, value_col_lookup, group_cols)),
        WherePred::ExprCompare { .. } => false,
        WherePred::Compare { column, op, value } => {
            if let Some(idx) = value_col_lookup.get(column) {
                let b = parse_numeric_metadata(value).unwrap_or(0.0);
                let a = values.get(*idx).copied().unwrap_or(0.0);
                match op {
                    CompareOp::Eq => (a - b).abs() < f64::EPSILON,
                    CompareOp::Ne => (a - b).abs() >= f64::EPSILON,
                    CompareOp::Lt => a < b,
                    CompareOp::Lte => a <= b,
                    CompareOp::Gt => a > b,
                    CompareOp::Gte => a >= b,
                }
            } else if !group_cols.is_empty() && group_cols[0] == *column {
                match op {
                    CompareOp::Eq => group_key == value,
                    CompareOp::Ne => group_key != value,
                    _ => false,
                }
            } else {
                false
            }
        }
        WherePred::In {
            column,
            values: allow,
        } => {
            if !group_cols.is_empty() && group_cols[0] == *column {
                allow.iter().any(|v| v == group_key)
            } else {
                false
            }
        }
        WherePred::Between {
            column,
            low,
            high,
            negated,
        } => {
            if let Some(idx) = value_col_lookup.get(column) {
                let a = values.get(*idx).copied().unwrap_or(0.0);
                let lo = parse_numeric_metadata(low).unwrap_or(f64::NEG_INFINITY);
                let hi = parse_numeric_metadata(high).unwrap_or(f64::INFINITY);
                (a >= lo && a <= hi) ^ negated
            } else {
                false
            }
        }
        WherePred::Like {
            column,
            pattern,
            negated,
        } => {
            // HAVING LIKE only makes sense against a string group key.
            if !group_cols.is_empty() && group_cols[0] == *column {
                crate::metadata_filter::like_matches(group_key, pattern) ^ negated
            } else {
                false
            }
        }
    }
}

fn value_column_name(func: &AggFunc, column: Option<&str>) -> String {
    match func {
        AggFunc::CountStar => "count".into(),
        AggFunc::Sum => format!("sum_{}", column.unwrap_or("value")),
        AggFunc::Avg => format!("avg_{}", column.unwrap_or("value")),
        AggFunc::Min => format!("min_{}", column.unwrap_or("value")),
        AggFunc::Max => format!("max_{}", column.unwrap_or("value")),
    }
}

pub(crate) fn metadata_field_value(
    field: &str,
    id: u64,
    metadata: &HashMap<String, String>,
) -> String {
    if field.eq_ignore_ascii_case("id") {
        id.to_string()
    } else {
        metadata
            .get(field)
            .cloned()
            .unwrap_or_else(|| "_null".into())
    }
}

fn group_key_eval(
    group_cols: &[String],
    group_exprs: &[Option<Expr>],
    row: &Row,
    col_types: &HashMap<String, toradb_core::ColumnType>,
    now_millis: i64,
) -> String {
    if group_cols.is_empty() {
        return "_all".into();
    }
    group_cols
        .iter()
        .enumerate()
        .map(|(i, col)| match group_exprs.get(i).and_then(|e| e.as_ref()) {
            Some(expr) => eval_expr(expr, row, col_types, now_millis)
                .as_string()
                .unwrap_or_else(|| "_null".into()),
            None => metadata_field_value(col, row.id, row.metadata),
        })
        .collect::<Vec<_>>()
        .join("|")
}

pub const DEFAULT_FACET_TOP_N: usize = 20;

#[derive(Debug, Clone)]
pub struct FacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct FacetResult {
    pub field: String,
    pub values: Vec<FacetValue>,
}

pub fn count_facets_for_ids(
    fields: &[String],
    docs: &HashMap<u64, IngestDoc>,
    ids: &[u64],
    top_n: usize,
) -> Vec<FacetResult> {
    if fields.is_empty() {
        return Vec::new();
    }
    const NULL: &str = "_null";
    let want_id = fields.iter().any(|f| f.eq_ignore_ascii_case("id"));
    let id_strs: Vec<String> = if want_id {
        ids.iter().map(|id| id.to_string()).collect()
    } else {
        Vec::new()
    };
    let mut counts: Vec<HashMap<&str, u64>> = vec![HashMap::new(); fields.len()];

    for (pos, &id) in ids.iter().enumerate() {
        let Some(doc) = docs.get(&id) else { continue };
        for (fi, field) in fields.iter().enumerate() {
            let key: &str = if field.eq_ignore_ascii_case("id") {
                id_strs[pos].as_str()
            } else {
                doc.metadata.get(field).map(String::as_str).unwrap_or(NULL)
            };
            *counts[fi].entry(key).or_insert(0) += 1;
        }
    }

    fields
        .iter()
        .zip(counts.into_iter())
        .map(|(field, map)| {
            let cmp = |a: &(u64, &str), b: &(u64, &str)| {
                b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1))
            };
            let mut entries: Vec<(u64, &str)> =
                map.into_iter().map(|(value, count)| (count, value)).collect();
            if top_n > 0 && top_n < entries.len() {
                entries.select_nth_unstable_by(top_n - 1, cmp);
                entries.truncate(top_n);
            }
            entries.sort_by(cmp);
            let values = entries
                .into_iter()
                .map(|(count, value)| FacetValue {
                    value: value.to_string(),
                    count,
                })
                .collect();
            FacetResult {
                field: field.clone(),
                values,
            }
        })
        .collect()
}

pub fn count_facets(
    dag: &mut DagRunner,
    table: &str,
    fields: &[String],
    candidates: &HashSet<u64>,
    top_n: usize,
) -> Result<Vec<FacetResult>, String> {
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    dag.ensure_table(table);
    let ids: Vec<u64> = candidates.iter().copied().collect();
    let docs: HashMap<u64, IngestDoc> = dag.fetch_documents(table, &ids)?.into_iter().collect();
    Ok(count_facets_for_ids(fields, &docs, &ids, top_n))
}

struct AggSpec {
    func: AggFunc,
    arg: Option<Expr>,
    alias: Option<String>,
}

fn aggregate_specs(sel: &SelectStmt) -> Result<Vec<AggSpec>, String> {
    let mut aggs = Vec::new();
    for item in &sel.select_items {
        if let SelectExpr::Aggregate { func, arg, alias } = item {
            aggs.push(AggSpec {
                func: func.clone(),
                arg: arg.clone(),
                alias: alias.clone(),
            });
        }
    }
    if aggs.is_empty() {
        return Err("analytics SELECT requires at least one aggregate expression".into());
    }
    Ok(aggs)
}

fn agg_value_name(spec: &AggSpec) -> String {
    spec.alias
        .clone()
        .unwrap_or_else(|| value_column_name(&spec.func, spec.arg.as_ref().map(|e| e.alias()).as_deref()))
}

enum GroupAccum {
    Count(u64),
    Sum(f64),
    Avg { sum: f64, n: u64 },
    Min(Option<f64>),
    Max(Option<f64>),
}

impl GroupAccum {
    fn new(func: &AggFunc) -> Self {
        match func {
            AggFunc::CountStar => GroupAccum::Count(0),
            AggFunc::Sum => GroupAccum::Sum(0.0),
            AggFunc::Avg => GroupAccum::Avg { sum: 0.0, n: 0 },
            AggFunc::Min => GroupAccum::Min(None),
            AggFunc::Max => GroupAccum::Max(None),
        }
    }

    fn update(&mut self, func: &AggFunc, col: Option<&str>, doc_value: Option<f64>) {
        match (self, func) {
            (GroupAccum::Count(n), AggFunc::CountStar) => *n += 1,
            (GroupAccum::Sum(s), AggFunc::Sum) => {
                if let Some(v) = doc_value {
                    *s += v;
                }
            }
            (GroupAccum::Avg { sum, n }, AggFunc::Avg) => {
                if let Some(v) = doc_value {
                    *sum += v;
                    *n += 1;
                }
            }
            (GroupAccum::Min(cur), AggFunc::Min) => {
                if let Some(v) = doc_value {
                    *cur = Some(cur.map(|m| m.min(v)).unwrap_or(v));
                }
            }
            (GroupAccum::Max(cur), AggFunc::Max) => {
                if let Some(v) = doc_value {
                    *cur = Some(cur.map(|m| m.max(v)).unwrap_or(v));
                }
            }
            _ => {}
        }
        let _ = col;
    }

    fn finish(self, func: &AggFunc) -> f64 {
        match (self, func) {
            (GroupAccum::Count(n), AggFunc::CountStar) => n as f64,
            (GroupAccum::Sum(s), AggFunc::Sum) => s,
            (GroupAccum::Avg { sum, n }, AggFunc::Avg) => {
                if n == 0 {
                    0.0
                } else {
                    sum / n as f64
                }
            }
            (GroupAccum::Min(v), AggFunc::Min) => v.unwrap_or(0.0),
            (GroupAccum::Max(v), AggFunc::Max) => v.unwrap_or(0.0),
            _ => 0.0,
        }
    }
}

pub fn run_aggregate(dag: &mut DagRunner, sel: &SelectStmt) -> Result<SqlAggregateResult, String> {
    let agg_specs = aggregate_specs(sel)?;
    if sel.group_by.is_empty()
        && agg_specs
            .iter()
            .any(|s| !matches!(s.func, AggFunc::CountStar))
    {
        return Err("analytics SELECT without GROUP BY supports only COUNT(*)".into());
    }
    let group_cols = sel.group_by.clone();
    let group_exprs = sel.group_by_exprs.clone();
    let value_columns = agg_specs.iter().map(agg_value_name).collect::<Vec<_>>();
    let now = crate::sql_exec::now_millis();

    let filter_ids: Option<HashSet<u64>> = if sel.sparse_query.is_some()
        || sel.vector
        || sel.vector_query.is_some()
        || sel.vector_text.is_some()
    {
        let mut retrieval = sel.clone();
        retrieval.select_items = vec![SelectExpr::Column {
            name: "id".into(),
            alias: None,
        }];
        retrieval.group_by.clear();
        retrieval.having_clause = None;
        let sparse = sel.sparse_query.as_ref().is_some_and(|q| !q.is_empty());
        let vector = sel.vector || sel.vector_query.is_some() || sel.vector_text.is_some();
        let ids: Vec<u64> = if sparse && vector {
            let mut lexical = retrieval.clone();
            lexical.vector = false;
            lexical.vector_query = None;
            lexical.vector_text = None;
            run_search(dag, &lexical)?.ids
        } else if vector && !sparse {
            run_search(dag, &retrieval)?.ids
        } else {
            run_search(dag, &retrieval)?.ids
        };
        Some(ids.into_iter().collect())
    } else {
        None
    };

    dag.ensure_table(&sel.table);
    let col_types = dag.column_types_for(&sel.table);
    let mut groups: HashMap<String, Vec<GroupAccum>> = HashMap::new();
    if group_cols.is_empty() && filter_ids.is_none() && sel.where_clause.is_none() {
        let rows = dag.table_row_count(&sel.table)? as u64;
        let mut accs = agg_specs
            .iter()
            .map(|s| GroupAccum::new(&s.func))
            .collect::<Vec<_>>();
        for (idx, spec) in agg_specs.iter().enumerate() {
            match (&mut accs[idx], &spec.func) {
                (GroupAccum::Count(n), AggFunc::CountStar) => *n = rows,
                (slot, func) => slot.update(func, None, None),
            }
        }
        groups.insert("_all".into(), accs);
    } else {
        dag.scan_table_id_metadata(&sel.table, |id, metadata| {
            if let Some(ref allowed) = filter_ids {
                if !allowed.contains(&id) {
                    return Ok(());
                }
            }
            if let Some(ref pred) = sel.where_clause {
                if !metadata_matches(pred, metadata, &col_types, now) {
                    return Ok(());
                }
            }
            let row = Row {
                id,
                metadata,
                text: None,
                score: None,
            };
            let key = group_key_eval(&group_cols, &group_exprs, &row, &col_types, now);
            let entry = groups.entry(key).or_insert_with(|| {
                agg_specs
                    .iter()
                    .map(|s| GroupAccum::new(&s.func))
                    .collect::<Vec<_>>()
            });
            for (idx, spec) in agg_specs.iter().enumerate() {
                if matches!(spec.func, AggFunc::CountStar) {
                    entry[idx].update(&spec.func, None, None);
                    continue;
                }
                let numeric = spec
                    .arg
                    .as_ref()
                    .and_then(|e| eval_expr(e, &row, &col_types, now).as_f64());
                entry[idx].update(&spec.func, None, numeric);
            }
            Ok(())
        })?;
    }

    let mut pairs: Vec<(String, Vec<f64>)> = groups
        .into_iter()
        .map(|(k, accs)| {
            let values = accs
                .into_iter()
                .zip(agg_specs.iter())
                .map(|(acc, spec)| acc.finish(&spec.func))
                .collect::<Vec<_>>();
            (k, values)
        })
        .collect();
    if let Some(ref pred) = sel.having_clause {
        let value_col_lookup = value_columns
            .iter()
            .enumerate()
            .map(|(idx, name)| (name.clone(), idx))
            .collect::<HashMap<_, _>>();
        pairs.retain(|(group_key, values)| {
            having_matches(pred, group_key, values, &value_col_lookup, &group_cols)
        });
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let offset = sel.offset as usize;
    if offset > 0 {
        pairs = pairs.into_iter().skip(offset).collect();
    }
    let limit = sel.limit as usize;
    if limit > 0 && pairs.len() > limit {
        pairs.truncate(limit);
    }

    let (group_keys, value_rows): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();

    Ok(SqlAggregateResult {
        group_by_columns: if group_cols.is_empty() {
            vec!["_all".into()]
        } else {
            group_cols
        },
        group_keys,
        value_columns,
        value_rows,
    })
}
