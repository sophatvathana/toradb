use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::path::Path;
use toradb_core::{Batch, ExecCtx, QueryMetrics};
use toradb_engine::{
    materialized, persist, run_table_search, sql_exec, DagRunner, IndexBuildPhase,
    IndexBuildState, IndexBuildStatus, TableSearchOptions, TableSearchResult,
};
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
        Self::open_with_reload(path, true)
    }

    pub fn open_with_reload(path: String, reload: bool) -> PyResult<Self> {
        let dag = DagRunner::open_with_reload(&path, reload)
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
        let mut binder = Binder::new();
        if let Ok(cat) = toradb_sql::catalog_store::load_catalog(Path::new(&path)) {
            for manifest in cat.iter_tables() {
                binder.catalog.register(manifest.clone());
            }
        }
        Ok(Self { path, dag, binder })
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
                        vec!["view".into()],
                        view_names,
                        vec!["rows".into()],
                        row_counts.into_iter().map(|v| vec![v]).collect(),
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
                        let n = self.dag.table_row_count(&name).unwrap_or(0);
                        table_names.push(name);
                        row_counts.push(n as f64);
                    }
                    return Ok(SqlOutcome::Aggregate(AnalyticsResults::new(
                        vec!["table".into()],
                        table_names,
                        vec!["rows".into()],
                        row_counts.into_iter().map(|v| vec![v]).collect(),
                    )));
                }
                Stmt::CreateTable(t) => {
                    self.binder.bind(&stmts).map_err(|e| pyo3::exceptions::PyValueError::new_err(e))?;
                    let table = if let Some(ns) = &t.namespace {
                        format!("{ns}.{}", t.name).to_lowercase()
                    } else {
                        t.name.to_lowercase()
                    };
                    self.ensure_table(&table);
                    let _ = toradb_sql::catalog_store::save_catalog(
                        Path::new(&self.path),
                        &self.binder.catalog,
                    );
                    return Ok(SqlOutcome::Message(format!("ok: created table {table}")));
                }
                Stmt::ShowIndexes { table } => {
                    let table = table.to_lowercase();
                    let indexes = self
                        .dag
                        .table_index_sidecars(&table)
                        .unwrap_or_default();
                    return Ok(SqlOutcome::Message(format!(
                        "indexes on {table}: {}",
                        if indexes.is_empty() {
                            "(none)".into()
                        } else {
                            indexes.join(", ")
                        }
                    )));
                }
                Stmt::ShowCreateTable { table } => {
                    let table = table.to_lowercase();
                    let manifest = self
                        .binder
                        .catalog
                        .get(&table)
                        .cloned()
                        .ok_or_else(|| {
                            pyo3::exceptions::PyValueError::new_err(format!(
                                "unknown table {table}"
                            ))
                        })?;
                    let ddl = format!(
                        "CREATE TABLE {} USING {:?}",
                        manifest.name, manifest.index_mode
                    );
                    return Ok(SqlOutcome::Message(ddl));
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
                    let row_count = self.dag.table_row_count(&table).unwrap_or(0);
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
                            SearchResults::from_sql(
                                s.ids,
                                s.scores,
                                s.projected,
                                s.metrics,
                                s.explain_text,
                            ),
                        )),
                        sql_exec::SqlSelectResult::Aggregate(a) => Ok(SqlOutcome::Aggregate(
                            AnalyticsResults::new(
                                a.group_by_columns,
                                a.group_keys,
                                a.value_columns,
                                a.value_rows,
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

    pub(crate) fn run_retrieval(
        &mut self,
        batch: &mut Batch,
        ctx: &ExecCtx,
    ) -> Result<QueryMetrics, String> {
        if !batch.table.is_empty() {
            self.dag.ensure_table_queryable(&batch.table)?;
        }
        Ok(self.dag.run(batch, ctx))
    }

    pub(crate) fn run_table_search(
        &mut self,
        opts: TableSearchOptions,
    ) -> Result<TableSearchResult, String> {
        run_table_search(&mut self.dag, opts)
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

    pub(crate) fn ingest_record_batch(
        &mut self,
        table: &str,
        batch: &arrow::record_batch::RecordBatch,
    ) -> Result<usize, String> {
        self.dag.ingest_record_batch(table, batch)
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

fn ingest_doc_to_py<'py>(
    py: Python<'py>,
    doc_id: u64,
    doc: &toradb_index::IngestDoc,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("id", doc_id)?;
    d.set_item("text", &doc.text)?;
    let meta = PyDict::new(py);
    for (k, v) in &doc.metadata {
        meta.set_item(k, v)?;
    }
    d.set_item("metadata", meta)?;
    Ok(d)
}

#[pymethods]
impl Database {
    #[new]
    fn py_new(py: Python<'_>, path: String) -> PyResult<Self> {
        py.detach(|| Self::open(path))
    }

    fn sql<'py>(&mut self, py: Python<'py>, query: &str) -> PyResult<Bound<'py, PyAny>> {
        match py.detach(|| self.execute_sql(query))? {
            SqlOutcome::Message(s) => Ok(s.into_pyobject(py)?.into_any()),
            SqlOutcome::Search(results) => Ok(results.into_pyobject(py)?.into_any()),
            SqlOutcome::Aggregate(results) => Ok(results.into_pyobject(py)?.into_any()),
        }
    }

    #[pyo3(signature = (table, limit=50))]
    fn search_log<'py>(
        &self,
        py: Python<'py>,
        table: &str,
        limit: usize,
    ) -> PyResult<Bound<'py, PyList>> {
        let base = self.dag.db_path().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("database has no on-disk path")
        })?;
        let records = persist::read_search_log(base, table, limit)
            .map_err(pyo3::exceptions::PyRuntimeError::new_err)?;
        let list = PyList::empty(py);
        for rec in &records {
            let value = serde_json::to_value(rec)
                .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
            list.append(crate::table::json_to_py(py, &value)?)?;
        }
        Ok(list)
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
        if !sel.group_by.is_empty() {
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
            let out = py
                .detach(|| sql_exec::run_select(&mut self.dag, &sel))
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
                SearchResults::from_sql(
                    page.ids,
                    page.scores,
                    page.projected,
                    page.metrics,
                    None,
                );
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

    /// Start bulk ingest: defer per-batch dense rebuilds and table index writes until [`Self::finish_bulk_ingest`].
    fn begin_bulk_ingest(&mut self, table: &str) {
        self.dag.begin_bulk_ingest(table);
    }

    pub(crate) fn bulk_ingest_active(&self, table: &str) -> bool {
        self.dag.bulk_ingest_active(table)
    }

    /// Finalize table indexes after bulk load. Optionally compact segments and reindex BM25.
    #[pyo3(signature = (table, compact=false, reindex_bm25=false))]
    fn finish_bulk_ingest(
        &mut self,
        table: &str,
        compact: bool,
        reindex_bm25: bool,
    ) -> PyResult<()> {
        self.dag
            .finish_bulk_ingest(table, compact)
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
        if reindex_bm25 {
            self.dag
                .resume_index_build(table, false)
                .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
        }
        Ok(())
    }

    /// LRU cache hits/misses for segment BM25, segment parquet, and index blobs.
    fn cache_stats(&self) -> (u64, u64) {
        let s = self.dag.cache_stats();
        (s.hits, s.misses)
    }

    /// Read on-disk index build progress without loading the full corpus.
    fn index_build_status(&self, table: &str) -> PyResult<Option<IndexBuildStatusPy>> {
        let Some(ref path) = self.dag.db_path() else {
            return Ok(None);
        };
        Ok(persist::read_table_index_build_status(&path, table).map(IndexBuildStatusPy::from))
    }

    /// Resume or run index build after crash or partial finish.
    fn resume_index_build(&mut self, table: &str, compact: bool) -> PyResult<()> {
        self.dag
            .resume_index_build(table, compact)
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
        Ok(())
    }

    /// Load text and metadata for doc ids from RAM or on-disk Parquet segments.
    #[pyo3(signature = (table, ids))]
    fn fetch_documents<'py>(
        &mut self,
        py: Python<'py>,
        table: &str,
        ids: Vec<u64>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let rows = py
            .detach(|| self.dag.fetch_documents(table, &ids))
            .map_err(|e| pyo3::exceptions::PyOSError::new_err(e))?;
        let out = PyDict::new(py);
        for (id, doc) in rows {
            out.set_item(id, ingest_doc_to_py(py, id, &doc)?)?;
        }
        Ok(out)
    }

    fn __repr__(&self) -> String {
        format!("Database({})", self.path)
    }
}

#[derive(Clone)]
#[pyclass(skip_from_py_object)]
pub struct IndexBuildStatusPy {
    #[pyo3(get)]
    pub state: String,
    #[pyo3(get)]
    pub phase: Option<String>,
    #[pyo3(get)]
    pub segments_done: u32,
    #[pyo3(get)]
    pub segments_total: u32,
    #[pyo3(get)]
    pub message: Option<String>,
    #[pyo3(get)]
    pub updated_unix_secs: u64,
}

impl From<IndexBuildStatus> for IndexBuildStatusPy {
    fn from(s: IndexBuildStatus) -> Self {
        let state = match s.state {
            IndexBuildState::Building => "building",
            IndexBuildState::Ready => "ready",
            IndexBuildState::Failed => "failed",
        };
        let phase = s.phase.map(|p| match p {
            IndexBuildPhase::SegmentBm25 => "segment_bm25",
            IndexBuildPhase::MergeBm25 => "merge_bm25",
            IndexBuildPhase::TableIndexes => "table_indexes",
            IndexBuildPhase::ReloadTexts => "reload_texts",
        }.to_string());
        Self {
            state: state.into(),
            phase,
            segments_done: s.segments_done,
            segments_total: s.segments_total,
            message: s.message,
            updated_unix_secs: s.updated_unix_secs,
        }
    }
}
