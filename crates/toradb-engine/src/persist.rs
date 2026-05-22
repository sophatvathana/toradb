use std::collections::HashMap;
use std::path::{Path, PathBuf};

use memmap2::MmapOptions;
use toradb_index::dense::{diskann_codec, hnsw_codec, vector_codec};
use toradb_index::sparse::bm25_codec;
use toradb_index::{Bm25Snapshot, CorpusStore, IngestDoc, VectorSnapshot};
use toradb_storage::columnar::{
    read_segment, write_segment, ColumnarDoc, TableManifestFile,
};
use toradb_storage::wal;

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

fn table_vectors_bin_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("vectors.bin")
}

fn table_hnsw_bin_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("hnsw.bin")
}

fn table_diskann_bin_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("diskann.bin")
}

fn segment_hnsw_shard_path(base: &Path, table: &str, segment: u32) -> PathBuf {
    indexes_dir(base, table).join(format!("shard_{segment:02}.hnsw.bin"))
}

fn segment_vectors_bin_path(base: &Path, table: &str, segment_parquet: &str) -> PathBuf {
    let stem = segment_parquet
        .strip_suffix(".parquet")
        .unwrap_or(segment_parquet);
    indexes_dir(base, table).join(format!("{stem}.vectors.bin"))
}

fn load_snapshot_mmap(path: &Path) -> Result<Bm25Snapshot, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mmap = unsafe { MmapOptions::new().map(&file).map_err(|e| e.to_string())? };
    bm25_codec::decode_snapshot(&mmap)
}

fn load_vector_snapshot_mmap(path: &Path) -> Result<VectorSnapshot, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mmap = unsafe { MmapOptions::new().map(&file).map_err(|e| e.to_string())? };
    vector_codec::decode_snapshot(&mmap)
}

fn load_hnsw_index_mmap(path: &Path) -> Result<toradb_index::dense::hnsw_index::HnswIndex, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mmap = unsafe { MmapOptions::new().map(&file).map_err(|e| e.to_string())? };
    hnsw_codec::decode_index(&mmap)
}

pub fn save_table_hnsw_sidecar(
    base: &Path,
    table: &str,
    index: &toradb_index::dense::hnsw_index::HnswIndex,
) -> Result<(), String> {
    hnsw_codec::write_index_file(&table_hnsw_bin_path(base, table), index)
}

pub fn load_table_hnsw_sidecar(
    base: &Path,
    table: &str,
) -> Result<Option<toradb_index::dense::hnsw_index::HnswIndex>, String> {
    let bin = table_hnsw_bin_path(base, table);
    if bin.exists() {
        return load_hnsw_index_mmap(&bin).map(Some);
    }
    Ok(None)
}

fn load_diskann_index_mmap(path: &Path) -> Result<toradb_index::dense::hnsw_index::HnswIndex, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mmap = unsafe { MmapOptions::new().map(&file).map_err(|e| e.to_string())? };
    diskann_codec::decode_index(&mmap)
}

pub fn save_table_diskann_sidecar(
    base: &Path,
    table: &str,
    index: &toradb_index::dense::hnsw_index::HnswIndex,
) -> Result<(), String> {
    diskann_codec::write_index_file(&table_diskann_bin_path(base, table), index)
}

pub fn load_table_diskann_sidecar(
    base: &Path,
    table: &str,
) -> Result<Option<toradb_index::dense::hnsw_index::HnswIndex>, String> {
    let bin = table_diskann_bin_path(base, table);
    if bin.exists() {
        return load_diskann_index_mmap(&bin).map(Some);
    }
    Ok(None)
}

pub fn save_segment_hnsw_shards(
    base: &Path,
    table: &str,
    shards: &std::collections::HashMap<u32, toradb_index::dense::hnsw_index::HnswIndex>,
) -> Result<(), String> {
    for (seg, index) in shards {
        hnsw_codec::write_index_file(&segment_hnsw_shard_path(base, table, *seg), index)?;
    }
    Ok(())
}

pub fn load_segment_hnsw_shards(
    base: &Path,
    table: &str,
    num_segments: u32,
) -> Result<std::collections::HashMap<u32, toradb_index::dense::hnsw_index::HnswIndex>, String> {
    let mut out = std::collections::HashMap::new();
    for seg in 0..num_segments {
        let path = segment_hnsw_shard_path(base, table, seg);
        if path.exists() {
            out.insert(seg, load_hnsw_index_mmap(&path)?);
        }
    }
    Ok(out)
}

/// True when at least one per-segment HNSW shard exists on disk.
pub fn table_has_segment_hnsw_sidecars(base: &Path, table: &str) -> Result<bool, String> {
    let dir = indexes_dir(base, table);
    if !dir.exists() {
        return Ok(false);
    }
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("shard_") && name.ends_with(".hnsw.bin") {
            return Ok(true);
        }
    }
    Ok(false)
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

pub fn save_table_vector_sidecar(
    base: &Path,
    table: &str,
    snap: &VectorSnapshot,
) -> Result<(), String> {
    vector_codec::write_snapshot_file(&table_vectors_bin_path(base, table), snap)
}

pub fn save_segment_vector_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
    snap: &VectorSnapshot,
) -> Result<(), String> {
    vector_codec::write_snapshot_file(
        &segment_vectors_bin_path(base, table, segment_parquet),
        snap,
    )
}

pub fn load_table_vector_sidecar(base: &Path, table: &str) -> Result<Option<VectorSnapshot>, String> {
    let bin = table_vectors_bin_path(base, table);
    if bin.exists() {
        return load_vector_snapshot_mmap(&bin).map(Some);
    }
    Ok(None)
}

fn load_segment_vector_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
) -> Result<Option<VectorSnapshot>, String> {
    let bin = segment_vectors_bin_path(base, table, segment_parquet);
    if bin.exists() {
        return load_vector_snapshot_mmap(&bin).map(Some);
    }
    Ok(None)
}

/// True when at least one on-disk segment has a vector sidecar.
pub fn table_has_segment_vector_sidecars(base: &Path, table: &str) -> Result<bool, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(false);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    for seg in &manifest.segments {
        if segment_vectors_bin_path(base, table, seg).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_merged_segment_vector_map(
    base: &Path,
    table: &str,
) -> Result<HashMap<u64, Vec<f32>>, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(HashMap::new());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let mut merged = HashMap::new();
    for seg in &manifest.segments {
        if let Some(snap) = load_segment_vector_sidecar(base, table, seg)? {
            merged.extend(snap.to_map());
        }
    }
    Ok(merged)
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

fn snapshot_for_columnar_vectors(docs: &[ColumnarDoc]) -> Option<VectorSnapshot> {
    let mut pairs = Vec::new();
    let mut dim = None;
    for doc in docs {
        let Some(emb) = doc.embedding.as_ref() else {
            continue;
        };
        let d = *dim.get_or_insert(emb.len());
        if d != emb.len() {
            return None;
        }
        pairs.push((doc.id, emb.clone()));
    }
    let dim = dim?;
    VectorSnapshot::from_pairs(dim as u32, &pairs).ok()
}

fn vector_snapshot_from_store(store: &CorpusStore, table: &str) -> Option<VectorSnapshot> {
    let mut pairs = Vec::new();
    let mut dim = None;
    for (id, doc) in store.all_documents(table) {
        let Some(emb) = doc.vector else {
            continue;
        };
        let d = *dim.get_or_insert(emb.len());
        if d != emb.len() {
            return None;
        }
        pairs.push((id, emb));
    }
    let dim = dim?;
    VectorSnapshot::from_pairs(dim as u32, &pairs).ok()
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

/// Remove a table directory from disk.
pub fn drop_table(base: &Path, table: &str) -> Result<(), String> {
    let dir = base.join(table);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
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
    num_segments: u32,
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
    restore_hnsw_index(base, table, store, num_segments)?;
    restore_diskann_index(base, table, store)?;
    Ok(())
}

fn restore_diskann_index(base: &Path, table: &str, store: &mut CorpusStore) -> Result<(), String> {
    if let Some(index) = load_table_diskann_sidecar(base, table)? {
        store.restore_diskann(table, index);
        return Ok(());
    }
    if let Some(snap) = load_table_vector_sidecar(base, table)? {
        if let Some(index) = diskann_codec::build_index_from_snapshot(&snap) {
            save_table_diskann_sidecar(base, table, &index)?;
            store.restore_diskann(table, index);
        }
    } else if store.table(table).map(|t| t.len()).unwrap_or(0) > 0 {
        store.rebuild_diskann(table);
        if let Some(index) = store.diskann_snapshot(table) {
            save_table_diskann_sidecar(base, table, &index)?;
        }
    }
    Ok(())
}

fn restore_hnsw_index(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    num_segments: u32,
) -> Result<(), String> {
    let shards = load_segment_hnsw_shards(base, table, num_segments)?;
    for (seg, index) in shards {
        store.restore_segment_hnsw(table, seg, index);
    }
    if let Some(index) = load_table_hnsw_sidecar(base, table)? {
        store.restore_hnsw(table, index);
    } else if !store.has_segment_hnsw(table) {
        store.rebuild_hnsw(table);
        if let Some(index) = store.hnsw_snapshot(table) {
            save_table_hnsw_sidecar(base, table, &index)?;
        }
    }
    if !store.has_segment_hnsw(table) {
        store.rebuild_segment_hnsw(table, num_segments);
        let snap = store.segment_hnsw_snapshot(table);
        if !snap.is_empty() {
            save_segment_hnsw_shards(base, table, &snap)?;
        }
    }
    Ok(())
}

/// Apply WAL flush records whose Parquet segments exist but are not yet in the manifest.
pub fn replay_flush_wal(base: &Path, table: &str) -> Result<usize, String> {
    let records = wal::read_flushes(base, table)?;
    if records.is_empty() {
        return Ok(0);
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    let mut manifest = if manifest_path.exists() {
        TableManifestFile::load(&manifest_path)?
    } else {
        TableManifestFile::default()
    };
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let mut recovered = 0usize;
    for record in &records {
        if manifest.segments.contains(&record.segment) {
            continue;
        }
        let path = seg_dir.join(&record.segment);
        if !path.exists() {
            continue;
        }
        let docs = read_segment(&path)?;
        if !segment_bm25_bin_path(base, table, &record.segment).exists() {
            let snap = snapshot_for_columnar_docs(&docs);
            save_segment_bm25_sidecar(base, table, &record.segment, &snap)?;
        }
        if let Some(vec_snap) = snapshot_for_columnar_vectors(&docs) {
            let vec_path = segment_vectors_bin_path(base, table, &record.segment);
            if !vec_path.exists() {
                save_segment_vector_sidecar(base, table, &record.segment, &vec_snap)?;
            }
        }
        manifest.push_segment(record.segment.clone());
        recovered += 1;
    }
    if recovered > 0 {
        manifest.save(&manifest_path)?;
    }
    let all_reconciled = records.iter().all(|r| {
        manifest.segments.contains(&r.segment) && seg_dir.join(&r.segment).exists()
    });
    if all_reconciled && !records.is_empty() {
        wal::truncate_flushes(base, table)?;
    }
    Ok(recovered)
}

pub fn load_table(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    segment_count: usize,
) -> Result<usize, String> {
    replay_flush_wal(base, table)?;
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(0);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let num_segments = num_segments_hint(segment_count.max(manifest.segments.len()));
    let merged_vectors = load_merged_segment_vector_map(base, table)?;
    let table_vectors = if merged_vectors.is_empty() {
        load_table_vector_sidecar(base, table)?
            .map(|s| s.to_map())
            .unwrap_or_default()
    } else {
        merged_vectors
    };
    let mut n = 0usize;
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if !path.exists() {
            continue;
        }
        let seg_vectors = load_segment_vector_sidecar(base, table, seg)?
            .map(|s| s.to_map())
            .unwrap_or_default();
        for doc in read_segment(&path)? {
            let embedding = seg_vectors
                .get(&doc.id)
                .or_else(|| table_vectors.get(&doc.id))
                .cloned()
                .or(doc.embedding);
            store.insert_stored(
                table,
                doc.id,
                IngestDoc {
                    text: doc.text,
                    metadata: doc.metadata,
                    vector: embedding,
                },
                num_segments,
            );
            n += 1;
        }
    }
    if n > 0 {
        restore_bm25_index(base, table, store, num_segments)?;
    }
    Ok(n)
}

pub fn flush_batch(
    base: &Path,
    table: &str,
    docs: &[ColumnarDoc],
    since_id: u64,
) -> Result<String, String> {
    if docs.is_empty() {
        return Err("flush_batch: empty docs".into());
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    let manifest = if manifest_path.exists() {
        TableManifestFile::load(&manifest_path)?
    } else {
        TableManifestFile::default()
    };
    let seg_name = format!("seg_{:05}.parquet", manifest.segments.len() + 1);
    let seg_path = TableManifestFile::segments_dir(base, table).join(&seg_name);
    write_segment(&seg_path, docs)?;
    let seg_snap = snapshot_for_columnar_docs(docs);
    save_segment_bm25_sidecar(base, table, &seg_name, &seg_snap)?;
    if let Some(vec_snap) = snapshot_for_columnar_vectors(docs) {
        save_segment_vector_sidecar(base, table, &seg_name, &vec_snap)?;
    }
    wal::append_flush(base, table, &seg_name, since_id, docs.len())?;
    let mut manifest = manifest;
    let seg_name_clone = seg_name.clone();
    manifest.push_segment(seg_name);
    manifest.save(&manifest_path)?;
    Ok(seg_name_clone)
}

pub fn flush_new_docs(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    since_id: u64,
    num_segments: u32,
) -> Result<(), String> {
    let columnar: Vec<ColumnarDoc> = store
        .docs_with_ids_since(table, since_id)
        .into_iter()
        .map(|(id, doc)| ingest_to_columnar(id, &doc))
        .collect();
    if columnar.is_empty() {
        return Ok(());
    }
    let _segment = flush_batch(base, table, &columnar, since_id)?;
    save_table_indexes(base, table, store, num_segments)?;
    Ok(())
}

/// Rebuild per-segment BM25 and/or vector sidecars from on-disk Parquet segments.
pub fn rebuild_segment_sidecars(
    base: &Path,
    table: &str,
    sparse: bool,
    vectors: bool,
) -> Result<(), String> {
    if !sparse && !vectors {
        return Ok(());
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if !path.exists() {
            continue;
        }
        let docs = read_segment(&path)?;
        if sparse {
            let snap = snapshot_for_columnar_docs(&docs);
            save_segment_bm25_sidecar(base, table, seg, &snap)?;
        }
        if vectors {
            if let Some(snap) = snapshot_for_columnar_vectors(&docs) {
                save_segment_vector_sidecar(base, table, seg, &snap)?;
            }
        }
    }
    Ok(())
}

/// Write table-level BM25, vector, and HNSW sidecars from the in-memory corpus.
pub fn save_table_indexes(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    num_segments: u32,
) -> Result<(), String> {
    if let Some(snap) = store.bm25_snapshot(table) {
        save_bm25_sidecar(base, table, &snap)?;
    }
    if let Some(snap) = vector_snapshot_from_store(store, table) {
        save_table_vector_sidecar(base, table, &snap)?;
    }
    store.rebuild_segment_hnsw(table, num_segments);
    let segment_snap = store.segment_hnsw_snapshot(table);
    if !segment_snap.is_empty() {
        save_segment_hnsw_shards(base, table, &segment_snap)?;
    }
    if let Some(index) = store.hnsw_snapshot(table) {
        save_table_hnsw_sidecar(base, table, &index)?;
    }
    store.rebuild_diskann(table);
    if let Some(index) = store.diskann_snapshot(table) {
        save_table_diskann_sidecar(base, table, &index)?;
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
