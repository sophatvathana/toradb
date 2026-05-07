use pyo3::prelude::*;
use pyo3::types::PyList;
use toradb_core::{Batch, ExecCtx};
use toradb_engine::DagRunner;

#[pyclass]
pub struct Table {
    pub name: String,
}

#[pymethods]
impl Table {
    fn add(&self, docs: &Bound<'_, PyList>) -> PyResult<usize> {
        Ok(docs.len())
    }

    #[pyo3(signature = (query, top_k=None))]
    fn search(&self, query: &str, top_k: Option<u32>) -> PyResult<SearchResults> {
        let _ = (query, top_k);
        let mut dag = DagRunner::new();
        let mut batch = Batch::new();
        let ctx = ExecCtx::default();
        let metrics = dag.run(&mut batch, &ctx);
        Ok(SearchResults {
            ids: batch.candidates.ids,
            scores: batch.candidates.scores,
            metrics,
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
        format!(
            "tier1={} tier2={} tier3={} decompressions={}",
            self.metrics.tier1_candidates,
            self.metrics.tier2_candidates,
            self.metrics.tier3_candidates,
            self.metrics.decompressions
        )
    }
}
