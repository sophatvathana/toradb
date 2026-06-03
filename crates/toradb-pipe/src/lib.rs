//! toraPipe — sync data from external sources into ToraDB tables.

pub mod auth;
pub mod embed;
pub mod model;
pub mod pipeline;
pub mod scheduler;
pub mod secret;
pub mod source;
pub mod store;

pub use auth::AuthStore;
pub use embed::{build_embedder, Embedder, EmbedderConfig};
pub use model::{ColumnMapping, Connection, Pipeline, PipelineRun, Schedule, SourceKind, SyncMode};
pub use pipeline::{run_pipeline, JobReporter, NullReporter, RunOutcome};
pub use scheduler::{spawn_scheduler, RunGuard, RunningSet};
pub use secret::SecretBox;
pub use source::sql::{
    install_drivers, list_columns, list_tables, test_connection, validate_sqlite, SqlSource,
};
pub use source::{Cursor, FieldInfo, Source};
pub use store::PipeStore;

pub async fn embed_query(config: &EmbedderConfig, text: &str) -> Result<Vec<f32>, String> {
    let embedder = build_embedder(config)?;
    let mut out = embedder.embed(&[text.to_string()]).await?;
    out.pop()
        .ok_or_else(|| "embedder returned no vector".to_string())
}

pub async fn open_sql_source(url: &str, pipeline: &Pipeline) -> Result<Box<dyn Source>, String> {
    let src = SqlSource::open(
        url,
        pipeline.query.clone(),
        pipeline.mapping.cursor_column.clone(),
        pipeline.mapping.id_column.clone(),
    )
    .await?;
    Ok(Box::new(src))
}
