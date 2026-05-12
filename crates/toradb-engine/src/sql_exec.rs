use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_sql::ast::SelectStmt;

use crate::dag::DagRunner;

pub struct SqlSearchResult {
    pub ids: Vec<u64>,
    pub scores: Vec<f32>,
    pub metrics: QueryMetrics,
}

pub fn run_select(dag: &mut DagRunner, sel: &SelectStmt) -> Result<SqlSearchResult, String> {
    let query = sel
        .sparse_query
        .clone()
        .filter(|q| !q.is_empty())
        .ok_or("SELECT retrieval requires SPARSE SEARCH ... BM25('query')")?;

    dag.ensure_table(&sel.table);
    let mut batch = Batch::new();
    batch.table = sel.table.clone();
    batch.query = query;
    let _ = sel.vector;

    let k = sel.limit.max(1);
    let ctx = ExecCtx::new(k.saturating_mul(50).min(1000), k.saturating_mul(5).min(100), k);
    let metrics = dag.run(&mut batch, &ctx);
    batch.candidates.truncate(k as usize);

    Ok(SqlSearchResult {
        ids: batch.candidates.ids,
        scores: batch.candidates.scores,
        metrics,
    })
}
