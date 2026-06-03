//! HTTP handlers for toraPipe: connections, pipelines, and sync jobs.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::{Multipart, Path as AxumPath, State};
use axum::http::{header, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use toradb_pipe::{
    open_sql_source, run_pipeline, ColumnMapping, JobReporter, Pipeline, RunGuard, SourceKind,
    SyncMode,
};

use super::{ApiError, AppState, IngestJob};

fn now_secs() -> u64 {
    super::now_unix_secs()
}

#[derive(Serialize)]
pub struct ConnectionResponse {
    id: String,
    name: String,
    kind: String,
    url_masked: String,
    created_at: u64,
}

#[derive(Deserialize)]
pub struct CreateConnectionBody {
    name: String,
    #[serde(default)]
    kind: Option<String>,
    url: String,
}

#[derive(Deserialize)]
pub struct TestConnectionBody {
    url: String,
}

#[derive(Serialize)]
pub struct MappingResponse {
    text_column: String,
    metadata_columns: Vec<String>,
    vector_column: Option<String>,
    id_column: Option<String>,
    cursor_column: Option<String>,
}

#[derive(Serialize)]
pub struct PipelineResponse {
    id: String,
    name: String,
    connection_id: String,
    query: String,
    target_table: String,
    mapping: MappingResponse,
    mode: String,
    last_cursor: Option<String>,
    schedule_interval_secs: Option<u64>,
    drop_table_on_full: bool,
    enabled: bool,
    batch_size: usize,
    has_embedder: bool,
    created_at: u64,
}

#[derive(Deserialize)]
pub struct CreatePipelineBody {
    name: String,
    connection_id: String,
    query: String,
    target_table: String,
    text_column: String,
    #[serde(default)]
    metadata_columns: Vec<String>,
    #[serde(default)]
    vector_column: Option<String>,
    #[serde(default)]
    id_column: Option<String>,
    #[serde(default)]
    cursor_column: Option<String>,
    /// `full` | `incremental` | `cdc`.
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    schedule_interval_secs: Option<u64>,
    #[serde(default)]
    drop_table_on_full: bool,
    #[serde(default)]
    batch_size: Option<usize>,
    #[serde(default)]
    enabled: Option<bool>,
    /// Optional embedder config (OpenAI-compatible HTTP or local).
    #[serde(default)]
    embedder: Option<toradb_pipe::EmbedderConfig>,
}

#[derive(Deserialize)]
pub struct PatchPipelineBody {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    schedule_interval_secs: Option<Option<u64>>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Serialize)]
pub struct PipelineRunResponse {
    id: u64,
    pipeline_id: String,
    state: String,
    phase: Option<String>,
    rows_synced: u64,
    started_at: u64,
    finished_at: Option<u64>,
    message: Option<String>,
    cursor_after: Option<String>,
}

#[derive(Serialize)]
pub struct SyncJobStarted {
    job_id: u64,
}

fn mask_url(url: &str) -> String {
    // scheme://[user[:pw]@]rest
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let Some((authority, tail)) = rest.split_once('@') else {
        return url.to_string();
    };
    let masked_auth = match authority.split_once(':') {
        Some((user, _pw)) => format!("{user}:***"),
        None => authority.to_string(),
    };
    format!("{scheme}://{masked_auth}@{tail}")
}

fn parse_kind(s: Option<&str>) -> SourceKind {
    match s.map(str::to_ascii_lowercase).as_deref() {
        Some("objectstore") | Some("object_store") | Some("s3") => SourceKind::ObjectStore,
        Some("http") => SourceKind::Http,
        _ => SourceKind::Sql,
    }
}

fn kind_str(k: SourceKind) -> &'static str {
    match k {
        SourceKind::Sql => "sql",
        SourceKind::ObjectStore => "objectstore",
        SourceKind::Http => "http",
    }
}

fn mode_str(m: SyncMode) -> &'static str {
    match m {
        SyncMode::Full => "full",
        SyncMode::Incremental => "incremental",
        SyncMode::Cdc => "cdc",
    }
}

fn parse_mode(s: Option<&str>) -> SyncMode {
    match s.map(str::to_ascii_lowercase).as_deref() {
        Some("incremental") => SyncMode::Incremental,
        Some("cdc") => SyncMode::Cdc,
        _ => SyncMode::Full,
    }
}

fn connection_response(c: &toradb_pipe::Connection) -> ConnectionResponse {
    ConnectionResponse {
        id: c.id.clone(),
        name: c.name.clone(),
        kind: kind_str(c.kind).to_string(),
        url_masked: mask_url(&c.url),
        created_at: c.created_at,
    }
}

fn pipeline_response(p: &Pipeline) -> PipelineResponse {
    PipelineResponse {
        id: p.id.clone(),
        name: p.name.clone(),
        connection_id: p.connection_id.clone(),
        query: p.query.clone(),
        target_table: p.target_table.clone(),
        mapping: MappingResponse {
            text_column: p.mapping.text_column.clone(),
            metadata_columns: p.mapping.metadata_columns.clone(),
            vector_column: p.mapping.vector_column.clone(),
            id_column: p.mapping.id_column.clone(),
            cursor_column: p.mapping.cursor_column.clone(),
        },
        mode: mode_str(p.mode).to_string(),
        last_cursor: p.last_cursor.clone(),
        schedule_interval_secs: p.schedule.as_ref().map(|s| s.interval_secs),
        drop_table_on_full: p.drop_table_on_full,
        enabled: p.enabled,
        batch_size: p.batch_size,
        has_embedder: p.embedder.is_some(),
        created_at: p.created_at,
    }
}

fn lock_store<'a>(
    state: &'a AppState,
) -> Result<std::sync::MutexGuard<'a, toradb_pipe::PipeStore>, ApiError> {
    state
        .pipe_store
        .lock()
        .map_err(|_| ApiError::internal("pipe store lock poisoned"))
}

pub async fn list_connections(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConnectionResponse>>, ApiError> {
    let store = lock_store(&state)?;
    Ok(Json(
        store
            .connections()
            .iter()
            .map(connection_response)
            .collect(),
    ))
}

pub async fn create_connection(
    State(state): State<AppState>,
    Json(body): Json<CreateConnectionBody>,
) -> Result<Json<ConnectionResponse>, ApiError> {
    if body.name.trim().is_empty() || body.url.trim().is_empty() {
        return Err(ApiError::bad_request("name and url are required"));
    }
    let kind = parse_kind(body.kind.as_deref());
    let mut store = lock_store(&state)?;
    let conn = store
        .add_connection(body.name, kind, body.url)
        .map_err(ApiError::internal)?;
    Ok(Json(connection_response(&conn)))
}

pub async fn upload_connection(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<ConnectionResponse>, ApiError> {
    let mut name = String::new();
    let mut filename = String::new();
    let mut file_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        match field.name().unwrap_or("") {
            "name" => {
                name = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?
            }
            "file" => {
                filename = field.file_name().unwrap_or("upload.db").to_string();
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

    let bytes = file_bytes.ok_or_else(|| ApiError::bad_request("file is required"))?;
    if name.trim().is_empty() {
        name = filename.clone();
    }
    let ext = std::path::Path::new(&filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("db")
        .to_lowercase();
    if !matches!(ext.as_str(), "db" | "sqlite" | "sqlite3") {
        return Err(ApiError::bad_request(
            "uploadable connections support SQLite files (.db/.sqlite/.sqlite3)",
        ));
    }

    let dir = state.db_path.join(".torapipe").join("files");
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
    let stamped = format!("{}_{}.{}", super::now_unix_secs(), sanitize(&filename), ext);
    let path = dir.join(&stamped);
    std::fs::write(&path, &bytes).map_err(|e| ApiError::internal(e.to_string()))?;

    let abs = path.canonicalize().unwrap_or(path).display().to_string();
    let url = format!("sqlite://{abs}?mode=ro");

    if let Err(e) = toradb_pipe::validate_sqlite(&url).await {
        let _ = std::fs::remove_file(&dir.join(&stamped));
        return Err(ApiError::bad_request(format!(
            "not a valid SQLite database: {e}"
        )));
    }

    let mut store = lock_store(&state)?;
    let conn = store
        .add_connection(name, SourceKind::Sql, url)
        .map_err(ApiError::internal)?;
    Ok(Json(connection_response(&conn)))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(40)
        .collect()
}

pub async fn delete_connection(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut store = lock_store(&state)?;
    let uploaded_file = store
        .connection(&id)
        .and_then(|c| uploaded_sqlite_path(&state, &c.url));
    let removed = store.delete_connection(&id).map_err(ApiError::internal)?;
    if removed {
        if let Some(p) = uploaded_file {
            let _ = std::fs::remove_file(p);
        }
    }
    Ok(Json(serde_json::json!({ "deleted": removed })))
}

fn uploaded_sqlite_path(state: &AppState, url: &str) -> Option<std::path::PathBuf> {
    let rest = url.strip_prefix("sqlite://")?;
    let path = rest.split('?').next().unwrap_or(rest);
    let p = std::path::PathBuf::from(path);
    let files_dir = state.db_path.join(".torapipe").join("files");
    let files_dir = files_dir.canonicalize().unwrap_or(files_dir);
    let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
    if canon.starts_with(&files_dir) {
        Some(canon)
    } else {
        None
    }
}

pub async fn test_connection(
    Json(body): Json<TestConnectionBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    match toradb_pipe::test_connection(&body.url).await {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true }))),
        Err(e) => Ok(Json(serde_json::json!({ "ok": false, "error": e }))),
    }
}

pub async fn introspect_connection(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    axum::extract::Query(q): axum::extract::Query<IntrospectQuery>,
) -> Result<Json<Vec<FieldResponse>>, ApiError> {
    let url = {
        let store = lock_store(&state)?;
        store
            .connection(&id)
            .map(|c| c.url.clone())
            .ok_or_else(|| ApiError::bad_request("connection not found"))?
    };
    let query = q
        .query
        .ok_or_else(|| ApiError::bad_request("query parameter is required"))?;
    let src = toradb_pipe::SqlSource::open(&url, query, None, None)
        .await
        .map_err(ApiError::internal)?;
    let fields = toradb_pipe::Source::introspect(&src)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(
        fields
            .into_iter()
            .map(|f| FieldResponse {
                name: f.name,
                data_type: f.data_type,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct IntrospectQuery {
    query: Option<String>,
}

#[derive(Serialize)]
pub struct FieldResponse {
    name: String,
    data_type: String,
}

/// List the tables/views in a connection's database (for the table browser).
pub async fn list_connection_tables(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<String>>, ApiError> {
    let url = connection_url(&state, &id)?;
    let tables = toradb_pipe::list_tables(&url)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(tables))
}

/// Describe the columns of a specific table in a connection.
pub async fn list_connection_columns(
    State(state): State<AppState>,
    AxumPath((id, table)): AxumPath<(String, String)>,
) -> Result<Json<Vec<FieldResponse>>, ApiError> {
    let url = connection_url(&state, &id)?;
    let cols = toradb_pipe::list_columns(&url, &table)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(
        cols.into_iter()
            .map(|(name, data_type)| FieldResponse { name, data_type })
            .collect(),
    ))
}

fn connection_url(state: &AppState, id: &str) -> Result<String, ApiError> {
    let store = lock_store(state)?;
    store
        .connection(id)
        .map(|c| c.url.clone())
        .ok_or_else(|| ApiError::bad_request("connection not found"))
}

pub async fn list_pipelines(
    State(state): State<AppState>,
) -> Result<Json<Vec<PipelineResponse>>, ApiError> {
    let store = lock_store(&state)?;
    Ok(Json(
        store.pipelines().iter().map(pipeline_response).collect(),
    ))
}

pub async fn get_pipeline(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<PipelineResponse>, ApiError> {
    let store = lock_store(&state)?;
    store
        .pipeline(&id)
        .map(|p| Json(pipeline_response(p)))
        .ok_or_else(|| ApiError::bad_request("pipeline not found"))
}

pub async fn create_pipeline(
    State(state): State<AppState>,
    Json(body): Json<CreatePipelineBody>,
) -> Result<Json<PipelineResponse>, ApiError> {
    if body.name.trim().is_empty()
        || body.query.trim().is_empty()
        || body.target_table.trim().is_empty()
        || body.text_column.trim().is_empty()
    {
        return Err(ApiError::bad_request(
            "name, query, target_table, text_column are required",
        ));
    }
    let mode = parse_mode(body.mode.as_deref());
    if mode == SyncMode::Incremental && body.cursor_column.is_none() {
        return Err(ApiError::bad_request(
            "incremental mode requires cursor_column",
        ));
    }
    {
        let store = lock_store(&state)?;
        if store.connection(&body.connection_id).is_none() {
            return Err(ApiError::bad_request("connection not found"));
        }
    }
    let pipeline = Pipeline {
        id: String::new(),
        name: body.name,
        connection_id: body.connection_id,
        query: body.query,
        target_table: body.target_table,
        mapping: ColumnMapping {
            text_column: body.text_column,
            metadata_columns: body.metadata_columns,
            vector_column: body.vector_column,
            id_column: body.id_column,
            cursor_column: body.cursor_column,
        },
        embedder: body.embedder,
        mode,
        last_cursor: None,
        schedule: body
            .schedule_interval_secs
            .map(|interval_secs| toradb_pipe::Schedule { interval_secs }),
        drop_table_on_full: body.drop_table_on_full,
        enabled: body.enabled.unwrap_or(true),
        batch_size: body.batch_size.unwrap_or(10_000),
        created_at: 0,
    };
    let mut store = lock_store(&state)?;
    let created = store.add_pipeline(pipeline).map_err(ApiError::internal)?;
    Ok(Json(pipeline_response(&created)))
}

pub async fn patch_pipeline(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<PatchPipelineBody>,
) -> Result<Json<PipelineResponse>, ApiError> {
    let mut store = lock_store(&state)?;
    let updated = store
        .update_pipeline(&id, |p| {
            if let Some(e) = body.enabled {
                p.enabled = e;
            }
            if let Some(s) = body.schedule_interval_secs {
                p.schedule = s.map(|interval_secs| toradb_pipe::Schedule { interval_secs });
            }
            if let Some(q) = body.query {
                p.query = q;
            }
            if let Some(n) = body.name {
                p.name = n;
            }
        })
        .map_err(ApiError::bad_request)?;
    Ok(Json(pipeline_response(&updated)))
}

pub async fn delete_pipeline(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut store = lock_store(&state)?;
    let removed = store.delete_pipeline(&id).map_err(ApiError::internal)?;
    Ok(Json(serde_json::json!({ "deleted": removed })))
}

pub async fn pipeline_runs(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<Vec<PipelineRunResponse>>, ApiError> {
    let store = lock_store(&state)?;
    Ok(Json(
        store
            .runs_for(&id)
            .into_iter()
            .map(|r| PipelineRunResponse {
                id: r.id,
                pipeline_id: r.pipeline_id,
                state: r.state,
                phase: r.phase,
                rows_synced: r.rows_synced,
                started_at: r.started_at,
                finished_at: r.finished_at,
                message: r.message,
                cursor_after: r.cursor_after,
            })
            .collect(),
    ))
}

pub async fn run_pipeline_now(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SyncJobStarted>, ApiError> {
    {
        let store = lock_store(&state)?;
        if store.pipeline(&id).is_none() {
            return Err(ApiError::bad_request("pipeline not found"));
        }
    }
    let job_id = spawn_sync_job(&state, &id)
        .ok_or_else(|| ApiError::bad_request("pipeline is already running"))?;
    Ok(Json(SyncJobStarted { job_id }))
}

pub async fn list_sync_jobs(
    State(state): State<AppState>,
) -> Result<Json<Vec<IngestJob>>, ApiError> {
    let jobs = state
        .sync_jobs
        .lock()
        .map_err(|_| ApiError::internal("sync jobs lock poisoned"))?;
    Ok(Json(jobs.clone()))
}

pub async fn get_sync_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<u64>,
) -> Result<Json<IngestJob>, ApiError> {
    let jobs = state
        .sync_jobs
        .lock()
        .map_err(|_| ApiError::internal("sync jobs lock poisoned"))?;
    jobs.iter()
        .find(|j| j.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::bad_request("sync job not found"))
}

pub async fn cancel_sync_job(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<u64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if let Ok(mut set) = state.sync_cancel.lock() {
        set.insert(id);
    }
    Ok(Json(serde_json::json!({ "cancelled": 1 })))
}

#[derive(Deserialize)]
pub struct EmbedBody {
    text: String,
    embedder: toradb_pipe::EmbedderConfig,
}

#[derive(Serialize)]
pub struct EmbedResponse {
    vector: Vec<f32>,
}

pub async fn embed_query(Json(body): Json<EmbedBody>) -> Result<Json<EmbedResponse>, ApiError> {
    let vector = toradb_pipe::embed_query(&body.embedder, &body.text)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(EmbedResponse { vector }))
}

struct ApiSyncReporter {
    state: AppState,
    job_id: u64,
}

impl JobReporter for ApiSyncReporter {
    fn progress(&self, phase: &str, rows: u64, pct: Option<u8>) {
        update_sync_job(
            &self.state,
            self.job_id,
            "running",
            Some(phase.to_string()),
            None,
            pct,
            Some(rows),
        );
    }
    fn is_cancelled(&self) -> bool {
        self.state
            .sync_cancel
            .lock()
            .map(|s| s.contains(&self.job_id))
            .unwrap_or(false)
    }
}

fn update_sync_job(
    state: &AppState,
    id: u64,
    state_str: &str,
    phase: Option<String>,
    message: Option<String>,
    progress: Option<u8>,
    rows: Option<u64>,
) {
    if let Ok(mut jobs) = state.sync_jobs.lock() {
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
            if let Some(r) = rows {
                job.rows_ingested = r;
            }
            if matches!(state_str, "done" | "failed" | "cancelled") {
                job.finished_at_unix_secs = Some(now_secs());
            }
        }
    }
}

pub fn spawn_sync_job(state: &AppState, pipeline_id: &str) -> Option<u64> {
    let guard = RunGuard::claim(&state.scheduler_running, pipeline_id)?;

    let (pipeline, url) = {
        let store = state.pipe_store.lock().ok()?;
        let p = store.pipeline(pipeline_id)?.clone();
        let url = store.connection(&p.connection_id).map(|c| c.url.clone());
        (p, url)
    };

    let job_id = state.next_sync_job_id.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut jobs) = state.sync_jobs.lock() {
        jobs.push(IngestJob {
            id: job_id,
            table: pipeline.target_table.clone(),
            source: "pipe".to_string(),
            state: "running".to_string(),
            phase: Some("starting".to_string()),
            message: None,
            progress: Some(1),
            rows_ingested: 0,
            started_at_unix_secs: now_secs(),
            finished_at_unix_secs: None,
        });
        // Bound the in-memory list.
        let len = jobs.len();
        if len > 50 {
            jobs.drain(0..len - 50);
        }
    }

    let state2 = state.clone();
    let pid = pipeline_id.to_string();
    tokio::spawn(async move {
        // Move the guard into the task so it releases on completion (incl. panics).
        let _guard = guard;
        let run_id = {
            match state2.pipe_store.lock() {
                Ok(mut s) => Some(s.start_run(&pid, pipeline.last_cursor.clone())),
                Err(_) => None,
            }
        };

        let Some(url) = url else {
            update_sync_job(
                &state2,
                job_id,
                "failed",
                Some("failed".into()),
                Some("connection not found".into()),
                None,
                None,
            );
            return;
        };

        let reporter = Arc::new(ApiSyncReporter {
            state: state2.clone(),
            job_id,
        });

        let result = async {
            let source = open_sql_source(&url, &pipeline).await?;
            run_pipeline(state2.dag.clone(), source, &pipeline, reporter).await
        }
        .await;

        match result {
            Ok(outcome) => {
                // Persist watermark + finish the run record.
                if let Ok(mut s) = state2.pipe_store.lock() {
                    let _ = s.set_last_cursor(&pid, outcome.cursor_after.clone());
                    if let Some(rid) = run_id {
                        s.finish_run(
                            rid,
                            &outcome.state,
                            outcome.rows,
                            None,
                            outcome.cursor_after,
                        );
                    }
                }
                let final_state = if outcome.state == "cancelled" {
                    "cancelled"
                } else {
                    "done"
                };
                update_sync_job(
                    &state2,
                    job_id,
                    final_state,
                    Some(final_state.into()),
                    None,
                    Some(100),
                    Some(outcome.rows),
                );
            }
            Err(e) => {
                if let Ok(mut s) = state2.pipe_store.lock() {
                    if let Some(rid) = run_id {
                        s.finish_run(rid, "failed", 0, Some(e.clone()), None);
                    }
                }
                update_sync_job(
                    &state2,
                    job_id,
                    "failed",
                    Some("failed".into()),
                    Some(e),
                    None,
                    None,
                );
            }
        }
        // Clear any cancel flag for this job id.
        if let Ok(mut set) = state2.sync_cancel.lock() {
            set.remove(&job_id);
        }
    });

    Some(job_id)
}

const SESSION_COOKIE: &str = "toradb_session";

#[derive(Deserialize)]
pub struct LoginBody {
    name: String,
    password: String,
}

#[derive(Deserialize)]
pub struct BootstrapBody {
    name: String,
    password: String,
}

#[derive(Serialize)]
pub struct MeResponse {
    authenticated: bool,
    user: Option<String>,
    auth_enabled: bool,
}

#[derive(Deserialize)]
pub struct CreateApiKeyBody {
    name: String,
}

fn auth_enabled(state: &AppState) -> bool {
    state.auth_enabled.load(Ordering::SeqCst)
}

pub async fn auth_bootstrap(
    State(state): State<AppState>,
    Json(body): Json<BootstrapBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut auth = state
        .auth
        .lock()
        .map_err(|_| ApiError::internal("auth lock poisoned"))?;
    if auth.has_users() {
        return Err(ApiError::bad_request("an admin already exists"));
    }
    auth.create_user(&body.name, &body.password)
        .map_err(ApiError::bad_request)?;
    state.auth_enabled.store(true, Ordering::SeqCst);
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn auth_login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Response, ApiError> {
    let token = {
        let auth = state
            .auth
            .lock()
            .map_err(|_| ApiError::internal("auth lock poisoned"))?;
        auth.login(&body.name, &body.password)
    };
    match token {
        Some(token) => {
            let cookie =
                format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800");
            let mut resp = Json(serde_json::json!({ "ok": true })).into_response();
            resp.headers_mut().insert(
                header::SET_COOKIE,
                cookie
                    .parse()
                    .map_err(|_| ApiError::internal("bad cookie"))?,
            );
            Ok(resp)
        }
        None => Err(ApiError::bad_request("invalid credentials")),
    }
}

pub async fn auth_logout() -> Result<Response, ApiError> {
    let cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; Max-Age=0");
    let mut resp = Json(serde_json::json!({ "ok": true })).into_response();
    resp.headers_mut().insert(
        header::SET_COOKIE,
        cookie
            .parse()
            .map_err(|_| ApiError::internal("bad cookie"))?,
    );
    Ok(resp)
}

pub async fn auth_me(State(state): State<AppState>) -> Result<Json<MeResponse>, ApiError> {
    // `auth_me` is reachable without a guard; report status only.
    let enabled = auth_enabled(&state);
    Ok(Json(MeResponse {
        authenticated: !enabled,
        user: None,
        auth_enabled: enabled,
    }))
}

pub async fn list_api_keys(
    State(_state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // Keys are write-only (hashed); we don't list raw values.
    Ok(Json(
        serde_json::json!({ "note": "API keys are shown once at creation" }),
    ))
}

pub async fn create_api_key(
    State(state): State<AppState>,
    Json(body): Json<CreateApiKeyBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let mut auth = state
        .auth
        .lock()
        .map_err(|_| ApiError::internal("auth lock poisoned"))?;
    let raw = auth
        .create_api_key(&body.name)
        .map_err(ApiError::internal)?;
    Ok(Json(serde_json::json!({ "key": raw })))
}

fn cookie_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|kv| {
        let kv = kv.trim();
        kv.strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('='))
    })
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if !auth_enabled(&state) {
        return next.run(request).await;
    }
    let path = request.uri().path();
    let open = path == "/api/health"
        || path == "/api/auth/login"
        || path == "/api/auth/bootstrap"
        || path == "/api/auth/me"
        || !path.starts_with("/api/");
    if open {
        return next.run(request).await;
    }

    let headers = request.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let session = cookie_value(headers, SESSION_COOKIE);

    let ok = {
        match state.auth.lock() {
            Ok(auth) => {
                bearer.map(|k| auth.verify_api_key(k)).unwrap_or(false)
                    || session.and_then(|t| auth.validate_session(t)).is_some()
            }
            Err(_) => false,
        }
    };

    if ok {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response()
    }
}

pub fn spawn_pipe_scheduler(state: AppState) {
    let store = state.pipe_store.clone();
    let running = state.scheduler_running.clone();
    let sched_state = state.clone();
    toradb_pipe::spawn_scheduler(store, running, move |pipeline_id| {
        spawn_sync_job(&sched_state, &pipeline_id);
    });
}
