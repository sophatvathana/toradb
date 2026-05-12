use std::collections::{HashMap, HashSet};

use toradb_sql::ast::SelectStmt;

use crate::dag::DagRunner;
use crate::sql_exec::run_sparse_search;

#[derive(Debug, Clone)]
pub struct SqlAggregateResult {
    pub group_by_column: String,
    pub group_keys: Vec<String>,
    pub counts: Vec<u64>,
}

pub fn run_aggregate(dag: &mut DagRunner, sel: &SelectStmt) -> Result<SqlAggregateResult, String> {
    let group_col = sel
        .group_by
        .clone()
        .ok_or("analytics SELECT requires GROUP BY column")?;

    let filter_ids: Option<HashSet<u64>> = if sel.sparse_query.is_some() {
        let hits = run_sparse_search(dag, sel)?;
        Some(hits.ids.into_iter().collect())
    } else {
        None
    };

    dag.ensure_table(&sel.table);
    let docs = dag.retrieval.store.all_documents(&sel.table);
    let mut counts: HashMap<String, u64> = HashMap::new();

    for (id, doc) in docs {
        if let Some(ref allowed) = filter_ids {
            if !allowed.contains(&id) {
                continue;
            }
        }
        let key = doc
            .metadata
            .get(&group_col)
            .cloned()
            .unwrap_or_else(|| "_null".into());
        *counts.entry(key).or_insert(0) += 1;
    }

    let mut pairs: Vec<(String, u64)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let limit = sel.limit as usize;
    if limit > 0 && pairs.len() > limit {
        pairs.truncate(limit);
    }

    let (group_keys, counts): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();

    Ok(SqlAggregateResult {
        group_by_column: group_col,
        group_keys,
        counts,
    })
}
