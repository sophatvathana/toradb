use std::path::{Path, PathBuf};

use toradb_index::{Bm25Snapshot, CorpusStore, IngestDoc};
use toradb_storage::columnar::{
    read_segment, write_segment, ColumnarDoc, TableManifestFile,
};

fn num_segments_hint(segments_len: usize) -> u32 {
    (segments_len.max(4)) as u32
}

fn ingest_to_columnar(id: u64, doc: &IngestDoc) -> ColumnarDoc {
    ColumnarDoc {
        id,
        text: doc.text.clone(),
        metadata: doc.metadata.clone(),
        embedding: doc.vector.clone(),
    }
}

fn indexes_dir(base: &Path, table: &str) -> PathBuf {
    base.join(table).join("indexes")
}

fn bm25_sidecar_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("bm25.json")
}

pub fn save_bm25_sidecar(base: &Path, table: &str, snap: &Bm25Snapshot) -> Result<(), String> {
    let path = bm25_sidecar_path(base, table);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let data = serde_json::to_string(snap).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_bm25_sidecar(base: &Path, table: &str) -> Result<Option<Bm25Snapshot>, String> {
    let path = bm25_sidecar_path(base, table);
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string()).map(Some)
}

/// Read all documents for a table from on-disk Parquet segments.
pub fn read_table_documents(base: &Path, table: &str) -> Result<Vec<(u64, IngestDoc)>, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let mut out = Vec::new();
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if !path.exists() {
            continue;
        }
        for doc in read_segment(&path)? {
            out.push((
                doc.id,
                IngestDoc {
                    text: doc.text,
                    metadata: doc.metadata,
                    vector: doc.embedding,
                },
            ));
        }
    }
    Ok(out)
}

/// In-memory corpus first; fall back to columnar Parquet scan when empty.
pub fn table_documents(
    store: &CorpusStore,
    base: Option<&Path>,
    table: &str,
) -> Result<Vec<(u64, IngestDoc)>, String> {
    let mem = store.all_documents(table);
    if !mem.is_empty() {
        return Ok(mem);
    }
    if let Some(base) = base {
        return read_table_documents(base, table);
    }
    Ok(Vec::new())
}

pub fn load_all(base: &Path, store: &mut CorpusStore, segment_count: usize) -> Result<usize, String> {
    if !base.exists() {
        return Ok(0);
    }
    let mut total = 0usize;
    for entry in std::fs::read_dir(base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        total += load_table(base, &name, store, segment_count)?;
    }
    Ok(total)
}

pub fn load_table(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    segment_count: usize,
) -> Result<usize, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(0);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let num_segments = num_segments_hint(segment_count.max(manifest.segments.len()));
    let mut n = 0usize;
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if !path.exists() {
            continue;
        }
        for doc in read_segment(&path)? {
            store.insert_stored(
                table,
                doc.id,
                IngestDoc {
                    text: doc.text,
                    metadata: doc.metadata,
                    vector: doc.embedding,
                },
                num_segments,
            );
            n += 1;
        }
    }
    if n > 0 {
        if let Some(snap) = load_bm25_sidecar(base, table)? {
            store.restore_bm25(table, snap);
        } else {
            store.rebuild_bm25(table);
            if let Some(snap) = store.bm25_snapshot(table) {
                save_bm25_sidecar(base, table, &snap)?;
            }
        }
    }
    Ok(n)
}

pub fn flush_batch(base: &Path, table: &str, docs: &[ColumnarDoc]) -> Result<(), String> {
    if docs.is_empty() {
        return Ok(());
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    let mut manifest = if manifest_path.exists() {
        TableManifestFile::load(&manifest_path)?
    } else {
        TableManifestFile::default()
    };
    let seg_name = format!("seg_{:05}.parquet", manifest.segments.len() + 1);
    let seg_path = TableManifestFile::segments_dir(base, table).join(&seg_name);
    write_segment(&seg_path, docs)?;
    manifest.push_segment(seg_name);
    manifest.save(&manifest_path)?;
    Ok(())
}

pub fn flush_new_docs(
    base: &Path,
    table: &str,
    store: &CorpusStore,
    since_id: u64,
) -> Result<(), String> {
    let columnar: Vec<ColumnarDoc> = store
        .docs_with_ids_since(table, since_id)
        .into_iter()
        .map(|(id, doc)| ingest_to_columnar(id, &doc))
        .collect();
    flush_batch(base, table, &columnar)?;
    if let Some(snap) = store.bm25_snapshot(table) {
        save_bm25_sidecar(base, table, &snap)?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DbPath(pub PathBuf);

impl DbPath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}
