use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerNode {
    pub id: String,
    /// TCP listen address for this worker's RPC server.
    pub addr: String,
    /// On-disk database root (must contain table segments for assigned shards).
    pub db_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Coordinator database root (manifest + routing metadata).
    pub coordinator_db: PathBuf,
    pub workers: Vec<WorkerNode>,
}

impl ClusterConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("json") => serde_json::from_str(&text).map_err(|e| e.to_string()),
            Some("yaml") | Some("yml") => serde_yaml::from_str(&text).map_err(|e| e.to_string()),
            _ => serde_yaml::from_str(&text)
                .or_else(|_| serde_json::from_str(&text))
                .map_err(|e| format!("cluster config parse failed (expected YAML or JSON): {e}")),
        }
    }

    pub fn from_env() -> Option<Self> {
        let path = std::env::var("TORADB_CLUSTER_CONFIG").ok()?;
        Self::load(path).ok()
    }

    /// Worker responsible for segment `seg` when using hash sharding.
    pub fn worker_for_segment(&self, seg: u32) -> Option<&WorkerNode> {
        if self.workers.is_empty() {
            return None;
        }
        let idx = (seg as usize) % self.workers.len();
        Some(&self.workers[idx])
    }
}
