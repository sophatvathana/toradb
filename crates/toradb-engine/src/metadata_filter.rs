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

pub fn like_matches(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    let (mut ti, mut pi) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '_' || p[pi] == t[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < p.len() && p[pi] == '%' {
            star = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '%' {
        pi += 1;
    }
    pi == p.len()
}

pub fn metadata_matches(
    pred: &WherePred,
    metadata: &HashMap<String, String>,
    col_types: &HashMap<String, ColumnType>,
) -> bool {
    match pred {
        WherePred::And(parts) => parts
            .iter()
            .all(|p| metadata_matches(p, metadata, col_types)),
        WherePred::Or(parts) => parts
            .iter()
            .any(|p| metadata_matches(p, metadata, col_types)),
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
        WherePred::Like {
            column,
            pattern,
            negated,
        } => {
            let Some(v) = metadata.get(column) else {
                return false;
            };
            like_matches(v, pattern) ^ negated
        }
    }
}

#[cfg(test)]
mod tests {
    use super::like_matches;

    #[test]
    fn like_wildcards() {
        assert!(like_matches("nikola tesla", "%tesla%"));
        assert!(like_matches("tesla", "tesla"));
        assert!(like_matches("tesla", "t_sla"));
        assert!(like_matches("tesla", "te%"));
        assert!(like_matches("tesla", "%la"));
        assert!(!like_matches("tesla", "edison%"));
        assert!(!like_matches("tesla", "t_s")); // anchored, full match required
        assert!(like_matches("anything", "%"));
        assert!(like_matches("", "%"));
        assert!(!like_matches("abc", "")); // empty pattern only matches empty text
        assert!(like_matches("", ""));
        // case-sensitive
        assert!(!like_matches("Tesla", "tesla"));
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
    let mut col_types = dag.column_types_for(table);
    col_types.insert("id".to_string(), ColumnType::Int);
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
        let mut meta = doc.metadata.clone();
        meta.insert("id".to_string(), id.to_string());
        if metadata_matches(pred, &meta, &col_types) {
            kept_ids.push(*id);
            kept_scores.push(candidates.scores[i]);
        }
    }
    candidates.ids = kept_ids;
    candidates.scores = kept_scores;
    Ok(())
}
