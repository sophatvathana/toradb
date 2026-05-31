use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use toradb_engine::TableSearchOptions;
use pyo3_arrow::PyTable;
use toradb_index::IngestDoc;

use crate::database::Database;

#[pyclass]
pub struct Table {
    pub name: String,
    db: Py<Database>,
}

impl Table {
    pub fn new(name: String, db: Py<Database>) -> Self {
        Self { name, db }
    }
}

fn parse_ingest_doc(item: &Bound<'_, PyAny>) -> PyResult<IngestDoc> {
    if let Ok(text) = item.extract::<String>() {
        return Ok(IngestDoc {
            text,
            metadata: HashMap::new(),
            vector: None,
        });
    }

    let dict = item
        .cast::<PyDict>()
        .map_err(|_| pyo3::exceptions::PyTypeError::new_err("document must be str or dict"))?;

    let text = match dict.get_item("text")? {
        Some(v) => v.extract::<String>()?,
        None => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "dict document requires a 'text' field",
            ));
        }
    };

    let mut metadata = HashMap::new();
    let mut vector = None;

    for (key, value) in dict.iter() {
        let k = key.extract::<String>()?;
        if k == "text" {
            continue;
        }
        if k == "vector" || k == "embedding" {
            if let Ok(v) = value.extract::<Vec<f32>>() {
                vector = Some(v);
            }
            continue;
        }
        if let Ok(s) = value.extract::<String>() {
            metadata.insert(k, s);
        } else if let Ok(n) = value.extract::<i64>() {
            metadata.insert(k, n.to_string());
        } else if let Ok(n) = value.extract::<f64>() {
            metadata.insert(k, n.to_string());
        }
    }

    Ok(IngestDoc {
        text,
        metadata,
        vector,
    })
}

#[pymethods]
impl Table {
    fn add(&self, py: Python<'_>, docs: &Bound<'_, PyList>) -> PyResult<usize> {
        let mut parsed = Vec::with_capacity(docs.len());
        for item in docs.iter() {
            parsed.push(parse_ingest_doc(&item)?);
        }
        let mut db = self.db.borrow_mut(py);
        db.add_documents(&self.name, parsed)
    }

    /// Ingest a PyArrow Table via the Arrow PyCapsule interface (zero-copy column read in Rust).
    fn add_arrow(&self, py: Python<'_>, table: PyTable) -> PyResult<usize> {
        let mut db = self.db.borrow_mut(py);
        if db.bulk_ingest_active(&self.name) {
            let mut n = 0usize;
            for batch in table.batches() {
                n += db
                    .ingest_record_batch(&self.name, batch)
                    .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
            }
            return Ok(n);
        }
        let parsed = crate::arrow_ingest::ingest_pytable(table)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        db.add_documents(&self.name, parsed)
    }

    #[pyo3(signature = (query, top_k=None, offset=None, strategy=None, explain=None, graph_expand=None, depth=None, query_vector=None))]
    fn search(
        &self,
        py: Python<'_>,
        query: &str,
        top_k: Option<u32>,
        offset: Option<u32>,
        strategy: Option<&str>,
        explain: Option<bool>,
        graph_expand: Option<bool>,
        depth: Option<u32>,
        query_vector: Option<Vec<f32>>,
    ) -> PyResult<SearchResults> {
        let opts = TableSearchOptions {
            table: self.name.clone(),
            query: query.to_string(),
            top_k,
            offset,
            strategy: strategy.map(str::to_string),
            explain: explain.unwrap_or(false),
            graph_expand,
            depth,
            query_vector,
        };
        let db_handle = self.db.clone_ref(py);
        let out = py.detach(move || {
            Python::attach(|inner_py| {
                db_handle
                    .bind(inner_py)
                    .borrow_mut()
                    .run_table_search(opts)
                    .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e))
            })
        })?;
        let provenance_json = out.provenance.as_ref().and_then(|p| {
            serde_json::to_string_pretty(p).ok()
        });
        Ok(SearchResults {
            ids: out.ids,
            scores: out.scores,
            projected: Vec::new(),
            metrics: out.metrics,
            explain_text: out.explain_text,
            provenance_json,
        })
    }

    fn __repr__(&self) -> String {
        format!("Table({})", self.name)
    }
}

#[derive(Clone)]
pub(crate) enum SearchColumn {
    U64(Vec<u64>),
    F32(Vec<f32>),
    Str(Vec<String>),
}

impl From<toradb_engine::sql_exec::SqlProjectedColumn> for SearchColumn {
    fn from(col: toradb_engine::sql_exec::SqlProjectedColumn) -> Self {
        match col {
            toradb_engine::sql_exec::SqlProjectedColumn::U64(v) => SearchColumn::U64(v),
            toradb_engine::sql_exec::SqlProjectedColumn::F32(v) => SearchColumn::F32(v),
            toradb_engine::sql_exec::SqlProjectedColumn::Str(v) => SearchColumn::Str(v),
        }
    }
}

#[pyclass]
pub struct SearchResults {
    ids: Vec<u64>,
    scores: Vec<f32>,
    projected: Vec<(String, SearchColumn)>,
    metrics: toradb_core::QueryMetrics,
    explain_text: Option<String>,
    provenance_json: Option<String>,
}

impl SearchResults {
    pub(crate) fn from_sql(
        ids: Vec<u64>,
        scores: Vec<f32>,
        projected: Vec<(String, toradb_engine::sql_exec::SqlProjectedColumn)>,
        metrics: toradb_core::QueryMetrics,
        explain_text: Option<String>,
    ) -> Self {
        Self {
            ids,
            scores,
            projected: projected
                .into_iter()
                .map(|(name, col)| (name, col.into()))
                .collect(),
            metrics,
            explain_text,
            provenance_json: None,
        }
    }
}

#[pymethods]
impl SearchResults {
    fn to_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = pyo3::types::PyDict::new(py);
        if self.projected.is_empty() {
            dict.set_item("id", self.ids.clone())?;
            dict.set_item("score", self.scores.clone())?;
        } else {
            for (name, col) in &self.projected {
                match col {
                    SearchColumn::U64(v) => dict.set_item(name, v.clone())?,
                    SearchColumn::F32(v) => dict.set_item(name, v.clone())?,
                    SearchColumn::Str(v) => dict.set_item(name, v.clone())?,
                }
            }
        }
        Ok(dict.into_any())
    }

    fn to_polars<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_pandas(py)
    }

    fn explain(&self) -> String {
        if let Some(ref text) = self.explain_text {
            return text.clone();
        }
        format!(
            "tier1={} tier2={} tier3={} decompressions={}",
            self.metrics.tier1_candidates,
            self.metrics.tier2_candidates,
            self.metrics.tier3_candidates,
            self.metrics.decompressions
        )
    }

    #[getter]
    fn provenance(&self) -> Option<String> {
        self.provenance_json.clone()
    }

    #[getter]
    fn provenance_dict<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyAny>>> {
        match &self.provenance_json {
            Some(s) => {
                let value: serde_json::Value = serde_json::from_str(s).map_err(|e| {
                    pyo3::exceptions::PyValueError::new_err(format!("invalid provenance JSON: {e}"))
                })?;
                Ok(Some(json_to_py(py, &value)?))
            }
            None => Ok(None),
        }
    }
}

pub(crate) fn json_to_py<'py>(
    py: Python<'py>,
    value: &serde_json::Value,
) -> PyResult<Bound<'py, PyAny>> {
    use serde_json::Value;
    Ok(match value {
        Value::Null => py.None().into_bound(py),
        Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_pyobject(py)?.into_any()
            } else if let Some(u) = n.as_u64() {
                u.into_pyobject(py)?.into_any()
            } else {
                n.as_f64().unwrap_or(0.0).into_pyobject(py)?.into_any()
            }
        }
        Value::String(s) => s.into_pyobject(py)?.into_any(),
        Value::Array(arr) => {
            let list = PyList::empty(py);
            for item in arr {
                list.append(json_to_py(py, item)?)?;
            }
            list.into_any()
        }
        Value::Object(map) => {
            let dict = PyDict::new(py);
            for (k, v) in map {
                dict.set_item(k, json_to_py(py, v)?)?;
            }
            dict.into_any()
        }
    })
}

#[pyclass]
pub struct AnalyticsResults {
    group_by_columns: Vec<String>,
    group_keys: Vec<String>,
    value_columns: Vec<String>,
    value_rows: Vec<Vec<f64>>,
}

impl AnalyticsResults {
    pub(crate) fn new(
        group_by_columns: Vec<String>,
        group_keys: Vec<String>,
        value_columns: Vec<String>,
        value_rows: Vec<Vec<f64>>,
    ) -> Self {
        Self {
            group_by_columns,
            group_keys,
            value_columns,
            value_rows,
        }
    }
}

#[pymethods]
impl AnalyticsResults {
    fn to_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = pyo3::types::PyDict::new(py);
        if self.group_by_columns.is_empty() {
            dict.set_item("_all", self.group_keys.clone())?;
        } else if self.group_by_columns.len() == 1 {
            dict.set_item(&self.group_by_columns[0], self.group_keys.clone())?;
        } else {
            let mut columns = vec![Vec::with_capacity(self.group_keys.len()); self.group_by_columns.len()];
            for key in &self.group_keys {
                let parts = key.split('|').collect::<Vec<_>>();
                for (idx, col_values) in columns.iter_mut().enumerate() {
                    col_values.push(parts.get(idx).copied().unwrap_or("_null").to_string());
                }
            }
            for (name, values) in self.group_by_columns.iter().zip(columns.into_iter()) {
                dict.set_item(name, values)?;
            }
        }
        for (col_idx, col_name) in self.value_columns.iter().enumerate() {
            let values = self
                .value_rows
                .iter()
                .map(|row| row.get(col_idx).copied().unwrap_or(0.0))
                .collect::<Vec<_>>();
            dict.set_item(col_name, values)?;
        }
        Ok(dict.into_any())
    }

    fn to_polars<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_pandas(py)
    }
}
