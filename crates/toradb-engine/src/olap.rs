use std::collections::{HashMap, HashSet};

use toradb_sql::ast::{AggFunc, CompareOp, SelectExpr, SelectStmt, WherePred};

use crate::dag::DagRunner;
use crate::metadata_filter::metadata_matches;
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

fn group_key(group_cols: &[String], id: u64, metadata: &HashMap<String, String>) -> String {
    if group_cols.is_empty() {
        return "_all".into();
    }
    group_cols
        .iter()
        .map(|col| {
            if col.eq_ignore_ascii_case("id") {
                id.to_string()
            } else {
                metadata.get(col).cloned().unwrap_or_else(|| "_null".into())
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn aggregate_specs(sel: &SelectStmt) -> Result<Vec<(AggFunc, Option<String>)>, String> {
    let mut aggs = Vec::new();
    for item in &sel.select_items {
        if let SelectExpr::Aggregate { func, column } = item {
            aggs.push((func.clone(), column.clone()));
        }
    }
    if aggs.is_empty() {
        return Err("analytics SELECT requires at least one aggregate expression".into());
    }
    Ok(aggs)
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
            .any(|(func, _)| !matches!(func, AggFunc::CountStar))
    {
        return Err("analytics SELECT without GROUP BY supports only COUNT(*)".into());
    }
    let group_cols = sel.group_by.clone();
    let value_columns = agg_specs
        .iter()
        .map(|(func, col)| value_column_name(func, col.as_deref()))
        .collect::<Vec<_>>();

    let filter_ids: Option<HashSet<u64>> = if sel.sparse_query.is_some()
        || sel.vector
        || sel.vector_query.is_some()
        || sel.vector_text.is_some()
    {
        let mut retrieval = sel.clone();
        retrieval.select_items = vec![SelectExpr::Column("id".into())];
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
            .map(|(func, _)| GroupAccum::new(func))
            .collect::<Vec<_>>();
        for (idx, (func, _)) in agg_specs.iter().enumerate() {
            match (&mut accs[idx], func) {
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
                if !metadata_matches(pred, metadata, &col_types) {
                    return Ok(());
                }
            }
            let key = group_key(&group_cols, id, metadata);
            let entry = groups.entry(key).or_insert_with(|| {
                agg_specs
                    .iter()
                    .map(|(func, _)| GroupAccum::new(func))
                    .collect::<Vec<_>>()
            });
            for (idx, (func, col)) in agg_specs.iter().enumerate() {
                if matches!(func, AggFunc::CountStar) {
                    entry[idx].update(func, None, None);
                    continue;
                }
                let numeric = col
                    .as_deref()
                    .and_then(|c| metadata.get(c))
                    .and_then(|v| parse_numeric_metadata(v));
                entry[idx].update(func, col.as_deref(), numeric);
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
                .map(|(acc, (func, _))| acc.finish(func))
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
