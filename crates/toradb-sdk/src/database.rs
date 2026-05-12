use pyo3::prelude::*;
use pyo3::types::PyDict;
use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_engine::{sql_exec, DagRunner};
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
            if let Stmt::Select(sel) = &stmts[0] {
                let out = sql_exec::run_select(&mut self.dag, sel)
                    .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                return match out {
                    sql_exec::SqlSelectResult::Search(s) => Ok(SqlOutcome::Search(SearchResults::from_sql(
                        s.ids,
                        s.scores,
                        s.metrics,
                    ))),
                    sql_exec::SqlSelectResult::Aggregate(a) => Ok(SqlOutcome::Aggregate(
                        AnalyticsResults::new(a.group_by_column, a.group_keys, a.counts),
                    )),
                };
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

    fn __repr__(&self) -> String {
        format!("Database({})", self.path)
    }
}
