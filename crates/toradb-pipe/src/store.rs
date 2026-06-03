use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::model::{Connection, Pipeline, PipelineRun, SourceKind};
use crate::secret::SecretBox;

const MAX_RUNS_PER_PIPELINE: usize = 200;

pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct PipeStore {
    root: PathBuf,
    connections: Vec<Connection>,
    pipelines: Vec<Pipeline>,
    runs: Vec<PipelineRun>,
    next_run_id: u64,
    next_seq: u64,
    secret: SecretBox,
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            format!("toradb-pipe: corrupt {}: {e}", path.display())
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.to_string()),
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

impl PipeStore {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let root = db_path.join(".torapipe");
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        let secret = SecretBox::open(db_path)?;
        let mut connections: Vec<Connection> = read_json(&root.join("connections.json"))?;
        for c in &mut connections {
            c.url = secret.decrypt(&c.url)?;
        }
        let pipelines: Vec<Pipeline> = read_json(&root.join("pipelines.json"))?;
        let runs: Vec<PipelineRun> = read_json(&root.join("runs.json"))?;
        let next_run_id = runs.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        Ok(Self {
            root,
            connections,
            pipelines,
            runs,
            next_run_id,
            next_seq: 0,
            secret,
        })
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.next_seq += 1;
        format!("{prefix}_{}_{}", now_unix_secs(), self.next_seq)
    }

    fn save_connections(&self) -> Result<(), String> {
        let mut on_disk = self.connections.clone();
        for c in &mut on_disk {
            c.url = self.secret.encrypt(&c.url)?;
        }
        write_json_atomic(&self.root.join("connections.json"), &on_disk)
    }
    fn save_pipelines(&self) -> Result<(), String> {
        write_json_atomic(&self.root.join("pipelines.json"), &self.pipelines)
    }
    fn save_runs(&self) -> Result<(), String> {
        write_json_atomic(&self.root.join("runs.json"), &self.runs)
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    pub fn connection(&self, id: &str) -> Option<&Connection> {
        self.connections.iter().find(|c| c.id == id)
    }

    pub fn add_connection(
        &mut self,
        name: String,
        kind: SourceKind,
        url: String,
    ) -> Result<Connection, String> {
        let conn = Connection {
            id: self.next_id("conn"),
            name,
            kind,
            url,
            created_at: now_unix_secs(),
        };
        self.connections.push(conn.clone());
        self.save_connections()?;
        Ok(conn)
    }

    pub fn delete_connection(&mut self, id: &str) -> Result<bool, String> {
        let before = self.connections.len();
        self.connections.retain(|c| c.id != id);
        if self.connections.len() == before {
            return Ok(false);
        }
        self.save_connections()?;
        Ok(true)
    }

    pub fn pipelines(&self) -> &[Pipeline] {
        &self.pipelines
    }

    pub fn pipeline(&self, id: &str) -> Option<&Pipeline> {
        self.pipelines.iter().find(|p| p.id == id)
    }

    pub fn add_pipeline(&mut self, mut pipeline: Pipeline) -> Result<Pipeline, String> {
        if pipeline.id.is_empty() {
            pipeline.id = self.next_id("pipe");
        }
        if pipeline.created_at == 0 {
            pipeline.created_at = now_unix_secs();
        }
        self.pipelines.push(pipeline.clone());
        self.save_pipelines()?;
        Ok(pipeline)
    }

    pub fn update_pipeline(
        &mut self,
        id: &str,
        f: impl FnOnce(&mut Pipeline),
    ) -> Result<Pipeline, String> {
        let p = self
            .pipelines
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("pipeline {id} not found"))?;
        f(p);
        let updated = p.clone();
        self.save_pipelines()?;
        Ok(updated)
    }

    pub fn set_last_cursor(&mut self, id: &str, cursor: Option<String>) -> Result<(), String> {
        self.update_pipeline(id, |p| p.last_cursor = cursor)?;
        Ok(())
    }

    pub fn delete_pipeline(&mut self, id: &str) -> Result<bool, String> {
        let before = self.pipelines.len();
        self.pipelines.retain(|p| p.id != id);
        if self.pipelines.len() == before {
            return Ok(false);
        }
        self.runs.retain(|r| r.pipeline_id != id);
        self.save_pipelines()?;
        self.save_runs()?;
        Ok(true)
    }

    pub fn enabled_scheduled(&self) -> Vec<Pipeline> {
        self.pipelines
            .iter()
            .filter(|p| p.enabled && p.schedule.is_some())
            .cloned()
            .collect()
    }

    pub fn start_run(&mut self, pipeline_id: &str, cursor_before: Option<String>) -> u64 {
        let id = self.next_run_id;
        self.next_run_id += 1;
        self.runs.push(PipelineRun {
            id,
            pipeline_id: pipeline_id.to_string(),
            state: "running".into(),
            phase: Some("starting".into()),
            rows_synced: 0,
            started_at: now_unix_secs(),
            finished_at: None,
            message: None,
            cursor_before,
            cursor_after: None,
        });
        self.cap_runs(pipeline_id);
        let _ = self.save_runs();
        id
    }

    pub fn finish_run(
        &mut self,
        run_id: u64,
        state: &str,
        rows: u64,
        message: Option<String>,
        cursor_after: Option<String>,
    ) {
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == run_id) {
            r.state = state.to_string();
            r.phase = None;
            r.rows_synced = rows;
            r.finished_at = Some(now_unix_secs());
            r.message = message;
            r.cursor_after = cursor_after;
        }
        let _ = self.save_runs();
    }

    pub fn last_run_started(&self, pipeline_id: &str) -> u64 {
        self.runs
            .iter()
            .filter(|r| r.pipeline_id == pipeline_id)
            .map(|r| r.started_at)
            .max()
            .unwrap_or(0)
    }

    pub fn runs_for(&self, pipeline_id: &str) -> Vec<PipelineRun> {
        let mut v: Vec<PipelineRun> = self
            .runs
            .iter()
            .filter(|r| r.pipeline_id == pipeline_id)
            .cloned()
            .collect();
        v.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        v
    }

    fn cap_runs(&mut self, pipeline_id: &str) {
        let count = self.runs.iter().filter(|r| r.pipeline_id == pipeline_id).count();
        if count <= MAX_RUNS_PER_PIPELINE {
            return;
        }
        let mut seen = 0usize;
        let drop_below = count - MAX_RUNS_PER_PIPELINE;
        let mut ids_to_drop = Vec::new();
        let mut ordered: Vec<&PipelineRun> = self
            .runs
            .iter()
            .filter(|r| r.pipeline_id == pipeline_id)
            .collect();
        ordered.sort_by_key(|r| r.started_at);
        for r in ordered {
            if seen >= drop_below {
                break;
            }
            ids_to_drop.push(r.id);
            seen += 1;
        }
        self.runs.retain(|r| !ids_to_drop.contains(&r.id));
    }
}
