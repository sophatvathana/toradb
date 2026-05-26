mod arrow_ingest;
mod database;
mod table;

use pyo3::prelude::*;

#[pyfunction]
#[pyo3(signature = (path, reload=None))]
fn local(path: &str, reload: Option<bool>) -> PyResult<database::Database> {
    database::Database::open_with_reload(path.to_string(), reload.unwrap_or(true))
}

#[pyfunction]
#[pyo3(signature = (path, reload=None))]
fn connect(path: &str, reload: Option<bool>) -> PyResult<database::Database> {
    database::Database::open_with_reload(path.to_string(), reload.unwrap_or(true))
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
