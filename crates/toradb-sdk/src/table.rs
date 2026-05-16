use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use toradb_core::{Batch, ExecCtx};
use toradb_engine::tune_ctx;
use pyo3_arrow::PyTable;
use toradb_index::{dense::query_embed::lexical_proxy_vector, IngestDoc};

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

fn exec_ctx(top_k: Option<u32>, offset: Option<u32>) -> ExecCtx {
    let k = top_k.unwrap_or(20);
    let off = offset.unwrap_or(0);
    let fetch = off.saturating_add(k).min(1000);
    ExecCtx::new(
        fetch.saturating_mul(50).min(1000),
        fetch.saturating_mul(5).min(100),
        fetch,
    )
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
        let parsed = crate::arrow_ingest::ingest_pytable(table)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
        let mut db = self.db.borrow_mut(py);
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
        let mut db = self.db.borrow_mut(py);
        let mut batch = Batch::new();
        batch.table = self.name.clone();
        batch.query = query.to_string();
        batch.query_vector = query_vector;
        batch.tier1_enable_sparse =
            !matches!(strategy, Some("dense") | Some("vector") | Some("hnsw"));
        batch.tier1_enable_dense = !matches!(strategy, Some("sparse") | Some("bm25"));
        if batch.tier1_enable_dense && batch.query_vector.is_none() {
            if let Some(dim) = db.vector_dim(&self.name) {
                batch.query_vector = Some(lexical_proxy_vector(query, dim));
            }
        }
        batch.enable_hyde = matches!(strategy, Some("hyde"));
        batch.enable_crag = matches!(strategy, Some("crag"));
        batch.graph_expand = graph_expand.unwrap_or(false)
            || matches!(strategy, Some("graph") | Some("hybrid"));
        batch.graph_depth = depth.unwrap_or(2);
        let ctx = tune_ctx(exec_ctx(top_k, offset), query, strategy);
        let metrics = db.run_retrieval(&mut batch, &ctx);
        let page = batch
            .candidates
            .slice_range(offset.unwrap_or(0) as usize, top_k.unwrap_or(20) as usize);
        let explain_text = if explain.unwrap_or(false) {
            Some(format!(
                "table={} strategy={:?} graph_expand={} depth={} hyde={} crag={} tier1={} tier2={} tier3={}",
                self.name,
                strategy,
                batch.graph_expand,
                batch.graph_depth,
                batch.enable_hyde,
                batch.enable_crag,
                metrics.tier1_candidates,
                metrics.tier2_candidates,
                metrics.tier3_candidates
            ))
        } else {
            None
        };
        Ok(SearchResults {
            ids: page.ids,
            scores: page.scores,
            metrics,
            explain_text,
        })
    }

    fn __repr__(&self) -> String {
        format!("Table({})", self.name)
    }
}

#[pyclass]
pub struct SearchResults {
    ids: Vec<u64>,
    scores: Vec<f32>,
    metrics: toradb_core::QueryMetrics,
    explain_text: Option<String>,
}

impl SearchResults {
    pub(crate) fn from_sql(
        ids: Vec<u64>,
        scores: Vec<f32>,
        metrics: toradb_core::QueryMetrics,
    ) -> Self {
        Self {
            ids,
            scores,
            metrics,
            explain_text: None,
        }
    }
}

#[pymethods]
impl SearchResults {
    fn to_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("id", self.ids.clone())?;
        dict.set_item("score", self.scores.clone())?;
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
}

#[pyclass]
pub struct AnalyticsResults {
    group_by_column: String,
    group_keys: Vec<String>,
    value_column: String,
    values: Vec<f64>,
}

impl AnalyticsResults {
    pub(crate) fn new(
        group_by_column: String,
        group_keys: Vec<String>,
        value_column: String,
        values: Vec<f64>,
    ) -> Self {
        Self {
            group_by_column,
            group_keys,
            value_column,
            values,
        }
    }
}

#[pymethods]
impl AnalyticsResults {
    fn to_pandas<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item(&self.group_by_column, self.group_keys.clone())?;
        dict.set_item(&self.value_column, self.values.clone())?;
        Ok(dict.into_any())
    }

    fn to_polars<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        self.to_pandas(py)
    }
}
