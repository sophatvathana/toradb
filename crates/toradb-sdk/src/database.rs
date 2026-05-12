use pyo3::prelude::*;
use pyo3::types::PyDict;
use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_engine::DagRunner;
use toradb_sql::{binder::Binder, parse};

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

    fn execute_sql(&mut self, query: &str) -> PyResult<usize> {
        let stmts = parse(query).map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        let n = stmts.len();
        self.binder.bind(&stmts).map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        Ok(n)
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

    fn sql(&mut self, query: &str) -> PyResult<String> {
        let n = self.execute_sql(query)?;
        Ok(format!("ok:{} stmts", n))
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
