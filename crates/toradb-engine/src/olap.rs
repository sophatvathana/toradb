use std::collections::{HashMap, HashSet};

use toradb_sql::ast::{AggFunc, CompareOp, SelectExpr, SelectStmt, WherePred};

use crate::dag::DagRunner;
use crate::sql_exec::run_search;

#[derive(Debug, Clone)]
pub struct SqlAggregateResult {
    pub group_by_column: String,
    pub group_keys: Vec<String>,
    pub value_column: String,
    pub values: Vec<f64>,
}

fn parse_numeric_metadata(value: &str) -> Option<f64> {
    value.trim().parse().ok()
}

fn metadata_matches(pred: &WherePred, metadata: &std::collections::HashMap<String, String>) -> bool {
    match pred {
        WherePred::Compare { column, op, value } => {
            let Some(v) = metadata.get(column) else {
                return false;
            };
            match op {
                CompareOp::Eq => v == value,
                CompareOp::Ne => v != value,
                CompareOp::Lt | CompareOp::Lte | CompareOp::Gt | CompareOp::Gte => {
                    let (Some(a), Some(b)) =
                        (parse_numeric_metadata(v), parse_numeric_metadata(value))
                    else {
                        return false;
                    };
                    match op {
                        CompareOp::Lt => a < b,
                        CompareOp::Lte => a <= b,
                        CompareOp::Gt => a > b,
                        CompareOp::Gte => a >= b,
                        CompareOp::Eq | CompareOp::Ne => false,
                    }
                }
            }
        }
        WherePred::In { column, values } => metadata
            .get(column)
            .map(|v| values.iter().any(|x| x == v))
            .unwrap_or(false),
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

fn primary_aggregate(sel: &SelectStmt) -> Result<(AggFunc, Option<String>), String> {
    let mut aggs = Vec::new();
    for item in &sel.select_items {
        if let SelectExpr::Aggregate { func, column } = item {
            aggs.push((func.clone(), column.clone()));
        }
    }
    if aggs.len() != 1 {
        return Err("analytics SELECT requires exactly one aggregate expression".into());
    }
    Ok(aggs.remove(0))
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
    let group_col = sel
        .group_by
        .clone()
        .ok_or("analytics SELECT requires GROUP BY column")?;

    let (agg_func, agg_col) = primary_aggregate(sel)?;
    let value_col = value_column_name(&agg_func, agg_col.as_deref());

    let filter_ids: Option<HashSet<u64>> = if sel.sparse_query.is_some()
        || sel.vector
        || sel.vector_query.is_some()
        || sel.vector_text.is_some()
    {
        let hits = run_search(dag, sel)?;
        let sparse = sel.sparse_query.as_ref().is_some_and(|q| !q.is_empty());
        let vector = sel.vector || sel.vector_query.is_some() || sel.vector_text.is_some();
        let ids: Vec<u64> = if vector && !sparse {
            hits.ids.into_iter().take(1).collect()
        } else {
            hits.ids
        };
        Some(ids.into_iter().collect())
    } else {
        None
    };

    dag.ensure_table(&sel.table);
    let docs = dag.table_documents(&sel.table)?;
    let mut groups: HashMap<String, GroupAccum> = HashMap::new();

    for (id, doc) in docs {
        if let Some(ref allowed) = filter_ids {
            if !allowed.contains(&id) {
                continue;
            }
        }
        if let Some(ref pred) = sel.where_clause {
            if !metadata_matches(pred, &doc.metadata) {
                continue;
            }
        }
        let key = doc
            .metadata
            .get(&group_col)
            .cloned()
            .unwrap_or_else(|| "_null".into());

        let numeric = agg_col
            .as_deref()
            .and_then(|c| doc.metadata.get(c))
            .and_then(|v| parse_numeric_metadata(v));

        let entry = groups
            .entry(key)
            .or_insert_with(|| GroupAccum::new(&agg_func));
        if matches!(agg_func, AggFunc::CountStar) {
            entry.update(&agg_func, None, None);
        } else {
            entry.update(&agg_func, agg_col.as_deref(), numeric);
        }
    }

    let mut pairs: Vec<(String, f64)> = groups
        .into_iter()
        .map(|(k, acc)| (k, acc.finish(&agg_func)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let limit = sel.limit as usize;
    if limit > 0 && pairs.len() > limit {
        pairs.truncate(limit);
    }

    let (group_keys, values): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();

    Ok(SqlAggregateResult {
        group_by_column: group_col,
        group_keys,
        value_column: value_col,
        values,
    })
}
