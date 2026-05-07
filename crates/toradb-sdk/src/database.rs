use pyo3::prelude::*;
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
        Ok(Self {
            path,
            dag: DagRunner::new(),
            binder: Binder::new(),
        })
    }
}

#[pymethods]
impl Database {
    #[new]
    fn py_new(path: String) -> PyResult<Self> {
        Self::open(path)
    }

    fn sql(&mut self, query: &str) -> PyResult<String> {
        let stmts = parse(query).map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        self.binder
            .bind(&stmts)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        Ok(format!("ok:{} stmts", stmts.len()))
    }

    #[pyo3(signature = (name, mode=None))]
    fn create_table(&mut self, name: &str, mode: Option<&str>) -> PyResult<super::table::Table> {
        let m = mode.unwrap_or("text");
        let sql = format!("CREATE TABLE {} USING {}", name.to_uppercase(), m.to_uppercase());
        self.sql(&sql)?;
        Ok(super::table::Table { name: name.to_string() })
    }

    fn __repr__(&self) -> String {
        format!("Database({})", self.path)
    }
}
