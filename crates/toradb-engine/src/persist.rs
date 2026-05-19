use std::path::{Path, PathBuf};

use memmap2::MmapOptions;
use toradb_index::sparse::bm25_codec;
use toradb_index::{Bm25Snapshot, CorpusStore, IngestDoc};
use toradb_storage::columnar::{
    read_segment, write_segment, ColumnarDoc, TableManifestFile,
};

pub const DEFAULT_SEGMENT_PARALLELISM: u32 = 4;

fn num_segments_hint(segments_len: usize) -> u32 {
    segments_len.max(DEFAULT_SEGMENT_PARALLELISM as usize) as u32
}

/// Segment-parallel fan-out for a table (manifest segment count, minimum 4).
pub fn table_segment_count(base: &Path, table: &str) -> Result<u32, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(DEFAULT_SEGMENT_PARALLELISM);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    Ok(num_segments_hint(manifest.segments.len()))
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

fn bm25_table_bin_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("bm25.bin")
}

fn bm25_table_json_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("bm25.json")
}

fn segment_bm25_bin_path(base: &Path, table: &str, segment_parquet: &str) -> PathBuf {
    let stem = segment_parquet
        .strip_suffix(".parquet")
        .unwrap_or(segment_parquet);
    indexes_dir(base, table).join(format!("{stem}.bm25.bin"))
}

fn segment_bm25_json_path(base: &Path, table: &str, segment_parquet: &str) -> PathBuf {
    let stem = segment_parquet
        .strip_suffix(".parquet")
        .unwrap_or(segment_parquet);
    indexes_dir(base, table).join(format!("{stem}.bm25.json"))
}

fn load_snapshot_mmap(path: &Path) -> Result<Bm25Snapshot, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mmap = unsafe { MmapOptions::new().map(&file).map_err(|e| e.to_string())? };
    bm25_codec::decode_snapshot(&mmap)
}

fn load_snapshot_json(path: &Path) -> Result<Bm25Snapshot, String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

pub fn save_bm25_sidecar(base: &Path, table: &str, snap: &Bm25Snapshot) -> Result<(), String> {
    bm25_codec::write_snapshot_file(&bm25_table_bin_path(base, table), snap)
}

pub fn save_segment_bm25_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
    snap: &Bm25Snapshot,
) -> Result<(), String> {
    bm25_codec::write_snapshot_file(
        &segment_bm25_bin_path(base, table, segment_parquet),
        snap,
    )
}

pub fn load_bm25_sidecar(base: &Path, table: &str) -> Result<Option<Bm25Snapshot>, String> {
    let bin = bm25_table_bin_path(base, table);
    if bin.exists() {
        return load_snapshot_mmap(&bin).map(Some);
    }
    let json = bm25_table_json_path(base, table);
    if json.exists() {
        return load_snapshot_json(&json).map(Some);
    }
    Ok(None)
}

fn load_segment_bm25_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
) -> Result<Option<Bm25Snapshot>, String> {
    let bin = segment_bm25_bin_path(base, table, segment_parquet);
    if bin.exists() {
        return load_snapshot_mmap(&bin).map(Some);
    }
    let json = segment_bm25_json_path(base, table, segment_parquet);
    if json.exists() {
        return load_snapshot_json(&json).map(Some);
    }
    Ok(None)
}

/// True when at least one on-disk segment has a BM25 sidecar (bin or json).
pub fn table_has_segment_bm25_sidecars(base: &Path, table: &str) -> Result<bool, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(false);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    for seg in &manifest.segments {
        if segment_bm25_bin_path(base, table, seg).exists()
            || segment_bm25_json_path(base, table, seg).exists()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Merge all per-segment BM25 sidecars under `indexes/`.
pub fn load_merged_segment_bm25_sidecars(
    base: &Path,
    table: &str,
) -> Result<Option<Bm25Snapshot>, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let mut merged: Option<Bm25Snapshot> = None;
    for seg in &manifest.segments {
        if let Some(snap) = load_segment_bm25_sidecar(base, table, seg)? {
            match merged {
                None => merged = Some(snap),
                Some(ref mut acc) => acc.merge(snap),
            }
        }
    }
    Ok(merged)
}

fn snapshot_for_columnar_docs(docs: &[ColumnarDoc]) -> Bm25Snapshot {
    Bm25Snapshot::from_documents(docs.iter().map(|d| (d.id, d.text.as_str())))
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

/// Table names under a database path (directories with a manifest.json).
pub fn list_tables(base: &Path) -> Result<Vec<String>, String> {
    if !base.exists() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in std::fs::read_dir(base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if TableManifestFile::path_for_table(base, &name).exists() {
            names.push(name);
        }
    }
    names.sort_unstable();
    Ok(names)
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

fn restore_bm25_index(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
) -> Result<(), String> {
    if let Some(snap) = load_merged_segment_bm25_sidecars(base, table)? {
        store.restore_bm25(table, snap);
    } else if let Some(snap) = load_bm25_sidecar(base, table)? {
        store.restore_bm25(table, snap);
    } else {
        store.rebuild_bm25(table);
        if let Some(snap) = store.bm25_snapshot(table) {
            save_bm25_sidecar(base, table, &snap)?;
        }
    }
    store.rebuild_hnsw(table);
    Ok(())
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
        restore_bm25_index(base, table, store)?;
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
    let seg_snap = snapshot_for_columnar_docs(docs);
    save_segment_bm25_sidecar(base, table, &seg_name, &seg_snap)?;
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
