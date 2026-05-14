mod arrow_ingest;
mod database;
mod table;

use pyo3::prelude::*;

#[pyfunction]
fn local(path: &str) -> PyResult<database::Database> {
    database::Database::open(path.to_string())
}

#[pyfunction]
fn connect(path: &str) -> PyResult<database::Database> {
    database::Database::open(path.to_string())
}

#[pymodule]
fn _toradb_sdk(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(local, m)?)?;
    m.add_function(wrap_pyfunction!(connect, m)?)?;
    m.add_class::<database::Database>()?;
    m.add_class::<table::Table>()?;
    m.add_class::<table::SearchResults>()?;
    m.add_class::<table::AnalyticsResults>()?;
    Ok(())
}
