use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};
use toradb_core::QueryMetrics;
use toradb_engine::{
    download_hf_dataset_with_progress, ingest_hf_bundle, ingest_jsonl, ingest_parquet, materialized,
    persist, sql_exec, DagRunner, HfIngestParams, MaterializedViewInfo,
};
use toradb_sql::{
    ast::{CreateTableStmt, Stmt},
    binder::Binder,
    catalog_store,
    parse,
};

const MAX_UPLOAD_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone)]
pub struct ServeConfig {
    pub db_path: PathBuf,
    pub listen_addr: String,
    pub static_dir: PathBuf,
}

#[derive(Clone)]
struct AppState {
    db_path: PathBuf,
    dag: Arc<Mutex<DagRunner>>,
    query_history: Arc<Mutex<Vec<QueryHistoryEntry>>>,
    tasks: Arc<Mutex<Vec<OpTask>>>,
    next_task_id: Arc<AtomicU64>,
    ingest_jobs: Arc<Mutex<Vec<IngestJob>>>,
    next_ingest_job_id: Arc<AtomicU64>,
    ingest_cancel: Arc<Mutex<HashSet<u64>>>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(serde_json::json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    db_path: String,
    tables: Vec<String>,
}

#[derive(Serialize)]
struct TableInfo {
    name: String,
    rows: usize,
    vector_dim: Option<usize>,
    state: String,
}

#[derive(Serialize)]
struct TableDetailResponse {
    name: String,
    rows: usize,
    vector_dim: Option<usize>,
    state: String,
    segment_count: u32,
    segment_workers: u32,
    query_mode: String,
    index_sidecars: Vec<String>,
    is_materialized_view: bool,
    bulk_ingest_active: bool,
}

#[derive(Serialize)]
struct MetricsResponse {
    table_count: usize,
    cache_hits: u64,
    cache_misses: u64,
    indexing_tables: usize,
    query_count: usize,
    avg_query_latency_ms: f64,
}

#[derive(Deserialize)]
struct SqlRequest {
    query: String,
}

#[derive(Serialize)]
struct QueryMetricsResponse {
    tier1_candidates: u32,
    tier2_candidates: u32,
    tier3_candidates: u32,
    decompressions: u32,
    cache_hits: u64,
    io_bytes: u64,
    segments_scanned: u32,
    segment_workers: u32,
}

#[derive(Serialize)]
struct SqlResponse {
    kind: String,
    columns: Vec<String>,
    rows: Vec<serde_json::Value>,
    latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    explain_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metrics: Option<QueryMetricsResponse>,
}

#[derive(Clone, Serialize)]
struct QueryHistoryEntry {
    query: String,
    kind: String,
    status: String,
    latency_ms: f64,
    executed_at_unix_secs: u64,
}

#[derive(Serialize)]
struct JobInfo {
    table: String,
    state: String,
    phase: Option<String>,
    segments_done: u32,
    segments_total: u32,
    message: Option<String>,
}

#[derive(Clone, Serialize)]
struct OpTask {
    id: u64,
    table: String,
    kind: String,
    state: String,
    message: Option<String>,
    started_at_unix_secs: u64,
    finished_at_unix_secs: Option<u64>,
}

#[derive(Deserialize)]
struct QueryPreviewParams {
    table: String,
    query: String,
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct SampleParams {
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct CompactBody {
    compact: Option<bool>,
}

#[derive(Deserialize)]
struct IngestBeginBody {
    table: String,
    drop_table: Option<bool>,
}

#[derive(Deserialize)]
struct IngestFinishBody {
    table: String,
    compact: Option<bool>,
}

#[derive(Deserialize)]
struct IngestHfBody {
    table: String,
    dataset: String,
    config: Option<String>,
    split: Option<String>,
    text_column: Option<String>,
    limit: Option<u64>,
}

#[derive(Serialize)]
struct IngestUploadResponse {
    rows_ingested: u64,
    table: String,
}

#[derive(Serialize)]
struct TaskStartedResponse {
    task_id: u64,
}

#[derive(Clone, Serialize)]
struct IngestJob {
    id: u64,
    table: String,
    source: String,
    state: String,
    phase: Option<String>,
    message: Option<String>,
    progress: Option<u8>,
    rows_ingested: u64,
    started_at_unix_secs: u64,
    finished_at_unix_secs: Option<u64>,
}

#[derive(Serialize)]
struct IngestJobStartedResponse {
    job_id: u64,
}

#[derive(Serialize)]
struct DdlResponse {
    ddl: String,
}

#[derive(Deserialize)]
struct CreateMvBody {
    name: String,
    query: String,
}

#[derive(Deserialize)]
struct FullCompactBody {
    full: Option<bool>,
}

pub fn serve_blocking(config: ServeConfig) -> Result<(), String> {
    let addr: SocketAddr = config
        .listen_addr
        .parse()
        .map_err(|e| format!("invalid listen address {}: {e}", config.listen_addr))?;

    if !config.static_dir.is_dir() {
        return Err(format!(
            "platform static assets not found: {} (build apps/platform first)",
            config.static_dir.display()
        ));
    }

    let dag = DagRunner::open_with_reload(&config.db_path, false)?;
    let state = AppState {
        db_path: config.db_path.clone(),
        dag: Arc::new(Mutex::new(dag)),
        query_history: Arc::new(Mutex::new(Vec::new())),
        tasks: Arc::new(Mutex::new(Vec::new())),
        next_task_id: Arc::new(AtomicU64::new(1)),
        ingest_jobs: Arc::new(Mutex::new(Vec::new())),
        next_ingest_job_id: Arc::new(AtomicU64::new(1)),
        ingest_cancel: Arc::new(Mutex::new(HashSet::new())),
    };

    let static_service =
        ServeDir::new(&config.static_dir).fallback(ServeFile::new(config.static_dir.join("index.html")));

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/tables", get(tables))
        .route("/api/tables/{name}", get(table_detail))
        .route("/api/tables/{name}/sample", get(table_sample))
        .route("/api/tables/{name}/finish", post(table_finish))
        .route("/api/tables/{name}/resume", post(table_resume))
        .route("/api/tables/{name}/drop", post(table_drop))
        .route("/api/tables/{name}/compact", post(table_compact))
        .route("/api/tables/{name}/ddl", get(table_ddl))
        .route("/api/tables/{name}/indexes", get(table_indexes))
        .route("/api/materialized-views", get(materialized_views).post(mv_create))
        .route("/api/materialized-views/{name}", get(mv_detail))
        .route(
            "/api/materialized-views/{name}/refresh",
            post(mv_refresh),
        )
        .route("/api/materialized-views/{name}/drop", post(mv_drop))
        .route("/api/metrics", get(metrics))
        .route("/api/jobs", get(jobs))
        .route("/api/tasks", get(list_tasks))
        .route("/api/sql", post(sql))
        .route("/api/query-preview", get(query_preview))
        .route("/api/query-history", get(query_history))
        .route("/api/ingest/begin", post(ingest_begin))
        .route("/api/ingest/upload", post(ingest_upload))
        .route("/api/ingest/hf", post(ingest_hf_handler))
        .route("/api/ingest/jobs", get(list_ingest_jobs))
        .route("/api/ingest/jobs/{id}", get(get_ingest_job))
        .route("/api/ingest/jobs/{id}/cancel", post(cancel_ingest_job))
        .route("/api/ingest/finish", post(ingest_finish))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
        .with_state(state)
        .fallback_service(static_service);

    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| e.to_string())?;
        axum::serve(listener, app).await.map_err(|e| e.to_string())
    })
}

async fn health(State(state): State<AppState>) -> Result<Json<HealthResponse>, ApiError> {
    let dag = state
        .dag
        .lock()
        .map_err(|_| ApiError::internal("failed to lock database state"))?;
    let tables = dag
        .list_tables()
        .map_err(|e| ApiError::internal(format!("failed to list tables: {e}")))?;
    Ok(Json(HealthResponse {
        status: "ok",
        db_path: state.db_path.display().to_string(),
        tables,
    }))
}

fn table_build_state(db_path: &Path, table: &str) -> String {
    persist::read_table_index_build_status(db_path, table)
        .map(|status| match status.state {
            toradb_engine::IndexBuildState::Building => "building",
            toradb_engine::IndexBuildState::Ready => "ready",
            toradb_engine::IndexBuildState::Failed => "failed",
        })
        .map(|s| s.to_string())
        .unwrap_or_else(|| "ready".to_string())
}

fn table_detail_for(
    state: &AppState,
    dag: &DagRunner,
    name: &str,
) -> Result<TableDetailResponse, ApiError> {
    let tables = dag
        .list_tables()
        .map_err(|e| ApiError::internal(format!("failed to list tables: {e}")))?;
    if !tables.iter().any(|t| t == name) {
        return Err(ApiError::not_found(format!("table not found: {name}")));
    }
    let segment_count = persist::table_segment_count(&state.db_path, name).unwrap_or(1);
    let segment_workers = persist::table_segment_workers(&state.db_path, name).unwrap_or(1);
    let query_mode = persist::table_query_mode(&state.db_path, name)
        .map(|m| format!("{m:?}"))
        .unwrap_or_else(|_| "default".to_string());
    let index_sidecars = dag
        .table_index_sidecars(name)
        .unwrap_or_default();
    let is_materialized_view = materialized::is_materialized_view(&state.db_path, name);
    Ok(TableDetailResponse {
        name: name.to_string(),
        rows: dag.table_row_count(name).unwrap_or(0),
        vector_dim: dag.vector_dim(name),
        state: table_build_state(&state.db_path, name),
        segment_count,
        segment_workers,
        query_mode,
        index_sidecars,
        is_materialized_view,
        bulk_ingest_active: dag.bulk_ingest_active(name),
    })
}

async fn tables(State(state): State<AppState>) -> Result<Json<Vec<TableInfo>>, ApiError> {
    let dag = state
        .dag
        .lock()
        .map_err(|_| ApiError::internal("failed to lock database state"))?;
    let names = dag
        .list_tables()
        .map_err(|e| ApiError::internal(format!("failed to list tables: {e}")))?;
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let rows = dag.table_row_count(&name).unwrap_or(0);
        let state_str = table_build_state(&state.db_path, &name);
        out.push(TableInfo {
            vector_dim: dag.vector_dim(&name),
            name,
            rows,
            state: state_str,
        });
    }
    Ok(Json(out))
}

async fn table_detail(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<TableDetailResponse>, ApiError> {
    let dag = state
        .dag
        .lock()
        .map_err(|_| ApiError::internal("failed to lock database state"))?;
    Ok(Json(table_detail_for(&state, &dag, &name)?))
}

async fn table_sample(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Query(params): Query<SampleParams>,
) -> Result<Json<SqlResponse>, ApiError> {
    let limit = params.limit.unwrap_or(20).max(1).min(500);
    let query = format!("SELECT id, score FROM {name} LIMIT {limit}");
    Ok(Json(execute_sql(&state, &query, false)?))
}

async fn materialized_views(
    State(state): State<AppState>,
) -> Result<Json<Vec<MaterializedViewInfo>>, ApiError> {
    let views = materialized::list_materialized_view_infos(&state.db_path)
        .map_err(ApiError::internal)?;
    Ok(Json(views))
}

async fn mv_detail(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<MaterializedViewInfo>, ApiError> {
    materialized::get_materialized_view_info(&state.db_path, &name).map_err(|e| {
        if e.contains("does not exist") {
            ApiError::not_found(e)
        } else {
            ApiError::internal(e)
        }
    }).map(Json)
}

async fn mv_create(
    State(state): State<AppState>,
    Json(body): Json<CreateMvBody>,
) -> Result<Json<MaterializedViewInfo>, ApiError> {
    if body.name.trim().is_empty() {
        return Err(ApiError::bad_request("name is required"));
    }
    let stmts = parse(&body.query).map_err(ApiError::bad_request)?;
    let Stmt::Select(sel) = stmts.into_iter().next().ok_or_else(|| {
        ApiError::bad_request("materialized view query must be a single SELECT")
    })? else {
        return Err(ApiError::bad_request("materialized view query must be SELECT"));
    };
    let name = body.name.trim().to_lowercase();
    let rows = {
        let mut dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        materialized::create_materialized_view(&mut dag, &state.db_path, &name, &sel)
            .map_err(ApiError::bad_request)?
    };
    let mut info =
        materialized::get_materialized_view_info(&state.db_path, &name).map_err(ApiError::internal)?;
    info.row_count = rows;
    Ok(Json(info))
}

async fn mv_refresh(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<MaterializedViewInfo>, ApiError> {
    let rows = {
        let mut dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        materialized::refresh_materialized_view(&mut dag, &state.db_path, &name)
            .map_err(ApiError::bad_request)?
    };
    let mut info =
        materialized::get_materialized_view_info(&state.db_path, &name).map_err(ApiError::internal)?;
    info.row_count = rows;
    Ok(Json(info))
}

async fn mv_drop(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    materialized::drop_materialized_view(&state.db_path, &name).map_err(ApiError::bad_request)?;
    Ok(Json(serde_json::json!({ "dropped": name })))
}

async fn table_ddl(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<DdlResponse>, ApiError> {
    let table = name.to_lowercase();
    let mut binder = load_binder(&state.db_path);
    let ddl = if let Some(manifest) = binder.catalog.get(&table) {
        format!("CREATE TABLE {} USING {:?}", manifest.name, manifest.index_mode)
    } else {
        let dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        if !dag
            .list_tables()
            .map_err(ApiError::internal)?
            .iter()
            .any(|t| t == &table)
        {
            return Err(ApiError::not_found(format!("table not found: {table}")));
        }
        format!("CREATE TABLE {table} USING HYBRID")
    };
    Ok(Json(DdlResponse { ddl }))
}

async fn table_indexes(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<SqlResponse>, ApiError> {
    let dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
    let table = name.to_lowercase();
    if !dag
        .list_tables()
        .map_err(ApiError::internal)?
        .iter()
        .any(|t| t == &table)
    {
        return Err(ApiError::not_found(format!("table not found: {table}")));
    }
    let indexes = dag.table_index_sidecars(&table).unwrap_or_default();
    let rows: Vec<serde_json::Value> = if indexes.is_empty() {
        vec![]
    } else {
        indexes
            .into_iter()
            .map(|idx| serde_json::json!({ "index": idx }))
            .collect()
    };
    Ok(Json(SqlResponse {
        kind: "show_indexes".to_string(),
        columns: vec!["index".to_string()],
        rows,
        latency_ms: 0.0,
        explain_text: None,
        metrics: None,
    }))
}

async fn table_compact(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<FullCompactBody>,
) -> Result<Json<TaskStartedResponse>, ApiError> {
    let full = body.full.unwrap_or(false);
    {
        let dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        if !dag
            .list_tables()
            .map_err(ApiError::internal)?
            .iter()
            .any(|t| t == &name)
        {
            return Err(ApiError::not_found(format!("table not found: {name}")));
        }
    }
    let id = spawn_compact_task(&state, name, full);
    Ok(Json(TaskStartedResponse { task_id: id }))
}

async fn metrics(State(state): State<AppState>) -> Result<Json<MetricsResponse>, ApiError> {
    let dag = state
        .dag
        .lock()
        .map_err(|_| ApiError::internal("failed to lock database state"))?;
    let names = dag
        .list_tables()
        .map_err(|e| ApiError::internal(format!("failed to list tables: {e}")))?;
    let cache = dag.cache_stats();
    let history = state
        .query_history
        .lock()
        .map_err(|_| ApiError::internal("failed to lock query history"))?;
    let avg_query_latency_ms = if history.is_empty() {
        0.0
    } else {
        history.iter().map(|h| h.latency_ms).sum::<f64>() / history.len() as f64
    };
    let indexing_tables = names
        .iter()
        .filter(|name| table_is_building(&state.db_path, name))
        .count();
    Ok(Json(MetricsResponse {
        table_count: names.len(),
        cache_hits: cache.hits,
        cache_misses: cache.misses,
        indexing_tables,
        query_count: history.len(),
        avg_query_latency_ms,
    }))
}

async fn jobs(State(state): State<AppState>) -> Result<Json<Vec<JobInfo>>, ApiError> {
    let dag = state
        .dag
        .lock()
        .map_err(|_| ApiError::internal("failed to lock database state"))?;
    let names = dag
        .list_tables()
        .map_err(|e| ApiError::internal(format!("failed to list tables: {e}")))?;
    let mut out = Vec::with_capacity(names.len());
    for table in names {
        let status = persist::read_table_index_build_status(&state.db_path, &table);
        if let Some(status) = status {
            out.push(JobInfo {
                table,
                state: match status.state {
                    toradb_engine::IndexBuildState::Building => "building".to_string(),
                    toradb_engine::IndexBuildState::Ready => "ready".to_string(),
                    toradb_engine::IndexBuildState::Failed => "failed".to_string(),
                },
                phase: status.phase.map(|p| format!("{p:?}").to_lowercase()),
                segments_done: status.segments_done,
                segments_total: status.segments_total,
                message: status.message,
            });
        } else {
            out.push(JobInfo {
                table,
                state: "ready".to_string(),
                phase: None,
                segments_done: 0,
                segments_total: 0,
                message: None,
            });
        }
    }
    Ok(Json(out))
}

async fn list_tasks(State(state): State<AppState>) -> Result<Json<Vec<OpTask>>, ApiError> {
    let tasks = state
        .tasks
        .lock()
        .map_err(|_| ApiError::internal("failed to lock tasks"))?;
    Ok(Json(tasks.clone()))
}

fn metrics_to_response(m: &QueryMetrics) -> QueryMetricsResponse {
    QueryMetricsResponse {
        tier1_candidates: m.tier1_candidates,
        tier2_candidates: m.tier2_candidates,
        tier3_candidates: m.tier3_candidates,
        decompressions: m.decompressions,
        cache_hits: m.cache_hits,
        io_bytes: m.io_bytes,
        segments_scanned: m.segments_scanned,
        segment_workers: m.segment_workers,
    }
}

fn push_history(state: &AppState, entry: QueryHistoryEntry) {
    if let Ok(mut history) = state.query_history.lock() {
        history.push(entry);
        if history.len() > 100 {
            let drain_count = history.len() - 100;
            history.drain(0..drain_count);
        }
    }
}

fn push_task(state: &AppState, task: OpTask) {
    if let Ok(mut tasks) = state.tasks.lock() {
        tasks.push(task);
        if tasks.len() > 50 {
            let excess = tasks.len() - 50;
            tasks.drain(0..excess);
        }
    }
}

fn update_task(state: &AppState, id: u64, state_str: &str, message: Option<String>) {
    if let Ok(mut tasks) = state.tasks.lock() {
        if let Some(task) = tasks.iter_mut().find(|t| t.id == id) {
            task.state = state_str.to_string();
            task.message = message;
            if state_str == "done" || state_str == "failed" {
                task.finished_at_unix_secs = Some(now_unix_secs());
            }
        }
    }
}

fn load_binder(db_path: &Path) -> Binder {
    let mut binder = Binder::new();
    if let Ok(cat) = catalog_store::load_catalog(db_path) {
        for manifest in cat.iter_tables() {
            binder.catalog.register(manifest.clone());
        }
    }
    binder
}

fn push_ingest_job(state: &AppState, table: String, source: &str) -> u64 {
    let id = state.next_ingest_job_id.fetch_add(1, Ordering::SeqCst);
    let job = IngestJob {
        id,
        table,
        source: source.to_string(),
        state: "running".to_string(),
        phase: Some("starting".to_string()),
        message: None,
        progress: Some(0),
        rows_ingested: 0,
        started_at_unix_secs: now_unix_secs(),
        finished_at_unix_secs: None,
    };
    if let Ok(mut jobs) = state.ingest_jobs.lock() {
        jobs.push(job);
        let len = jobs.len();
        if len > 30 {
            jobs.drain(0..len - 30);
        }
    }
    id
}

fn update_ingest_job(
    state: &AppState,
    id: u64,
    state_str: &str,
    phase: Option<String>,
    message: Option<String>,
    progress: Option<u8>,
    rows_ingested: Option<u64>,
) {
    if let Ok(mut jobs) = state.ingest_jobs.lock() {
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            job.state = state_str.to_string();
            if phase.is_some() {
                job.phase = phase;
            }
            if message.is_some() {
                job.message = message;
            }
            if let Some(p) = progress {
                job.progress = Some(job.progress.map_or(p, |cur| cur.max(p)));
            }
            if let Some(rows) = rows_ingested {
                job.rows_ingested = rows;
            }
            if state_str == "done" || state_str == "failed" || state_str == "cancelled" {
                job.finished_at_unix_secs = Some(now_unix_secs());
            }
        }
    }
}

fn is_ingest_cancelled(state: &AppState, id: u64) -> bool {
    state
        .ingest_cancel
        .lock()
        .map(|set| set.contains(&id))
        .unwrap_or(false)
}

fn spawn_compact_task(state: &AppState, table: String, full: bool) -> u64 {
    let id = state.next_task_id.fetch_add(1, Ordering::SeqCst);
    push_task(
        state,
        OpTask {
            id,
            table: table.clone(),
            kind: "compact".to_string(),
            state: "running".to_string(),
            message: None,
            started_at_unix_secs: now_unix_secs(),
            finished_at_unix_secs: None,
        },
    );

    let db_path = state.db_path.clone();
    let snapshot = AppState {
        db_path: state.db_path.clone(),
        dag: Arc::clone(&state.dag),
        query_history: Arc::clone(&state.query_history),
        tasks: Arc::clone(&state.tasks),
        next_task_id: Arc::clone(&state.next_task_id),
        ingest_jobs: Arc::clone(&state.ingest_jobs),
        next_ingest_job_id: Arc::clone(&state.next_ingest_job_id),
        ingest_cancel: Arc::clone(&state.ingest_cancel),
    };

    thread::spawn(move || {
        let result = (|| {
            let mut dag = DagRunner::open_with_reload(&db_path, false)?;
            dag.compact_table(&table, full).map(|r| {
                format!(
                    "merges={} segments {} -> {}",
                    r.merges, r.segments_before, r.segments_after
                )
            })
        })();
        match result {
            Ok(msg) => update_task(&snapshot, id, "done", Some(msg)),
            Err(e) => update_task(&snapshot, id, "failed", Some(e)),
        }
        if let Ok(mut dag) = snapshot.dag.lock() {
            if let Ok(fresh) = DagRunner::open_with_reload(&snapshot.db_path, false) {
                *dag = fresh;
            }
        }
    });

    id
}

fn spawn_index_task(state: &AppState, table: String, kind: &'static str, compact: bool) -> u64 {
    let id = state.next_task_id.fetch_add(1, Ordering::SeqCst);
    let started = now_unix_secs();
    push_task(
        state,
        OpTask {
            id,
            table: table.clone(),
            kind: kind.to_string(),
            state: "running".to_string(),
            message: None,
            started_at_unix_secs: started,
            finished_at_unix_secs: None,
        },
    );

    let db_path = state.db_path.clone();
    let tasks = Arc::clone(&state.tasks);
    let dag_arc = Arc::clone(&state.dag);
    let next_id = Arc::clone(&state.next_task_id);

    let snapshot = AppState {
        db_path: state.db_path.clone(),
        dag: dag_arc,
        query_history: Arc::clone(&state.query_history),
        tasks,
        next_task_id: next_id,
        ingest_jobs: Arc::clone(&state.ingest_jobs),
        next_ingest_job_id: Arc::clone(&state.next_ingest_job_id),
        ingest_cancel: Arc::clone(&state.ingest_cancel),
    };

    thread::spawn(move || {
        let result = (|| {
            let mut dag = DagRunner::open_with_reload(&db_path, false)?;
            if kind == "finish" && dag.bulk_ingest_active(&table) {
                dag.finish_bulk_ingest(&table, compact)
            } else {
                dag.resume_index_build(&table, compact)
            }
        })();
        match result {
            Ok(()) => update_task(&snapshot, id, "done", None),
            Err(e) => update_task(&snapshot, id, "failed", Some(e)),
        }
        if let Ok(mut dag) = snapshot.dag.lock() {
            if let Ok(fresh) = DagRunner::open_with_reload(&snapshot.db_path, false) {
                *dag = fresh;
            }
        }
    });

    id
}

fn qualified_table_name(t: &CreateTableStmt) -> String {
    if let Some(ns) = &t.namespace {
        format!("{ns}.{}", t.name).to_lowercase()
    } else {
        t.name.to_lowercase()
    }
}

fn message_response(kind: &str, text: &str, started: Instant) -> SqlResponse {
    SqlResponse {
        kind: kind.to_string(),
        columns: vec!["message".to_string()],
        rows: vec![serde_json::json!({ "message": text })],
        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
        explain_text: None,
        metrics: None,
    }
}

fn tabular_response(
    kind: &str,
    columns: Vec<String>,
    rows: Vec<serde_json::Value>,
    started: Instant,
) -> SqlResponse {
    SqlResponse {
        kind: kind.to_string(),
        columns,
        rows,
        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
        explain_text: None,
        metrics: None,
    }
}

fn execute_sql(
    state: &AppState,
    query: &str,
    record_history: bool,
) -> Result<SqlResponse, ApiError> {
    let started = Instant::now();
    let mut dag = state
        .dag
        .lock()
        .map_err(|_| ApiError::internal("failed to lock database state"))?;

    let stmts = parse(query).map_err(|e| {
        let err = ApiError::bad_request(e.to_string());
        if record_history {
            push_history(
                state,
                QueryHistoryEntry {
                    query: query.chars().take(500).collect(),
                    kind: "sql".to_string(),
                    status: "error".to_string(),
                    latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                    executed_at_unix_secs: now_unix_secs(),
                },
            );
        }
        err
    })?;

    if stmts.len() != 1 {
        let err = ApiError::bad_request("exactly one SQL statement is required per request");
        if record_history {
            record_error_history(state, query, started);
        }
        return Err(err);
    }

    let resp = match &stmts[0] {
        Stmt::Select(sel) => match sql_exec::run_select(&mut dag, sel) {
        Ok(result) => match result {
            sql_exec::SqlSelectResult::Search(result) => {
                if let Some(text) = result.explain_text {
                    SqlResponse {
                        kind: "explain".to_string(),
                        columns: vec![],
                        rows: vec![],
                        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                        explain_text: Some(text),
                        metrics: Some(metrics_to_response(&result.metrics)),
                    }
                } else {
                    let mut rows = Vec::with_capacity(result.ids.len());
                    for row_idx in 0..result.ids.len() {
                        let mut row = serde_json::Map::new();
                        for (name, col) in &result.projected {
                            let value = match col {
                                sql_exec::SqlProjectedColumn::U64(values) => {
                                    serde_json::json!(values[row_idx])
                                }
                                sql_exec::SqlProjectedColumn::F32(values) => {
                                    serde_json::json!(values[row_idx])
                                }
                                sql_exec::SqlProjectedColumn::Str(values) => {
                                    serde_json::json!(values[row_idx])
                                }
                            };
                            row.insert(name.clone(), value);
                        }
                        rows.push(serde_json::Value::Object(row));
                    }
                    SqlResponse {
                        kind: "search".to_string(),
                        columns: result.projected.iter().map(|(n, _)| n.clone()).collect(),
                        rows,
                        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                        explain_text: None,
                        metrics: Some(metrics_to_response(&result.metrics)),
                    }
                }
            }
            sql_exec::SqlSelectResult::Aggregate(result) => {
                let mut rows = Vec::with_capacity(result.group_keys.len());
                for (row_idx, key) in result.group_keys.iter().enumerate() {
                    let mut row = serde_json::Map::new();
                    if let Some(first) = result.group_by_columns.first() {
                        row.insert(first.clone(), serde_json::json!(key));
                    }
                    for (value_idx, value_col) in result.value_columns.iter().enumerate() {
                        let value = result
                            .value_rows
                            .get(row_idx)
                            .and_then(|vals| vals.get(value_idx))
                            .copied()
                            .unwrap_or_default();
                        row.insert(value_col.clone(), serde_json::json!(value));
                    }
                    rows.push(serde_json::Value::Object(row));
                }
                let mut columns = result.group_by_columns.clone();
                columns.extend(result.value_columns.clone());
                SqlResponse {
                    kind: "aggregate".to_string(),
                    columns,
                    rows,
                    latency_ms: started.elapsed().as_secs_f64() * 1000.0,
                    explain_text: None,
                    metrics: None,
                }
            }
        },
        Err(e) => {
            if record_history {
                record_error_history(state, query, started);
            }
            return Err(ApiError::bad_request(e));
        }
        },
        Stmt::CreateTable(t) => {
            let mut binder = load_binder(&state.db_path);
            binder.bind(&stmts).map_err(|e| {
                if record_history {
                    record_error_history(state, query, started);
                }
                ApiError::bad_request(e)
            })?;
            let table = qualified_table_name(t);
            dag.ensure_table(&table);
            let _ = catalog_store::save_catalog(&state.db_path, &binder.catalog);
            message_response("ddl", &format!("ok: created table {table}"), started)
        }
        Stmt::ShowTables => {
            let names = dag.list_tables().map_err(|e| ApiError::bad_request(e))?;
            let mut rows = Vec::new();
            for name in names {
                let n = dag.table_row_count(&name).unwrap_or(0);
                rows.push(serde_json::json!({ "table": name, "rows": n }));
            }
            tabular_response(
                "show_tables",
                vec!["table".to_string(), "rows".to_string()],
                rows,
                started,
            )
        }
        Stmt::ShowMaterializedViews => {
            let views = materialized::list_materialized_view_infos(&state.db_path)
                .map_err(ApiError::bad_request)?;
            let rows: Vec<serde_json::Value> = views
                .into_iter()
                .map(|v| serde_json::json!({ "view": v.name, "rows": v.row_count }))
                .collect();
            tabular_response(
                "show_mvs",
                vec!["view".to_string(), "rows".to_string()],
                rows,
                started,
            )
        }
        Stmt::ShowIndexes { table } => {
            let table = table.to_lowercase();
            let indexes = dag.table_index_sidecars(&table).unwrap_or_default();
            let rows: Vec<serde_json::Value> = indexes
                .into_iter()
                .map(|idx| serde_json::json!({ "index": idx }))
                .collect();
            tabular_response(
                "show_indexes",
                vec!["index".to_string()],
                rows,
                started,
            )
        }
        Stmt::ShowCreateTable { table } => {
            let table = table.to_lowercase();
            let mut binder = load_binder(&state.db_path);
            let ddl = if let Some(manifest) = binder.catalog.get(&table) {
                format!("CREATE TABLE {} USING {:?}", manifest.name, manifest.index_mode)
            } else {
                format!("CREATE TABLE {table} USING HYBRID")
            };
            message_response("show_create", &ddl, started)
        }
        Stmt::CreateMaterializedView(mv) => {
            let rows = materialized::create_materialized_view(
                &mut dag,
                &state.db_path,
                &mv.name,
                &mv.select,
            )
            .map_err(|e| {
                if record_history {
                    record_error_history(state, query, started);
                }
                ApiError::bad_request(e)
            })?;
            message_response(
                "ddl",
                &format!("ok: created materialized view {} ({} rows)", mv.name, rows),
                started,
            )
        }
        Stmt::RefreshMaterializedView { name } => {
            let rows = materialized::refresh_materialized_view(&mut dag, &state.db_path, name)
                .map_err(ApiError::bad_request)?;
            message_response(
                "ddl",
                &format!("ok: refreshed materialized view {name} ({rows} rows)"),
                started,
            )
        }
        Stmt::DropMaterializedView { name } => {
            materialized::drop_materialized_view(&state.db_path, name)
                .map_err(ApiError::bad_request)?;
            message_response("ddl", &format!("ok: dropped materialized view {name}"), started)
        }
        Stmt::CompactTable { table, full } => {
            let report = dag
                .compact_table(&table.to_lowercase(), *full)
                .map_err(ApiError::bad_request)?;
            message_response(
                "ddl",
                &format!(
                    "ok: compacted {table} merges={} segments {} -> {}",
                    report.merges, report.segments_before, report.segments_after
                ),
                started,
            )
        }
        Stmt::DropTable { name } => {
            let table = name.to_lowercase();
            let table_dir = state.db_path.join(&table);
            if table_dir.exists() {
                std::fs::remove_dir_all(&table_dir)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
            }
            let fresh =
                DagRunner::open_with_reload(&state.db_path, false).map_err(ApiError::internal)?;
            *dag = fresh;
            message_response("ddl", &format!("ok: dropped table {table}"), started)
        }
        Stmt::Describe { name } => {
            let table = name.to_lowercase();
            let text = if materialized::is_materialized_view(&state.db_path, &table) {
                let rows = materialized::load_view_row_count(&state.db_path, &table)
                    .map_err(ApiError::bad_request)?;
                format!("materialized_view: {table}\nrows: {rows}")
            } else {
                let row_count = dag.table_row_count(&table).unwrap_or(0);
                let vector_dim = dag
                    .vector_dim(&table)
                    .map(|d| d.to_string())
                    .unwrap_or_else(|| "none".to_string());
                format!("table: {table}\nrows: {row_count}\nvector_dim: {vector_dim}")
            };
            message_response("describe", &text, started)
        }
        _ => {
            let err = ApiError::bad_request("unsupported SQL statement for /api/sql");
            if record_history {
                record_error_history(state, query, started);
            }
            return Err(err);
        }
    };

    if record_history {
        push_history(
            state,
            QueryHistoryEntry {
                query: query.chars().take(500).collect(),
                kind: resp.kind.clone(),
                status: "ok".to_string(),
                latency_ms: resp.latency_ms,
                executed_at_unix_secs: now_unix_secs(),
            },
        );
    }

    Ok(resp)
}

fn record_error_history(state: &AppState, query: &str, started: Instant) {
    push_history(
        state,
        QueryHistoryEntry {
            query: query.chars().take(500).collect(),
            kind: "sql".to_string(),
            status: "error".to_string(),
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            executed_at_unix_secs: now_unix_secs(),
        },
    );
}

async fn sql(
    State(state): State<AppState>,
    Json(req): Json<SqlRequest>,
) -> Result<Json<SqlResponse>, ApiError> {
    Ok(Json(execute_sql(&state, &req.query, true)?))
}

async fn query_preview(
    State(state): State<AppState>,
    Query(params): Query<QueryPreviewParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let stmt = format!(
        "SELECT id, score FROM {} SPARSE SEARCH text BM25('{}') LIMIT {}",
        params.table,
        params.query.replace('\'', "''"),
        params.limit.unwrap_or(10).max(1)
    );
    let resp = execute_sql(&state, &stmt, false)?;
    Ok(Json(serde_json::json!({
        "statement": stmt,
        "result": resp,
    })))
}

async fn table_finish(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<CompactBody>,
) -> Result<Json<TaskStartedResponse>, ApiError> {
    let compact = body.compact.unwrap_or(false);
    {
        let dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        if !dag
            .list_tables()
            .map_err(ApiError::internal)?
            .iter()
            .any(|t| t == &name)
        {
            return Err(ApiError::not_found(format!("table not found: {name}")));
        }
    }
    let id = spawn_index_task(&state, name, "finish", compact);
    Ok(Json(TaskStartedResponse { task_id: id }))
}

async fn table_resume(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<CompactBody>,
) -> Result<Json<TaskStartedResponse>, ApiError> {
    let compact = body.compact.unwrap_or(false);
    {
        let dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        if !dag
            .list_tables()
            .map_err(ApiError::internal)?
            .iter()
            .any(|t| t == &name)
        {
            return Err(ApiError::not_found(format!("table not found: {name}")));
        }
    }
    let id = spawn_index_task(&state, name, "resume", compact);
    Ok(Json(TaskStartedResponse { task_id: id }))
}

async fn table_drop(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let table_dir = state.db_path.join(&name);
    if table_dir.exists() {
        std::fs::remove_dir_all(&table_dir).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    let fresh = DagRunner::open_with_reload(&state.db_path, false).map_err(ApiError::internal)?;
    let mut dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
    *dag = fresh;
    Ok(Json(serde_json::json!({ "dropped": name })))
}

async fn ingest_begin(
    State(state): State<AppState>,
    Json(body): Json<IngestBeginBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if body.drop_table.unwrap_or(false) {
        let table_dir = state.db_path.join(&body.table);
        if table_dir.exists() {
            std::fs::remove_dir_all(&table_dir).map_err(|e| ApiError::internal(e.to_string()))?;
        }
    }
    let mut dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
    dag.ensure_table(&body.table);
    dag.begin_bulk_ingest(&body.table);
    Ok(Json(serde_json::json!({
        "table": body.table,
        "bulk_active": true,
    })))
}

async fn ingest_upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<IngestJobStartedResponse>, ApiError> {
    let mut table = String::new();
    let mut format = String::new();
    let mut limit: u64 = 0;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename = "upload.bin".to_string();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "table" => table = field.text().await.map_err(|e| ApiError::bad_request(e.to_string()))?,
            "format" => {
                format = field.text().await.map_err(|e| ApiError::bad_request(e.to_string()))?
            }
            "limit" => {
                let t = field.text().await.map_err(|e| ApiError::bad_request(e.to_string()))?;
                limit = t.parse().unwrap_or(0);
            }
            "file" => {
                filename = field.file_name().unwrap_or("upload.bin").to_string();
                file_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| ApiError::bad_request(e.to_string()))?
                        .to_vec(),
                );
            }
            _ => {}
        }
    }

    if table.is_empty() {
        return Err(ApiError::bad_request("table is required"));
    }
    let bytes = file_bytes.ok_or_else(|| ApiError::bad_request("file is required"))?;

    let ext = if format == "jsonl" {
        "jsonl"
    } else if format == "parquet" || filename.ends_with(".parquet") {
        "parquet"
    } else if filename.ends_with(".jsonl") {
        "jsonl"
    } else {
        return Err(ApiError::bad_request("format must be parquet or jsonl"));
    };

    let temp_dir = state.db_path.join(".platform_uploads");
    std::fs::create_dir_all(&temp_dir).map_err(|e| ApiError::internal(e.to_string()))?;
    let temp_path = temp_dir.join(format!(
        "{}_{}",
        now_unix_secs(),
        if ext == "parquet" {
            "upload.parquet"
        } else {
            "upload.jsonl"
        }
    ));
    std::fs::write(&temp_path, &bytes).map_err(|e| ApiError::internal(e.to_string()))?;

    {
        let mut dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        if !dag.bulk_ingest_active(&table) {
            dag.ensure_table(&table);
            dag.begin_bulk_ingest(&table);
        }
    }

    let format: &'static str = if ext == "parquet" { "parquet" } else { "jsonl" };
    let job_id = push_ingest_job(&state, table.clone(), "upload");
    update_ingest_job(
        &state,
        job_id,
        "running",
        Some("upload received".into()),
        None,
        Some(5),
        None,
    );

    let worker_state = state.clone();
    let path = temp_path.clone();
    tokio::spawn(async move {
        run_upload_ingest_job(worker_state, job_id, table, path, format, limit).await;
    });

    Ok(Json(IngestJobStartedResponse { job_id }))
}

async fn ingest_hf_handler(
    State(state): State<AppState>,
    Json(body): Json<IngestHfBody>,
) -> Result<Json<IngestJobStartedResponse>, ApiError> {
    if body.dataset.trim().is_empty() {
        return Err(ApiError::bad_request("dataset is required"));
    }
    if body.table.trim().is_empty() {
        return Err(ApiError::bad_request("table is required"));
    }

    let params = HfIngestParams {
        dataset: body.dataset.trim().to_string(),
        config: body.config.filter(|s| !s.is_empty()),
        split: body.split.unwrap_or_else(|| "train".to_string()),
        text_column: body
            .text_column
            .unwrap_or_else(|| "text".to_string()),
        limit: body.limit.unwrap_or(0),
    };

    let table = body.table.clone();
    {
        let mut dag = state.dag.lock().map_err(|_| ApiError::internal("lock"))?;
        if !dag.bulk_ingest_active(&table) {
            dag.ensure_table(&table);
            dag.begin_bulk_ingest(&table);
        }
    }

    let job_id = push_ingest_job(&state, table.clone(), "hf");
    let worker_state = state.clone();
    tokio::spawn(async move {
        run_hf_ingest_job(worker_state, job_id, table, params).await;
    });

    Ok(Json(IngestJobStartedResponse { job_id }))
}

async fn run_hf_ingest_job(state: AppState, job_id: u64, table: String, params: HfIngestParams) {
    update_ingest_job(
        &state,
        job_id,
        "running",
        Some("resolving dataset".into()),
        None,
        Some(2),
        None,
    );

    if is_ingest_cancelled(&state, job_id) {
        update_ingest_job(
            &state,
            job_id,
            "cancelled",
            Some("cancelled".into()),
            None,
            None,
            None,
        );
        return;
    }

    let temp_dir = state.db_path.join(".platform_uploads");
    let progress_state = state.clone();
    let bundle_result = download_hf_dataset_with_progress(
        &temp_dir,
        &params,
        Some(move |phase: &str, progress: Option<u8>| {
            if is_ingest_cancelled(&progress_state, job_id) {
                return;
            }
            update_ingest_job(
                &progress_state,
                job_id,
                "running",
                Some(phase.to_string()),
                None,
                progress,
                None,
            );
        }),
    )
    .await;

    if is_ingest_cancelled(&state, job_id) {
        update_ingest_job(
            &state,
            job_id,
            "cancelled",
            Some("cancelled".into()),
            None,
            None,
            None,
        );
        return;
    }

    let bundle = match bundle_result {
        Ok(b) => b,
        Err(e) => {
            update_ingest_job(
                &state,
                job_id,
                "failed",
                Some("failed".into()),
                Some(e),
                None,
                None,
            );
            return;
        }
    };

    update_ingest_job(
        &state,
        job_id,
        "running",
        Some("ingesting".into()),
        None,
        Some(88),
        None,
    );

    let db_path = state.db_path.clone();
    let limit = params.limit;
    let ingest_result = tokio::task::spawn_blocking(move || {
        let mut dag = DagRunner::open_with_reload(&db_path, false)?;
        ingest_hf_bundle(&mut dag, &table, &bundle, limit)
    })
    .await;

    let rows = match ingest_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            update_ingest_job(
                &state,
                job_id,
                "failed",
                Some("ingesting".into()),
                Some(e),
                None,
                None,
            );
            return;
        }
        Err(e) => {
            update_ingest_job(
                &state,
                job_id,
                "failed",
                Some("ingesting".into()),
                Some(e.to_string()),
                None,
                None,
            );
            return;
        }
    };

    if let Ok(mut dag) = state.dag.lock() {
        if let Ok(fresh) = DagRunner::open_with_reload(&state.db_path, false) {
            *dag = fresh;
        }
    }

    update_ingest_job(
        &state,
        job_id,
        "done",
        Some("complete".into()),
        None,
        Some(100),
        Some(rows),
    );
}

async fn run_upload_ingest_job(
    state: AppState,
    job_id: u64,
    table: String,
    temp_path: PathBuf,
    format: &'static str,
    limit: u64,
) {
    if is_ingest_cancelled(&state, job_id) {
        update_ingest_job(
            &state,
            job_id,
            "cancelled",
            Some("cancelled".into()),
            None,
            None,
            None,
        );
        return;
    }

    update_ingest_job(
        &state,
        job_id,
        "running",
        Some("ingesting".into()),
        None,
        Some(85),
        None,
    );

    let db_path = state.db_path.clone();
    let path = temp_path.clone();
    let ingest_result = tokio::task::spawn_blocking(move || {
        let mut dag = DagRunner::open_with_reload(&db_path, false)?;
        if !dag.bulk_ingest_active(&table) {
            dag.ensure_table(&table);
            dag.begin_bulk_ingest(&table);
        }
        let result = if format == "parquet" {
            ingest_parquet(&mut dag, &table, &path, limit)
        } else {
            ingest_jsonl(&mut dag, &table, &path, 200_000, limit)
        };
        let _ = std::fs::remove_file(&path);
        result
    })
    .await;

    let rows = match ingest_result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&temp_path);
            update_ingest_job(
                &state,
                job_id,
                "failed",
                Some("ingesting".into()),
                Some(e),
                None,
                None,
            );
            return;
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            update_ingest_job(
                &state,
                job_id,
                "failed",
                Some("ingesting".into()),
                Some(e.to_string()),
                None,
                None,
            );
            return;
        }
    };

    if let Ok(mut dag) = state.dag.lock() {
        if let Ok(fresh) = DagRunner::open_with_reload(&state.db_path, false) {
            *dag = fresh;
        }
    }

    update_ingest_job(
        &state,
        job_id,
        "done",
        Some("complete".into()),
        None,
        Some(100),
        Some(rows),
    );
}

async fn list_ingest_jobs(State(state): State<AppState>) -> Result<Json<Vec<IngestJob>>, ApiError> {
    let jobs = state
        .ingest_jobs
        .lock()
        .map_err(|_| ApiError::internal("lock"))?;
    Ok(Json(jobs.clone()))
}

async fn get_ingest_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<u64>,
) -> Result<Json<IngestJob>, ApiError> {
    let jobs = state
        .ingest_jobs
        .lock()
        .map_err(|_| ApiError::internal("lock"))?;
    jobs.iter()
        .find(|j| j.id == id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("ingest job not found: {id}")))
        .map(Json)
}

async fn cancel_ingest_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Ok(mut set) = state.ingest_cancel.lock() {
        set.insert(id);
    }
    update_ingest_job(
        &state,
        id,
        "cancelled",
        Some("cancelled".into()),
        Some("cancel requested".into()),
        None,
        None,
    );
    Ok(Json(serde_json::json!({ "cancelled": id })))
}

async fn ingest_finish(
    State(state): State<AppState>,
    Json(body): Json<IngestFinishBody>,
) -> Result<Json<TaskStartedResponse>, ApiError> {
    let compact = body.compact.unwrap_or(false);
    let id = spawn_index_task(&state, body.table, "finish", compact);
    Ok(Json(TaskStartedResponse { task_id: id }))
}

fn table_is_building(db_path: &Path, table: &str) -> bool {
    persist::read_table_index_build_status(db_path, table)
        .map(|status| matches!(status.state, toradb_engine::IndexBuildState::Building))
        .unwrap_or(false)
}

async fn query_history(State(state): State<AppState>) -> Result<Json<Vec<QueryHistoryEntry>>, ApiError> {
    let history = state
        .query_history
        .lock()
        .map_err(|_| ApiError::internal("failed to lock query history"))?;
    Ok(Json(history.clone()))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
