use std::sync::{Arc, Mutex};

use toradb_engine::DagRunner;

use crate::model::{Pipeline, SyncMode};
use crate::source::{row_to_doc, Batch, Cursor, Source};

pub trait JobReporter: Send + Sync {
    fn progress(&self, phase: &str, rows: u64, pct: Option<u8>);
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub struct NullReporter;
impl JobReporter for NullReporter {
    fn progress(&self, _phase: &str, _rows: u64, _pct: Option<u8>) {}
}

pub struct RunOutcome {
    pub rows: u64,
    pub cursor_after: Option<String>,
    pub state: String,
}

pub async fn run_pipeline(
    dag: Arc<Mutex<DagRunner>>,
    mut source: Box<dyn Source>,
    pipeline: &Pipeline,
    reporter: Arc<dyn JobReporter>,
) -> Result<RunOutcome, String> {
    let table = pipeline.target_table.clone();
    reporter.progress("preparing", 0, Some(1));

    {
        let mut d = dag.lock().map_err(|_| "dag lock poisoned".to_string())?;
        if pipeline.mode == SyncMode::Full && pipeline.drop_table_on_full {
            let _ = d.drop_table(&table);
        }
        d.ensure_table(&table);
        if !d.bulk_ingest_active(&table) {
            d.begin_bulk_ingest(&table);
        }
    }

    let mut cursor = match (pipeline.mode, &pipeline.last_cursor) {
        (SyncMode::Incremental | SyncMode::Cdc, Some(v)) => Cursor::Value(v.clone()),
        (SyncMode::Incremental | SyncMode::Cdc, None) => Cursor::Start,
        _ => Cursor::Start,
    };
    let mut total: u64 = 0;
    let mut max_cursor: Option<String> = pipeline.last_cursor.clone();
    let batch_size = pipeline.batch_size.max(1);

    // Build the embedder once, if configured.
    let embedder = match &pipeline.embedder {
        Some(cfg) => Some(crate::embed::build_embedder(cfg)?),
        None => None,
    };

    loop {
        if reporter.is_cancelled() {
            return finish_session(&dag, &table, total, false).map(|_| RunOutcome {
                rows: total,
                cursor_after: max_cursor.clone(),
                state: "cancelled".into(),
            });
        }

        let Batch { rows, next } = source.fetch_batch(&cursor, batch_size).await?;
        if rows.is_empty() {
            break;
        }
        let fetched = rows.len();

        let mut docs = Vec::with_capacity(rows.len());
        for r in &rows {
            let (doc, cur) = row_to_doc(r, &pipeline.mapping);
            if let Some(c) = cur {
                max_cursor = Some(match max_cursor.take() {
                    Some(prev) if prev >= c => prev,
                    _ => c,
                });
            }
            docs.push(doc);
        }

        if let Some(emb) = &embedder {
            let idxs: Vec<usize> = docs
                .iter()
                .enumerate()
                .filter(|(_, d)| d.vector.is_none() && !d.text.is_empty())
                .map(|(i, _)| i)
                .collect();
            if !idxs.is_empty() {
                let texts: Vec<String> = idxs.iter().map(|&i| docs[i].text.clone()).collect();
                let vectors = emb.embed(&texts).await?;
                for (slot, vec) in idxs.into_iter().zip(vectors.into_iter()) {
                    docs[slot].vector = Some(vec);
                }
            }
        }

        {
            let mut d = dag.lock().map_err(|_| "dag lock poisoned".to_string())?;
            d.add_documents(&table, docs)?;
        }

        total += fetched as u64;
        cursor = next;
        reporter.progress("syncing", total, None);

        if fetched < batch_size {
            break;
        }
    }

    let compact = pipeline.mode == SyncMode::Full && total > 0;
    finish_session(&dag, &table, total, compact)?;

    reporter.progress("done", total, Some(100));
    Ok(RunOutcome {
        rows: total,
        cursor_after: max_cursor,
        state: "done".into(),
    })
}

fn finish_session(
    dag: &Arc<Mutex<DagRunner>>,
    table: &str,
    _total: u64,
    compact: bool,
) -> Result<(), String> {
    let mut d = dag.lock().map_err(|_| "dag lock poisoned".to_string())?;
    if d.bulk_ingest_active(table) {
        d.finish_bulk_ingest(table, compact)?;
    }
    Ok(())
}
