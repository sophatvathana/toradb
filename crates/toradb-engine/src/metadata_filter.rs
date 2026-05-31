//! Type-aware metadata predicate evaluation for SQL WHERE clauses.

use std::cmp::Ordering;
use std::collections::HashMap;

use toradb_core::{CandidateSet, ColumnType};
use toradb_sql::ast::{CompareOp, WherePred};

use crate::dag::DagRunner;

fn parse_numeric_metadata(value: &str) -> Option<f64> {
    value.trim().parse().ok()
}

fn untyped_cmp(a: &str, b: &str) -> Option<Ordering> {
    let (x, y) = (parse_numeric_metadata(a)?, parse_numeric_metadata(b)?);
    x.partial_cmp(&y)
}

pub fn ordered(
    col_types: &HashMap<String, ColumnType>,
    column: &str,
    stored: &str,
    literal: &str,
) -> Option<Ordering> {
    match col_types.get(column) {
        Some(ty) if *ty != ColumnType::Text => {
            toradb_core::typed_cmp(*ty, stored, literal).or_else(|| untyped_cmp(stored, literal))
        }
        _ => untyped_cmp(stored, literal),
    }
}

pub fn typed_eq(
    col_types: &HashMap<String, ColumnType>,
    column: &str,
    stored: &str,
    literal: &str,
) -> bool {
    match ordered(col_types, column, stored, literal) {
        Some(ord) => ord == Ordering::Equal,
        None => stored == literal,
    }
}

pub fn metadata_matches(
    pred: &WherePred,
    metadata: &HashMap<String, String>,
    col_types: &HashMap<String, ColumnType>,
) -> bool {
    match pred {
        WherePred::Compare { column, op, value } => {
            let Some(v) = metadata.get(column) else {
                return false;
            };
            match op {
                CompareOp::Eq => typed_eq(col_types, column, v, value),
                CompareOp::Ne => !typed_eq(col_types, column, v, value),
                CompareOp::Lt => matches!(ordered(col_types, column, v, value), Some(Ordering::Less)),
                CompareOp::Lte => matches!(
                    ordered(col_types, column, v, value),
                    Some(Ordering::Less | Ordering::Equal)
                ),
                CompareOp::Gt => {
                    matches!(ordered(col_types, column, v, value), Some(Ordering::Greater))
                }
                CompareOp::Gte => matches!(
                    ordered(col_types, column, v, value),
                    Some(Ordering::Greater | Ordering::Equal)
                ),
            }
        }
        WherePred::In { column, values } => {
            let Some(v) = metadata.get(column) else {
                return false;
            };
            values
                .iter()
                .any(|x| typed_eq(col_types, column, v, x))
        }
        WherePred::Between {
            column,
            low,
            high,
            negated,
        } => {
            let Some(v) = metadata.get(column) else {
                return false;
            };
            let in_range = matches!(
                ordered(col_types, column, v, low),
                Some(Ordering::Greater | Ordering::Equal)
            ) && matches!(
                ordered(col_types, column, v, high),
                Some(Ordering::Less | Ordering::Equal)
            );
            in_range ^ negated
        }
    }
}

/// Drop retrieval candidates that fail a metadata WHERE predicate.
pub fn filter_candidates_by_where(
    dag: &mut DagRunner,
    table: &str,
    pred: &WherePred,
    candidates: &mut CandidateSet,
) -> Result<(), String> {
    if candidates.is_empty() {
        return Ok(());
    }
    let col_types = dag.column_types_for(table);
    let ids = candidates.ids.clone();
    let docs: HashMap<u64, _> = dag
        .fetch_documents(table, &ids)?
        .into_iter()
        .collect();
    let mut kept_ids = Vec::with_capacity(candidates.len());
    let mut kept_scores = Vec::with_capacity(candidates.len());
    for (i, id) in candidates.ids.iter().enumerate() {
        let Some(doc) = docs.get(id) else {
            continue;
        };
        if metadata_matches(pred, &doc.metadata, &col_types) {
            kept_ids.push(*id);
            kept_scores.push(candidates.scores[i]);
        }
    }
    candidates.ids = kept_ids;
    candidates.scores = kept_scores;
    Ok(())
}
