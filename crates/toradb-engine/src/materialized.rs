use std::path::Path;

use serde::{Deserialize, Serialize};
use toradb_core::{CandidateSet, QueryMetrics};
use toradb_sql::{format_select, parse, ast::Stmt};

use crate::dag::DagRunner;
use crate::sql_exec::{run_search, SqlSearchResult};

const VIEWS_DIR: &str = "_views";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializedViewFile {
    pub query: String,
    pub ids: Vec<u64>,
    pub scores: Vec<f32>,
}

fn views_root(base: &Path) -> std::path::PathBuf {
    base.join(VIEWS_DIR)
}

fn view_path(base: &Path, name: &str) -> std::path::PathBuf {
    views_root(base).join(name.to_lowercase()).join("data.json")
}

pub fn is_materialized_view(base: &Path, name: &str) -> bool {
    view_path(base, name).exists()
}

pub fn list_materialized_views(base: &Path) -> Result<Vec<String>, String> {
    let root = views_root(base);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if view_path(base, &name).exists() {
            names.push(name);
        }
    }
    names.sort_unstable();
    Ok(names)
}

fn load_view(base: &Path, name: &str) -> Result<MaterializedViewFile, String> {
    let path = view_path(base, name);
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

fn save_view(base: &Path, name: &str, view: &MaterializedViewFile) -> Result<(), String> {
    let path = view_path(base, name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(view).map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

pub fn load_view_row_count(base: &Path, name: &str) -> Result<usize, String> {
    Ok(load_view(base, name)?.ids.len())
}

pub fn drop_materialized_view(base: &Path, name: &str) -> Result<(), String> {
    if !is_materialized_view(base, name) {
        return Err(format!("materialized view {name} does not exist"));
    }
    let dir = views_root(base).join(name.to_lowercase());
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
}

pub fn create_materialized_view(
    dag: &mut DagRunner,
    base: &Path,
    name: &str,
    select: &toradb_sql::ast::SelectStmt,
) -> Result<usize, String> {
    if select.group_by.is_some() {
        return Err("materialized views support retrieval SELECT only (no GROUP BY)".into());
    }
    let query = format_select(select);
    let result = run_search(dag, select)?;
    let rows = result.ids.len();
    save_view(
        base,
        name,
        &MaterializedViewFile {
            query,
            ids: result.ids,
            scores: result.scores,
        },
    )?;
    Ok(rows)
}

pub fn refresh_materialized_view(dag: &mut DagRunner, base: &Path, name: &str) -> Result<usize, String> {
    let stored = load_view(base, name)?;
    let stmts = parse(&stored.query)?;
    let Stmt::Select(sel) = stmts
        .into_iter()
        .next()
        .ok_or("materialized view query must be a single SELECT")?
    else {
        return Err("materialized view query must be SELECT".into());
    };
    create_materialized_view(dag, base, name, &sel)
}

pub fn query_materialized_view(
    base: &Path,
    view_name: &str,
    sel: &toradb_sql::ast::SelectStmt,
) -> Result<SqlSearchResult, String> {
    if sel.group_by.is_some() {
        return Err("SELECT from materialized view does not support GROUP BY".into());
    }
    if sel.sparse_query.is_some() || sel.vector {
        return Err(
            "SELECT from materialized view reads cached rows; omit SPARSE/VECTOR clauses".into(),
        );
    }
    let stored = load_view(base, view_name)?;
    let mut candidates = CandidateSet::with_capacity(stored.ids.len());
    for (id, score) in stored.ids.into_iter().zip(stored.scores) {
        candidates.push(id, score);
    }
    if let Some(desc) = sel.order_by_score_desc {
        candidates.sort_by_score(desc);
    }
    let page = candidates.slice_range(sel.offset as usize, sel.limit.max(1) as usize);
    Ok(SqlSearchResult {
        ids: page.ids,
        scores: page.scores,
        metrics: QueryMetrics::default(),
    })
}
