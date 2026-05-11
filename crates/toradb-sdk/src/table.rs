use pyo3::prelude::*;
use pyo3::types::PyList;
use toradb_core::{Batch, ExecCtx};
use toradb_engine::tune_ctx;

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

fn exec_ctx(top_k: Option<u32>) -> ExecCtx {
    let k = top_k.unwrap_or(20);
    ExecCtx::new(k.saturating_mul(50).min(1000), k.saturating_mul(5).min(100), k)
}

#[pymethods]
impl Table {
    fn add(&self, docs: &Bound<'_, PyList>) -> PyResult<usize> {
        Ok(docs.len())
    }

    #[pyo3(signature = (query, top_k=None, strategy=None, explain=None, graph_expand=None, depth=None))]
    fn search(
        &self,
        py: Python<'_>,
        query: &str,
        top_k: Option<u32>,
        strategy: Option<&str>,
        explain: Option<bool>,
        graph_expand: Option<bool>,
        depth: Option<u32>,
    ) -> PyResult<SearchResults> {
        let mut db = self.db.borrow_mut(py);
        let mut batch = Batch::new();
        batch.query = query.to_string();
        batch.enable_hyde = matches!(strategy, Some("hyde"));
        batch.enable_crag = matches!(strategy, Some("crag"));
        batch.graph_expand = graph_expand.unwrap_or(false)
            || matches!(strategy, Some("graph") | Some("hybrid"));
        batch.graph_depth = depth.unwrap_or(2);
        let ctx = tune_ctx(exec_ctx(top_k), query, strategy);
        let metrics = db.run_retrieval(&mut batch, &ctx);
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
            ids: batch.candidates.ids,
            scores: batch.candidates.scores,
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
