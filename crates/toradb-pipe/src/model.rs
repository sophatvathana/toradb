//! Persisted data model for toraPipe: connections, pipelines, and run history.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Sql,
    ObjectStore,
    Http,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub created_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    Full,
    Incremental,
    Cdc,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub interval_secs: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ColumnMapping {
    pub text_column: String,
    #[serde(default)]
    pub metadata_columns: Vec<String>,
    #[serde(default)]
    pub vector_column: Option<String>,
    #[serde(default)]
    pub id_column: Option<String>,
    #[serde(default)]
    pub cursor_column: Option<String>,
}

impl ColumnMapping {
    pub fn wants_metadata(&self, name: &str) -> bool {
        if name == self.text_column {
            return false;
        }
        if self.metadata_columns.is_empty()
            || self.metadata_columns.iter().any(|c| c == "*")
        {
            return true;
        }
        self.metadata_columns.iter().any(|c| c == name)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    pub connection_id: String,
    pub query: String,
    pub target_table: String,
    pub mapping: ColumnMapping,
    #[serde(default)]
    pub embedder: Option<crate::embed::EmbedderConfig>,
    pub mode: SyncMode,
    #[serde(default)]
    pub last_cursor: Option<String>,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    #[serde(default)]
    pub drop_table_on_full: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    pub created_at: u64,
}

fn default_true() -> bool {
    true
}
fn default_batch_size() -> usize {
    10_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: u64,
    pub pipeline_id: String,
    /// `running` | `done` | `failed` | `cancelled`.
    pub state: String,
    #[serde(default)]
    pub phase: Option<String>,
    pub rows_synced: u64,
    pub started_at: u64,
    #[serde(default)]
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub cursor_before: Option<String>,
    #[serde(default)]
    pub cursor_after: Option<String>,
}
