use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_engine::{materialized, persist, sql_exec, DagRunner};
use toradb_sql::{ast::Stmt, binder::Binder, parse};

use crate::table::{AnalyticsResults, SearchResults};

enum SqlOutcome {
    Message(String),
    Search(SearchResults),
    Aggregate(AnalyticsResults),
}

#[pyclass]
pub struct Database {
    path: String,
    dag: DagRunner,
    binder: Binder,
}

impl Database {
    pub fn open(path: String) -> PyResult<Self> {
        let dag = DagRunner::open(&path)
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
        Ok(Self { path, dag, binder: Binder::new() })
    }

    fn execute_sql(&mut self, query: &str) -> PyResult<SqlOutcome> {
        let stmts = parse(query).map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        if stmts.is_empty() {
            return Ok(SqlOutcome::Message("ok:0 stmts".into()));
        }
        if stmts.len() == 1 {
            match &stmts[0] {
                Stmt::ShowMaterializedViews => {
                    let base = self
                        .dag
                        .db_path()
                        .ok_or_else(|| {
                            pyo3::exceptions::PyValueError::new_err(
                                "materialized views require a local on-disk database",
                            )
                        })?
                        .to_path_buf();
                    let names = materialized::list_materialized_views(base.as_path())
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    let mut view_names = Vec::new();
                    let mut row_counts = Vec::new();
                    for name in names {
                        let n = materialized::load_view_row_count(base.as_path(), &name)
                            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                        view_names.push(name);
                        row_counts.push(n as f64);
                    }
                    return Ok(SqlOutcome::Aggregate(AnalyticsResults::new(
                        "view".into(),
                        view_names,
                        "rows".into(),
                        row_counts,
                    )));
                }
                Stmt::ShowTables => {
                    let names = self
                        .dag
                        .list_tables()
                        .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
                    let mut table_names = Vec::new();
                    let mut row_counts = Vec::new();
                    for name in names {
                        let n = self
                            .dag
                            .table_documents(&name)
                            .map(|d| d.len())
                            .unwrap_or(0);
                        table_names.push(name);
                        row_counts.push(n as f64);
                    }
                    return Ok(SqlOutcome::Aggregate(AnalyticsResults::new(
                        "table".into(),
                        table_names,
                        "rows".into(),
                        row_counts,
                    )));
                }
                Stmt::CreateTable(t) => {
                    let table = t.name.to_lowercase();
                    self.ensure_table(&table);
                    return Ok(SqlOutcome::Message(format!("ok: created table {table}")));
                }
                Stmt::CreateMaterializedView(mv) => {
                    let base = self
                        .dag
                        .db_path()
                        .ok_or_else(|| {
                            pyo3::exceptions::PyValueError::new_err(
                                "materialized views require a local on-disk database",
                            )
                        })?
                        .to_path_buf();
                    let rows = materialized::create_materialized_view(
                        &mut self.dag,
                        base.as_path(),
                        &mv.name,
                        &mv.select,
                    )
                    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    return Ok(SqlOutcome::Message(format!(
                        "ok: created materialized view {} ({} rows)",
                        mv.name, rows
                    )));
                }
                Stmt::RefreshMaterializedView { name } => {
                    let base = self
                        .dag
                        .db_path()
                        .ok_or_else(|| {
                            pyo3::exceptions::PyValueError::new_err(
                                "materialized views require a local on-disk database",
                            )
                        })?
                        .to_path_buf();
                    let rows = materialized::refresh_materialized_view(
                        &mut self.dag,
                        base.as_path(),
                        &name,
                    )
                    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    return Ok(SqlOutcome::Message(format!(
                        "ok: refreshed materialized view {} ({} rows)",
                        name, rows
                    )));
                }
                Stmt::AlterTableSetSegmentWorkers { table, workers } => {
                    self.dag
                        .set_segment_workers(&table.to_lowercase(), *workers)
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    return Ok(SqlOutcome::Message(format!(
                        "ok: set segment_workers={workers} on {table}"
                    )));
                }
                Stmt::CompactTable { table, full } => {
                    let report = self
                        .dag
                        .compact_table(&table.to_lowercase(), *full)
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    return Ok(SqlOutcome::Message(format!(
                        "ok: compacted {table} merges={} segments {} -> {}",
                        report.merges, report.segments_before, report.segments_after
                    )));
                }
                Stmt::DropMaterializedView { name } => {
                    let base = self
                        .dag
                        .db_path()
                        .ok_or_else(|| {
                            pyo3::exceptions::PyValueError::new_err(
                                "materialized views require a local on-disk database",
                            )
                        })?
                        .to_path_buf();
                    materialized::drop_materialized_view(base.as_path(), &name)
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    return Ok(SqlOutcome::Message(format!(
                        "ok: dropped materialized view {name}"
                    )));
                }
                Stmt::CreateIndex(idx) => {
                    let table = idx.table.to_lowercase();
                    self.dag
                        .create_index(&table, &idx.using)
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    self.binder
                        .bind(&stmts)
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    return Ok(SqlOutcome::Message(format!(
                        "ok: created index {} on {table} ({}) USING {}",
                        idx.name, idx.column, idx.using
                    )));
                }
                Stmt::DropTable { name } => {
                    let table = name.to_lowercase();
                    self.dag
                        .drop_table(&table)
                        .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
                    return Ok(SqlOutcome::Message(format!("ok: dropped table {table}")));
                }
                Stmt::Describe { name } => {
                    let table = name.to_lowercase();
                    if let Some(base) = self.dag.db_path() {
                        if materialized::is_materialized_view(base, &table) {
                            let rows = materialized::load_view_row_count(base, &table)
                                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                            return Ok(SqlOutcome::Message(format!(
                                "materialized_view: {table}\nrows: {rows}\nsource: cached search results"
                            )));
                        }
                    }
                    let row_count = self
                        .dag
                        .table_documents(&table)
                        .map(|d| d.len())
                        .unwrap_or(0);
                    let vector_dim = self
                        .dag
                        .vector_dim(&table)
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "none".to_string());
                    let segments = self
                        .dag
                        .db_path()
                        .and_then(|p| persist::table_segment_count(p, &table).ok())
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "n/a".to_string());
                    let segment_workers = self
                        .dag
                        .db_path()
                        .and_then(|p| persist::table_segment_workers(p, &table).ok())
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "n/a".to_string());
                    let indexes = self
                        .dag
                        .table_index_sidecars(&table)
                        .unwrap_or_default()
                        .join(", ");
                    let indexes_line = if indexes.is_empty() {
                        "none".to_string()
                    } else {
                        indexes
                    };
                    return Ok(SqlOutcome::Message(format!(
                        "table: {table}\nrows: {row_count}\nvector_dim: {vector_dim}\nsegments: {segments}\nsegment_workers: {segment_workers}\nindexes: {indexes_line}"
                    )));
                }
                Stmt::Select(sel) => {
                    if sel.explain && sel.stream {
                        return Err(pyo3::exceptions::PyValueError::new_err(
                            "EXPLAIN does not support STREAM",
                        ));
                    }
                    let out = sql_exec::run_select(&mut self.dag, sel)
                        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    return match out {
                        sql_exec::SqlSelectResult::Search(s) => Ok(SqlOutcome::Search(
                            SearchResults::from_sql(s.ids, s.scores, s.metrics, s.explain_text),
                        )),
                        sql_exec::SqlSelectResult::Aggregate(a) => Ok(SqlOutcome::Aggregate(
                            AnalyticsResults::new(
                                a.group_by_column,
                                a.group_keys,
                                a.value_column,
                                a.values,
                            ),
                        )),
                    };
                }
            }
        }
        self.binder
            .bind(&stmts)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        Ok(SqlOutcome::Message(format!("ok:{} stmts", stmts.len())))
    }

    pub(crate) fn run_retrieval(&mut self, batch: &mut Batch, ctx: &ExecCtx) -> QueryMetrics {
        self.dag.run(batch, ctx)
    }

    pub(crate) fn ensure_table(&mut self, name: &str) {
        self.dag.ensure_table(name);
    }

    pub(crate) fn add_documents(
        &mut self,
        table: &str,
        docs: Vec<toradb_index::IngestDoc>,
    ) -> PyResult<usize> {
        self.dag
            .add_documents(table, docs)
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))
    }

    pub(crate) fn vector_dim(&self, table: &str) -> Option<usize> {
        self.dag.vector_dim(table)
    }

    pub(crate) fn table_has_diskann_sidecar(&self, table: &str) -> bool {
        self.dag.table_has_diskann_sidecar(table)
    }

    fn list_table_names(&self) -> PyResult<Vec<String>> {
        self.dag
            .list_tables()
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))
    }
}

#[pymethods]
impl Database {
    #[new]
    fn py_new(path: String) -> PyResult<Self> {
        Self::open(path)
    }

    fn sql<'py>(&mut self, py: Python<'py>, query: &str) -> PyResult<Bound<'py, PyAny>> {
        match self.execute_sql(query)? {
            SqlOutcome::Message(s) => Ok(s.into_pyobject(py)?.into_any()),
            SqlOutcome::Search(results) => Ok(results.into_pyobject(py)?.into_any()),
            SqlOutcome::Aggregate(results) => Ok(results.into_pyobject(py)?.into_any()),
        }
    }

    /// Run a retrieval `SELECT` in pages (uses `LIMIT` / `OFFSET` under the hood).
    #[pyo3(signature = (query, batch_size=128))]
    fn sql_stream<'py>(
        &mut self,
        py: Python<'py>,
        query: &str,
        batch_size: usize,
    ) -> PyResult<Bound<'py, PyList>> {
        let stmts = parse(query).map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        let Stmt::Select(mut sel) = stmts
            .into_iter()
            .next()
            .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("expected a single SELECT"))?
        else {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "sql_stream requires a retrieval SELECT",
            ));
        };
        if sel.group_by.is_some() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "sql_stream does not support GROUP BY",
            ));
        }
        if sel.explain {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "sql_stream does not support EXPLAIN",
            ));
        }
        let page_size = batch_size.max(1) as u32;
        let mut offset = sel.offset;
        let list = PyList::empty(py);
        loop {
            sel.limit = page_size;
            sel.offset = offset;
            let out = sql_exec::run_select(&mut self.dag, &sel)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
            let sql_exec::SqlSelectResult::Search(page) = out else {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "sql_stream requires a retrieval SELECT",
                ));
            };
            let n = page.ids.len();
            if n == 0 {
                break;
            }
            let results =
                SearchResults::from_sql(page.ids, page.scores, page.metrics, None);
            list.append(results.into_pyobject(py)?)?;
            offset += n as u32;
            if n < page_size as usize {
                break;
            }
        }
        Ok(list)
    }

    #[pyo3(signature = (name, mode=None, schema=None))]
    fn create_table(
        mut slf: PyRefMut<'_, Self>,
        py: Python<'_>,
        name: &str,
        mode: Option<&str>,
        schema: Option<Bound<'_, PyDict>>,
    ) -> PyResult<super::table::Table> {
        let m = mode.unwrap_or("text");
        if let Some(s) = schema {
            let mut parts = vec![format!("CREATE TABLE {}", name.to_uppercase())];
            let mut cols = Vec::new();
            for (key, value) in s.iter() {
                cols.push(format!("{} {}", key.extract::<String>()?, value.extract::<String>()?));
            }
            if !cols.is_empty() {
                parts.push(format!("({})", cols.join(", ")));
            }
            parts.push(format!("USING {}", m.to_uppercase()));
            slf.execute_sql(&parts.join(" "))?;
        } else {
            slf.execute_sql(&format!(
                "CREATE TABLE {} USING {}",
                name.to_uppercase(),
                m.to_uppercase()
            ))?;
        }
        slf.ensure_table(name);
        let db = slf.into_pyobject(py)?.unbind();
        Ok(super::table::Table::new(name.to_string(), db))
    }

    /// Open an existing table (loaded on `Database.open`); does not run CREATE TABLE DDL.
    fn table(mut slf: PyRefMut<'_, Self>, py: Python<'_>, name: &str) -> PyResult<super::table::Table> {
        slf.ensure_table(name);
        let db = slf.into_pyobject(py)?.unbind();
        Ok(super::table::Table::new(name.to_string(), db))
    }

    fn list_tables(slf: PyRef<'_, Self>) -> PyResult<Vec<String>> {
        slf.list_table_names()
    }

    #[pyo3(signature = (table, using=None, column=None))]
    fn reindex(
        &mut self,
        table: &str,
        using: Option<&str>,
        column: Option<&str>,
    ) -> PyResult<String> {
        let using = using.unwrap_or("BM25");
        let column = column.unwrap_or("text");
        let sql = format!("CREATE INDEX sdk_reindex ON {table} ({column}) USING {using}");
        match self.execute_sql(&sql)? {
            SqlOutcome::Message(s) => Ok(s),
            _ => Ok(format!("ok: reindexed {table} USING {using}")),
        }
    }

    fn __repr__(&self) -> String {
        format!("Database({})", self.path)
    }
}
