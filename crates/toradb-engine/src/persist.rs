use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use rayon::prelude::*;
use toradb_core::{CandidateSet, DocId};
use toradb_index::dense::{diskann_codec, hnsw_codec, quant_codec, turboquant_codec, vector_codec};
use toradb_index::sparse::bm25::tokenize;
use toradb_index::sparse::bm25_lexicon::{self, Bm25LexiconView};
use toradb_index::sparse::bm25_route::{self, Bm25RouteView};
use toradb_index::sparse::bm25_tbm3;
use toradb_index::sparse::learned_sparse;
use toradb_index::{Bm25Snapshot, CorpusStore, IngestDoc, SparseSnapshot, VectorSnapshot};
use toradb_storage::cache::{get_or_mmap, read_segment_cached, CachedBm25Segment, StorageCaches};
use toradb_storage::columnar::{
    bm25_snapshot_from_segment, parquet_row_count, read_segment, read_segment_id_bounds,
    read_segment_matching_ids, scan_segment_id_metadata, segment_uses_legacy_layout,
    write_segment_with_compression, ColumnarDoc, IndexMode, QueryMode, SegmentMeta,
    TableManifestFile, ROUTED_QUERY_MIN_SEGMENTS,
};
use toradb_storage::compaction::{self, CompactMode, CompactPolicy, CompactReport};
use toradb_storage::wal;

use crate::index_build_status::{
    self, mark_index_building, mark_index_failed, mark_index_ready, read_build_manifest,
    segment_sparse_up_to_date, write_build_manifest, IndexBuildPhase, SegmentBuildRecord,
};

pub use crate::index_build_status::{
    list_tables_on_disk, read_index_build_status as read_table_index_build_status,
    scan_indexing_tables,
};

pub fn default_parallelism() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4)
        .min(32)
        .max(1)
}

fn ingest_thread_count() -> usize {
    std::env::var("TORADB_INGEST_THREADS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        })
        .min(32)
}

fn num_segments_hint(segments_len: usize) -> u32 {
    segments_len.max(4) as u32
}

/// Segment-parallel fan-out for a table (manifest segment count, minimum 4).
pub fn table_segment_count(base: &Path, table: &str) -> Result<u32, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(4);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    Ok(num_segments_hint(manifest.segments.len()))
}

/// Configured rayon worker cap for distributed segment scans on this table.
/// Defaults to physical core count when not manually set via SET SEGMENT_WORKERS.
pub fn table_segment_workers(base: &Path, table: &str) -> Result<u32, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(default_parallelism());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    Ok(manifest
        .segment_workers
        .unwrap_or_else(default_parallelism)
        .max(1))
}

pub fn set_table_segment_workers(base: &Path, table: &str, workers: u32) -> Result<(), String> {
    if workers == 0 {
        return Err("segment_workers must be >= 1".into());
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Err(format!("table {table} not found"));
    }
    let mut manifest = TableManifestFile::load(&manifest_path)?;
    manifest.segment_workers = Some(workers);
    manifest.save(&manifest_path)
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

fn sparse_table_bin_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("sparse.bin")
}

fn segment_bm25_bin_path(base: &Path, table: &str, segment_parquet: &str) -> PathBuf {
    let stem = segment_parquet
        .strip_suffix(".parquet")
        .unwrap_or(segment_parquet);
    indexes_dir(base, table).join(format!("{stem}.bm25.bin"))
}

fn segment_bm25_lex_path(base: &Path, table: &str, segment_parquet: &str) -> PathBuf {
    let stem = segment_parquet
        .strip_suffix(".parquet")
        .unwrap_or(segment_parquet);
    indexes_dir(base, table).join(format!("{stem}.bm25.lex.bin"))
}

pub fn table_bm25_route_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("bm25.route.bin")
}

pub fn table_query_mode(base: &Path, table: &str) -> Result<QueryMode, String> {
    let path = TableManifestFile::path_for_table(base, table);
    if !path.exists() {
        return Ok(QueryMode::default());
    }
    Ok(TableManifestFile::load(&path)?.query_mode)
}

pub fn ensure_table_on_disk(base: &Path, table: &str) -> Result<(), String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if manifest_path.exists() {
        return Ok(());
    }
    TableManifestFile::default().save(&manifest_path)
}

pub fn tombstones_path(base: &Path, table: &str) -> PathBuf {
    indexes_dir(base, table).join("tombstones.bin")
}

pub fn load_tombstones(base: &Path, table: &str) -> std::collections::HashSet<u64> {
    let path = tombstones_path(base, table);
    if !path.exists() {
        return std::collections::HashSet::new();
    }
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Vec<u64>>(&bytes)
            .map(|v| v.into_iter().collect())
            .unwrap_or_default(),
        Err(_) => std::collections::HashSet::new(),
    }
}

pub fn save_tombstones(
    base: &Path,
    table: &str,
    ids: &std::collections::HashSet<u64>,
) -> Result<(), String> {
    let path = tombstones_path(base, table);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut sorted: Vec<u64> = ids.iter().copied().collect();
    sorted.sort_unstable();
    let data = serde_json::to_vec(&sorted).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, &path).map_err(|e| e.to_string())
}

pub fn add_tombstones(base: &Path, table: &str, ids: &[u64]) -> Result<usize, String> {
    let mut set = load_tombstones(base, table);
    let mut newly: Vec<u64> = Vec::new();
    for &id in ids {
        if set.insert(id) {
            newly.push(id);
        }
    }
    if newly.is_empty() {
        return Ok(0);
    }
    save_tombstones(base, table, &set)?;
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if manifest_path.exists() {
        if let Ok(mut manifest) = TableManifestFile::load(&manifest_path) {
            for &id in &newly {
                if let Some(seg) = manifest
                    .segment_meta
                    .iter_mut()
                    .find(|m| id >= m.min_id && id <= m.max_id)
                {
                    seg.deleted_count = seg.deleted_count.saturating_add(1);
                }
            }
            let _ = manifest.save(&manifest_path);
        }
    }
    Ok(newly.len())
}

pub fn set_table_column_types(
    base: &Path,
    table: &str,
    types: &[(String, toradb_core::ColumnTypeSpec)],
) -> Result<(), String> {
    if types.is_empty() {
        return Ok(());
    }
    ensure_table_on_disk(base, table)?;
    let manifest_path = TableManifestFile::path_for_table(base, table);
    let mut manifest = TableManifestFile::load(&manifest_path)?;
    manifest.set_column_types(types.to_vec());
    manifest.save(&manifest_path)
}

pub fn alter_table_column_type(
    base: &Path,
    table: &str,
    column: &str,
    ty: toradb_core::ColumnTypeSpec,
) -> Result<(), String> {
    ensure_table_on_disk(base, table)?;
    let manifest_path = TableManifestFile::path_for_table(base, table);
    let mut manifest = TableManifestFile::load(&manifest_path)?;
    let col = column.to_ascii_lowercase();
    let mut types = std::mem::take(&mut manifest.column_types);
    if let Some(entry) = types.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&col)) {
        entry.1 = ty;
    } else {
        types.push((col, ty));
    }
    manifest.set_column_types(types);
    manifest.save(&manifest_path)
}

pub fn table_needs_typed_segment_rewrite(base: &Path, table: &str) -> Result<bool, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(false);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    if manifest.column_types.is_empty() {
        return Ok(false);
    }
    let seg_dir = TableManifestFile::segments_dir(base, table);
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if path.exists() && segment_uses_legacy_layout(&path)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn format_alter_column_type_message(
    table: &str,
    column: &str,
    ty: toradb_core::ColumnTypeSpec,
    needs_rewrite: bool,
    compact_note: Option<&str>,
) -> String {
    let mut msg = format!("ok: set column {column} type {} on {table}", ty.sql_name());
    if let Some(note) = compact_note {
        msg.push_str("; ");
        msg.push_str(note);
    } else if needs_rewrite {
        msg.push_str(&format!(
            " (run COMPACT TABLE {table} FULL to rewrite segments)"
        ));
    }
    msg
}

pub fn format_create_table_ddl(
    table: &str,
    columns: &[(String, toradb_core::ColumnTypeSpec)],
    using: &str,
) -> String {
    if columns.is_empty() {
        format!("CREATE TABLE {table} USING {using}")
    } else {
        let cols = columns
            .iter()
            .map(|(n, t)| format!("{} {}", n, t.sql_name()))
            .collect::<Vec<_>>()
            .join(", ");
        format!("CREATE TABLE {table} ({cols}) USING {using}")
    }
}

pub fn format_describe_table(
    table: &str,
    row_count: usize,
    vector_dim: Option<usize>,
    segments: Option<u32>,
    segment_workers: Option<u32>,
    indexes: &[String],
    column_types: &[(String, toradb_core::ColumnTypeSpec)],
) -> String {
    let mut lines = vec![
        format!("table: {table}"),
        format!("rows: {row_count}"),
        format!(
            "vector_dim: {}",
            vector_dim
                .map(|d| d.to_string())
                .unwrap_or_else(|| "none".into())
        ),
    ];
    if let Some(n) = segments {
        lines.push(format!("segments: {n}"));
    }
    if let Some(w) = segment_workers {
        lines.push(format!("segment_workers: {w}"));
    }
    let indexes_line = if indexes.is_empty() {
        "none".to_string()
    } else {
        indexes.join(", ")
    };
    lines.push(format!("indexes: {indexes_line}"));
    if !column_types.is_empty() {
        lines.push("column_types:".into());
        for (name, ty) in column_types {
            lines.push(format!("  {name} {}", ty.sql_name()));
        }
    }
    lines.join("\n")
}

pub fn table_column_types_ordered(
    base: &Path,
    table: &str,
) -> Vec<(String, toradb_core::ColumnTypeSpec)> {
    let path = TableManifestFile::path_for_table(base, table);
    if !path.exists() {
        return Vec::new();
    }
    match TableManifestFile::load(&path) {
        Ok(m) => m.column_types,
        Err(_) => Vec::new(),
    }
}

pub fn table_column_types(
    base: &Path,
    table: &str,
) -> std::collections::HashMap<String, toradb_core::ColumnTypeSpec> {
    let path = TableManifestFile::path_for_table(base, table);
    if !path.exists() {
        return std::collections::HashMap::new();
    }
    match TableManifestFile::load(&path) {
        Ok(m) => m
            .column_types
            .into_iter()
            .map(|(name, ty)| (name.to_ascii_lowercase(), ty))
            .collect(),
        Err(_) => std::collections::HashMap::new(),
    }
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

fn segment_quant_bin_path(base: &Path, table: &str, segment_parquet: &str) -> PathBuf {
    let stem = segment_parquet
        .strip_suffix(".parquet")
        .unwrap_or(segment_parquet);
    indexes_dir(base, table).join(format!("{stem}.vectors.q.bin"))
}

fn segment_turboquant_bin_path(base: &Path, table: &str, segment_parquet: &str) -> PathBuf {
    let stem = segment_parquet
        .strip_suffix(".parquet")
        .unwrap_or(segment_parquet);
    indexes_dir(base, table).join(format!("{stem}.vectors.tq.bin"))
}

fn turboquant_config() -> Option<(turboquant_codec::TqMode, u8)> {
    let codec = std::env::var("TORADB_VECTOR_CODEC").ok()?;
    let mode = match codec.as_str() {
        "turboquant_mse" => turboquant_codec::TqMode::Mse,
        "turboquant_ip" => turboquant_codec::TqMode::Ip,
        _ => return None,
    };
    let bits = std::env::var("TORADB_TURBOQUANT_BITS")
        .ok()
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(3)
        .clamp(2, 4);
    Some((mode, bits))
}

fn snapshot_for_columnar_turboquant(
    docs: &[ColumnarDoc],
    mode: turboquant_codec::TqMode,
    bits: u8,
) -> Option<turboquant_codec::TurboQuantSnapshot> {
    let mut pairs = Vec::new();
    for doc in docs {
        if let Some(ref emb) = doc.embedding {
            pairs.push((doc.id, emb.clone()));
        }
    }
    if pairs.is_empty() {
        return None;
    }

    let seed = pairs.iter().fold(0u64, |acc, (id, _)| {
        acc.wrapping_mul(0x9E37).wrapping_add(*id)
    });
    turboquant_codec::TurboQuantSnapshot::from_pairs(
        &pairs,
        mode,
        bits,
        seed | 1,
        seed.rotate_left(17) | 1,
    )
    .ok()
}

fn save_segment_turboquant_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
    snap: &turboquant_codec::TurboQuantSnapshot,
) -> Result<(), String> {
    turboquant_codec::write_snapshot_file(
        &segment_turboquant_bin_path(base, table, segment_parquet),
        snap,
    )
}

fn load_bm25_snapshot_mmap(
    path: &Path,
    caches: Option<&mut StorageCaches>,
) -> Result<Bm25Snapshot, String> {
    let mmap = get_or_mmap(path, caches)?;
    bm25_tbm3::snapshot_from_tbm3(mmap.as_ref())
}

fn load_vector_snapshot_mmap(
    path: &Path,
    caches: Option<&mut StorageCaches>,
) -> Result<VectorSnapshot, String> {
    let mmap = get_or_mmap(path, caches)?;
    vector_codec::decode_snapshot(mmap.as_ref())
}

fn load_hnsw_index_mmap(
    path: &Path,
    caches: Option<&mut StorageCaches>,
) -> Result<toradb_index::dense::hnsw_index::HnswIndex, String> {
    let mmap = get_or_mmap(path, caches)?;
    hnsw_codec::decode_index(mmap.as_ref())
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
    caches: Option<&mut StorageCaches>,
) -> Result<Option<toradb_index::dense::hnsw_index::HnswIndex>, String> {
    let bin = table_hnsw_bin_path(base, table);
    if bin.exists() {
        return load_hnsw_index_mmap(&bin, caches).map(Some);
    }
    Ok(None)
}

fn load_diskann_index_mmap(
    path: &Path,
    caches: Option<&mut StorageCaches>,
) -> Result<toradb_index::dense::hnsw_index::HnswIndex, String> {
    let mmap = get_or_mmap(path, caches)?;
    diskann_codec::decode_index(mmap.as_ref())
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
    caches: Option<&mut StorageCaches>,
) -> Result<Option<toradb_index::dense::hnsw_index::HnswIndex>, String> {
    let bin = table_diskann_bin_path(base, table);
    if bin.exists() {
        return load_diskann_index_mmap(&bin, caches).map(Some);
    }
    Ok(None)
}

pub fn table_has_diskann_sidecar(base: &Path, table: &str) -> bool {
    table_diskann_bin_path(base, table).exists()
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
    mut caches: Option<&mut StorageCaches>,
) -> Result<std::collections::HashMap<u32, toradb_index::dense::hnsw_index::HnswIndex>, String> {
    let mut out = std::collections::HashMap::new();
    for seg in 0..num_segments {
        let path = segment_hnsw_shard_path(base, table, seg);
        if path.exists() {
            out.insert(seg, load_hnsw_index_mmap(&path, caches.as_deref_mut())?);
        }
    }
    Ok(out)
}

/// Names of on-disk index sidecars present for a table (for DESCRIBE / diagnostics).
pub fn table_index_sidecars(base: &Path, table: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    if bm25_table_bin_path(base, table).exists() {
        names.push("bm25".into());
    }
    if sparse_table_bin_path(base, table).exists() {
        names.push("sparse".into());
    }
    if table_vectors_bin_path(base, table).exists() {
        names.push("vectors".into());
    }
    if table_hnsw_bin_path(base, table).exists() {
        names.push("hnsw".into());
    }
    if table_diskann_bin_path(base, table).exists() {
        names.push("diskann".into());
    }
    if table_has_segment_bm25_sidecars(base, table)? {
        names.push("segment_bm25".into());
    }
    if table_has_segment_vector_sidecars(base, table)? {
        names.push("segment_vectors".into());
    }
    if table_has_segment_hnsw_sidecars(base, table)? {
        names.push("segment_hnsw".into());
    }
    names.sort_unstable();
    names.dedup();
    Ok(names)
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

pub fn save_bm25_sidecar(base: &Path, table: &str, snap: &Bm25Snapshot) -> Result<(), String> {
    bm25_tbm3::write_tbm3_file(&bm25_table_bin_path(base, table), snap)
}

pub fn save_sparse_sidecar(
    base: &Path,
    table: &str,
    snap: &SparseSnapshot,
) -> Result<(), String> {
    let dir = indexes_dir(base, table);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    learned_sparse::write_lsp1_file(&sparse_table_bin_path(base, table), snap)
}

pub fn load_sparse_sidecar(
    base: &Path,
    table: &str,
    caches: Option<&mut StorageCaches>,
) -> Result<Option<SparseSnapshot>, String> {
    let bin = sparse_table_bin_path(base, table);
    if bin.exists() {
        let mmap = get_or_mmap(&bin, caches)?;
        return learned_sparse::snapshot_from_lsp1(mmap.as_ref()).map(Some);
    }
    Ok(None)
}

pub fn save_segment_bm25_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
    snap: &Bm25Snapshot,
) -> Result<(), String> {
    bm25_tbm3::write_tbm3_file(&segment_bm25_bin_path(base, table, segment_parquet), snap)?;
    let terms = bm25_lexicon::terms_from_posting_keys(snap.postings.keys().map(|s| s.as_str()));
    bm25_lexicon::write_lexicon_file(&segment_bm25_lex_path(base, table, segment_parquet), &terms)
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

pub fn load_table_vector_sidecar(
    base: &Path,
    table: &str,
    caches: Option<&mut StorageCaches>,
) -> Result<Option<VectorSnapshot>, String> {
    let bin = table_vectors_bin_path(base, table);
    if bin.exists() {
        return load_vector_snapshot_mmap(&bin, caches).map(Some);
    }
    Ok(None)
}

fn load_segment_vector_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
    caches: Option<&mut StorageCaches>,
) -> Result<Option<VectorSnapshot>, String> {
    let bin = segment_vectors_bin_path(base, table, segment_parquet);
    if bin.exists() {
        return load_vector_snapshot_mmap(&bin, caches).map(Some);
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
    mut caches: Option<&mut StorageCaches>,
) -> Result<HashMap<u64, Vec<f32>>, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(HashMap::new());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let mut merged = HashMap::new();
    for seg in &manifest.segments {
        if let Some(snap) = load_segment_vector_sidecar(base, table, seg, caches.as_deref_mut())? {
            merged.extend(snap.to_map());
        }
    }
    Ok(merged)
}

pub fn load_bm25_sidecar(
    base: &Path,
    table: &str,
    caches: Option<&mut StorageCaches>,
) -> Result<Option<Bm25Snapshot>, String> {
    let bin = bm25_table_bin_path(base, table);
    if bin.exists() {
        return load_bm25_snapshot_mmap(&bin, caches).map(Some);
    }
    Ok(None)
}

fn load_segment_bm25_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
    caches: Option<&mut StorageCaches>,
) -> Result<Option<Bm25Snapshot>, String> {
    let bin = segment_bm25_bin_path(base, table, segment_parquet);
    if bin.exists() {
        return load_bm25_snapshot_mmap(&bin, caches).map(Some);
    }
    Ok(None)
}

/// True when at least one on-disk segment has a BM25 index blob (`.bm25.bin`).
pub fn table_index_mode(base: &Path, table: &str) -> Result<IndexMode, String> {
    let path = TableManifestFile::path_for_table(base, table);
    if !path.exists() {
        return Ok(IndexMode::Merged);
    }
    Ok(TableManifestFile::load(&path)?.index_mode)
}

/// Set manifest index mode to segment-only (bulk ingest default).
/// Rebuild `segment_id_ranges` from WAL flush records (for existing DBs missing ranges).
pub fn rebuild_segment_id_ranges(base: &Path, table: &str) -> Result<(), String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(());
    }
    let mut manifest = TableManifestFile::load(&manifest_path)?;
    manifest.segment_id_ranges.clear();
    let records = wal::read_flushes(base, table)?;
    if !records.is_empty() {
        for rec in records {
            let max_id = rec
                .since_id
                .saturating_add(rec.doc_count as u64)
                .saturating_sub(1);
            manifest.record_segment_id_range(&rec.segment, rec.since_id, max_id);
        }
    } else {
        let seg_dir = TableManifestFile::segments_dir(base, table);
        for seg in manifest.segments.clone() {
            let path = seg_dir.join(&seg);
            if !path.exists() {
                continue;
            }
            let (min_id, max_id) = read_segment_id_bounds(&path)?;
            manifest.record_segment_id_range(&seg, min_id, max_id);
        }
    }
    manifest.save(&manifest_path)
}

pub fn mark_table_segment_only(base: &Path, table: &str) -> Result<(), String> {
    let path = TableManifestFile::path_for_table(base, table);
    let mut manifest = if path.exists() {
        TableManifestFile::load(&path)?
    } else {
        TableManifestFile::default()
    };
    manifest.index_mode = IndexMode::SegmentOnly;
    manifest.save(&path)
}

/// Resolve per-segment BM25 sidecar paths in manifest order (one manifest read per query).
pub fn list_segment_bm25_bins(base: &Path, table: &str) -> Result<Vec<Option<PathBuf>>, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    Ok(manifest
        .segments
        .iter()
        .map(|seg| {
            let bin = segment_bm25_bin_path(base, table, seg);
            if bin.exists() {
                Some(bin)
            } else {
                None
            }
        })
        .collect())
}

fn load_cached_bm25_segment(bin_path: &Path) -> Result<CachedBm25Segment, String> {
    let mmap = get_or_mmap(bin_path, None)?;
    CachedBm25Segment::from_mmap(mmap).map_err(|e| {
        format!(
            "invalid BM25 sidecar at {} (expected TBM3): {}; re-run: toradb-ingest resume",
            bin_path.display(),
            e
        )
    })
}

/// BM25 search against one on-disk segment sidecar path.
pub fn search_segment_bm25_at_path(
    bin_path: &Path,
    query: &str,
    k: usize,
    caches: Option<&StorageCaches>,
) -> Result<CandidateSet, String> {
    if !bin_path.exists() {
        return Ok(CandidateSet::default());
    }
    if let Some(caches) = caches {
        if let Ok(guard) = caches
            .segment_bm25
            .read()
            .map_err(|_| "segment_bm25 cache lock poisoned".to_string())
        {
            if let Some(entry) = guard.get(bin_path) {
                return entry.search(query, k);
            }
        }
        let entry = {
            let mut guard = caches
                .segment_bm25
                .write()
                .map_err(|_| "segment_bm25 cache lock poisoned".to_string())?;
            let path = bin_path.to_path_buf();
            guard.get_or_insert(bin_path, || load_cached_bm25_segment(&path))?
        };
        return entry.search(query, k);
    }
    load_cached_bm25_segment(bin_path)?.search(query, k)
}

/// Segment indices to scan for BM25 given query terms and optional route/lexicon pruning.
pub fn filter_bm25_segment_indices(
    base: &Path,
    table: &str,
    num_segments: u32,
    query: &str,
    caches: Option<&StorageCaches>,
) -> Result<Vec<u32>, String> {
    let terms: Vec<String> = tokenize(query);
    if terms.is_empty() {
        return Ok((0..num_segments).collect());
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    let real_segments = if manifest_path.exists() {
        TableManifestFile::load(&manifest_path)
            .map(|m| m.segments.len())
            .unwrap_or(num_segments as usize)
    } else {
        num_segments as usize
    };
    if real_segments <= 1 {
        return Ok(vec![0]);
    }
    let query_mode = table_query_mode(base, table)?;
    if query_mode == QueryMode::Routed {
        let route_path = table_bm25_route_path(base, table);
        if route_path.exists() {
            let mmap = match caches {
                Some(c) => c.route_mmap(&route_path).ok(),
                None => None,
            };
            let owned;
            let bytes: &[u8] = match &mmap {
                Some(m) => m.as_ref(),
                None => {
                    owned = std::fs::read(&route_path).map_err(|e| e.to_string())?;
                    &owned
                }
            };
            if let Ok(route) = Bm25RouteView::open(bytes) {
                let segs = route.segments_for_query(terms.iter().map(|s| s.as_str()));
                if !segs.is_empty() {
                    return Ok(segs);
                }
            }
        }
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok((0..num_segments).collect());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let mut out = Vec::new();
    for (idx, seg) in manifest.segments.iter().enumerate() {
        if idx >= num_segments as usize {
            break;
        }
        let lex_path = segment_bm25_lex_path(base, table, seg);
        if !lex_path.exists() {
            continue;
        }
        let bytes = std::fs::read(&lex_path).map_err(|e| e.to_string())?;
        if let Ok(lex) = Bm25LexiconView::parse(&bytes) {
            if lex.intersects_query_terms(terms.iter().map(|s| s.as_str())) {
                out.push(idx as u32);
            }
        } else {
            out.push(idx as u32);
        }
    }
    if out.is_empty() {
        Ok((0..num_segments).collect())
    } else {
        Ok(out)
    }
}

/// Build `indexes/bm25.route.bin` from per-segment lexicons.
pub fn build_bm25_route_index(base: &Path, table: &str) -> Result<(), String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let mut lexicons = Vec::new();
    for (idx, seg) in manifest.segments.iter().enumerate() {
        let lex_path = segment_bm25_lex_path(base, table, seg);
        if !lex_path.exists() {
            continue;
        }
        let bytes = std::fs::read(&lex_path).map_err(|e| e.to_string())?;
        let view = Bm25LexiconView::parse(&bytes)?;
        let terms: Vec<String> = view.terms.iter().map(|s| s.to_string()).collect();
        lexicons.push((idx as u32, terms));
    }
    let entries = bm25_route::merge_lexicons_into_route(lexicons.into_iter());
    bm25_route::write_route_file(&table_bm25_route_path(base, table), &entries)?;
    let mut manifest = manifest;
    if manifest.segments.len() as u32 >= ROUTED_QUERY_MIN_SEGMENTS {
        manifest.query_mode = QueryMode::Routed;
    }
    manifest.save(&manifest_path)
}

/// BM25 search against one segment sidecar (mmap + decoded index LRU when `caches` is set).
pub fn search_segment_bm25_sidecar(
    base: &Path,
    table: &str,
    segment_index: u32,
    query: &str,
    k: usize,
    caches: Option<&StorageCaches>,
) -> Result<CandidateSet, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(CandidateSet::default());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let Some(seg) = manifest.segments.get(segment_index as usize) else {
        return Ok(CandidateSet::default());
    };
    let bin_path = segment_bm25_bin_path(base, table, seg);
    search_segment_bm25_at_path(&bin_path, query, k, caches)
}

/// True when merged BM25 is loaded in memory for this table (skip redundant segment fan-out).
pub fn table_has_merged_bm25_in_memory(store: &CorpusStore, table: &str) -> bool {
    store
        .table(table)
        .map(|t| t.has_bm25_index())
        .unwrap_or(false)
}

pub fn table_has_segment_bm25_sidecars(base: &Path, table: &str) -> Result<bool, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(false);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    for seg in &manifest.segments {
        if segment_bm25_bin_path(base, table, seg).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Merge all per-segment BM25 sidecars under `indexes/`.
pub fn load_merged_segment_bm25_sidecars(
    base: &Path,
    table: &str,
    caches: Option<&mut StorageCaches>,
) -> Result<Option<Bm25Snapshot>, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let base_owned = base.to_path_buf();
    let table_owned = table.to_string();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(ingest_thread_count())
        .build()
        .map_err(|e| e.to_string())?;
    let snaps: Vec<Bm25Snapshot> = pool.install(|| {
        manifest
            .segments
            .par_iter()
            .filter_map(|seg| {
                load_segment_bm25_sidecar(&base_owned, &table_owned, seg, None)
                    .ok()
                    .flatten()
            })
            .collect()
    });
    let _ = caches;
    let use_interner = std::env::var("TORADB_BM25_INTERNER")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(snaps.len() >= 8);
    if use_interner {
        Ok(Bm25Snapshot::merge_snapshots_interned(snaps))
    } else {
        Ok(Bm25Snapshot::merge_snapshots_tree(snaps))
    }
}

fn snapshot_for_columnar_docs(docs: &[ColumnarDoc]) -> Bm25Snapshot {
    Bm25Snapshot::from_documents(docs.iter().map(|d| (d.id, d.text.as_str())))
}

fn snapshot_for_segment_bm25(path: &Path) -> Result<Bm25Snapshot, String> {
    bm25_snapshot_from_segment(path)
}

fn snapshot_for_columnar_quant(docs: &[ColumnarDoc]) -> Option<quant_codec::QuantVectorSnapshot> {
    let mut pairs = Vec::new();
    for doc in docs {
        if let Some(ref emb) = doc.embedding {
            pairs.push((doc.id, emb.clone()));
        }
    }
    if pairs.is_empty() {
        return None;
    }
    quant_codec::QuantVectorSnapshot::from_pairs(&pairs).ok()
}

/// Load dequantized vectors for the given doc ids from segment quant sidecars.
pub fn load_vectors_for_ids(
    base: &Path,
    table: &str,
    ids: &[DocId],
) -> Result<HashMap<DocId, Vec<f32>>, String> {
    use toradb_index::dense::quant_codec;
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let want: HashSet<DocId> = ids.iter().copied().collect();
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(HashMap::new());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let mut out = HashMap::new();
    for seg in &manifest.segments {
        let path = segment_quant_bin_path(base, table, seg);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let snap = quant_codec::decode_snapshot(&bytes)?;
        for (i, &id) in snap.ids.iter().enumerate() {
            if want.contains(&id) && !out.contains_key(&id) {
                if let Ok(v) = snap.decompress_vector(i) {
                    out.insert(id, v);
                }
            }
        }
        if out.len() >= want.len() {
            break;
        }
    }
    Ok(out)
}

pub fn load_turboquant_sidecars(
    base: &Path,
    table: &str,
) -> Result<Vec<turboquant_codec::TurboQuantSnapshot>, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let mut out = Vec::new();
    for seg in &manifest.segments {
        let path = segment_turboquant_bin_path(base, table, seg);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        out.push(turboquant_codec::decode_snapshot(&bytes)?);
    }
    Ok(out)
}

pub fn table_has_turboquant_sidecars(base: &Path, table: &str) -> Result<bool, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(false);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    for seg in &manifest.segments {
        if segment_turboquant_bin_path(base, table, seg).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn table_has_quant_sidecars(base: &Path, table: &str) -> Result<bool, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(false);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    for seg in &manifest.segments {
        if segment_quant_bin_path(base, table, seg).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn save_segment_quant_sidecar(
    base: &Path,
    table: &str,
    segment_parquet: &str,
    snap: &quant_codec::QuantVectorSnapshot,
) -> Result<(), String> {
    quant_codec::write_snapshot_file(&segment_quant_bin_path(base, table, segment_parquet), snap)
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

pub fn scan_table_id_metadata(
    store: &CorpusStore,
    base: Option<&Path>,
    table: &str,
    mut f: impl FnMut(u64, &HashMap<String, String>) -> Result<(), String>,
) -> Result<(), String> {
    let mut mem_rows = 0usize;
    let mut scan_err: Option<String> = None;
    store.for_each_metadata(table, |id, metadata| {
        mem_rows += 1;
        if scan_err.is_none() {
            if let Err(e) = f(id, metadata) {
                scan_err = Some(e);
            }
        }
    });
    if let Some(e) = scan_err {
        return Err(e);
    }
    if mem_rows > 0 {
        return Ok(());
    }

    let Some(base) = base else {
        return Ok(());
    };
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
        scan_segment_id_metadata(&path, &manifest.column_types, |id, metadata| {
            f(id, &metadata)
        })?;
    }
    Ok(())
}

/// Read all documents for a table from on-disk Parquet segments.
pub fn read_table_documents(
    base: &Path,
    table: &str,
    mut caches: Option<&mut StorageCaches>,
) -> Result<Vec<(u64, IngestDoc)>, String> {
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
        for doc in read_segment_cached(&path, caches.as_deref_mut())? {
            out.push((
                doc.id,
                IngestDoc {
                    text: doc.text,
                    metadata: doc.metadata,
                    vector: doc.embedding,
                    sparse: None,
                },
            ));
        }
    }
    Ok(out)
}

pub fn table_row_count_on_disk(
    store: &CorpusStore,
    base: &Path,
    table: &str,
) -> Result<usize, String> {
    let deleted = load_tombstones(base, table).len();
    if let Some(t) = store.table(table) {
        let n = t.len();
        if n > 0 {
            return Ok(n.saturating_sub(deleted));
        }
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(0);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    if !manifest.segment_id_ranges.is_empty() {
        let n: u64 = manifest
            .segment_id_ranges
            .iter()
            .map(|r| r.max_id.saturating_sub(r.min_id) + 1)
            .sum();
        return Ok((n as usize).saturating_sub(deleted));
    }
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let mut n = 0usize;
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if path.exists() {
            n += parquet_row_count(&path)?;
        }
    }
    Ok(n.saturating_sub(deleted))
}

/// In-memory corpus first; fall back to columnar Parquet scan when empty.
pub fn table_documents(
    store: &CorpusStore,
    base: Option<&Path>,
    table: &str,
    mut caches: Option<&mut StorageCaches>,
) -> Result<Vec<(u64, IngestDoc)>, String> {
    let mem = store.all_documents(table);
    if !mem.is_empty() {
        return Ok(mem);
    }
    if let Some(base) = base {
        return read_table_documents(base, table, caches.as_deref_mut());
    }
    Ok(Vec::new())
}

fn columnar_to_ingest(doc: ColumnarDoc) -> IngestDoc {
    IngestDoc {
        text: doc.text,
        metadata: doc.metadata,
        vector: doc.embedding,
        sparse: None,
    }
}

/// Load documents by id: in-memory corpus first, then targeted Parquet segment reads.
pub fn fetch_documents_by_ids(
    store: &CorpusStore,
    base: Option<&Path>,
    table: &str,
    ids: &[u64],
    _caches: Option<&mut StorageCaches>,
) -> Result<Vec<(u64, IngestDoc)>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut want: std::collections::HashSet<u64> = ids.iter().copied().collect();
    let mut out: Vec<(u64, IngestDoc)> = Vec::with_capacity(ids.len());

    for (id, doc) in store.documents_by_ids(table, ids) {
        want.remove(&id);
        out.push((id, doc));
    }
    if want.is_empty() {
        return Ok(out);
    }
    let Some(base) = base else {
        return Ok(out);
    };
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(out);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);

    if !manifest.segment_id_ranges.is_empty() {
        for range in &manifest.segment_id_ranges {
            if want.is_empty() {
                break;
            }
            let path = seg_dir.join(&range.file);
            if !path.exists() {
                continue;
            }
            let seg_want: HashSet<u64> = want
                .iter()
                .filter(|id| **id >= range.min_id && **id <= range.max_id)
                .copied()
                .collect();
            if seg_want.is_empty() {
                continue;
            }
            let bounds = Some((range.min_id, range.max_id));
            for doc in read_segment_matching_ids(&path, &seg_want, bounds)? {
                want.remove(&doc.id);
                out.push((doc.id, columnar_to_ingest(doc)));
            }
        }
    } else {
        let remaining: Vec<u64> = want.iter().copied().collect();
        for seg in manifest.segments_for_ids(&remaining) {
            if want.is_empty() {
                break;
            }
            let path = seg_dir.join(seg);
            if !path.exists() {
                continue;
            }
            for doc in read_segment_matching_ids(&path, &want, None)? {
                want.remove(&doc.id);
                out.push((doc.id, columnar_to_ingest(doc)));
            }
        }
    }
    Ok(out)
}

/// Remove a table directory from disk.
pub fn drop_table(base: &Path, table: &str) -> Result<(), String> {
    if crate::materialized::is_materialized_view(base, table) {
        return crate::materialized::drop_materialized_view(base, table);
    }
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
    let mut views = crate::materialized::list_materialized_views(base)?;
    names.append(&mut views);
    names.sort_unstable();
    names.dedup();
    Ok(names)
}

pub fn load_all(
    base: &Path,
    store: &mut CorpusStore,
    segment_count: usize,
    mut caches: Option<&mut StorageCaches>,
) -> Result<usize, String> {
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
        total += load_table(base, &name, store, segment_count, caches.as_deref_mut())?;
    }
    Ok(total)
}

fn restore_bm25_index(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    num_segments: u32,
    mut caches: Option<&mut StorageCaches>,
) -> Result<(), String> {
    if let Some(snap) = load_merged_segment_bm25_sidecars(base, table, caches.as_deref_mut())? {
        store.restore_bm25(table, snap);
    } else if let Some(snap) = load_bm25_sidecar(base, table, caches.as_deref_mut())? {
        store.restore_bm25(table, snap);
    } else {
        store.rebuild_bm25(table);
        if let Some(snap) = store.bm25_snapshot(table) {
            save_bm25_sidecar(base, table, &snap)?;
        }
    }
    // Learned-sparse sidecar is optional: absent for text-only / older tables.
    if let Some(snap) = load_sparse_sidecar(base, table, caches.as_deref_mut())? {
        store.restore_sparse(table, snap);
    }
    restore_hnsw_index(base, table, store, num_segments, caches.as_deref_mut())?;
    restore_diskann_index(base, table, store, caches.as_deref_mut())?;
    restore_turboquant_sidecars(base, table, store)?;
    Ok(())
}

fn restore_turboquant_sidecars(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
) -> Result<(), String> {
    if !table_has_turboquant_sidecars(base, table)? {
        return Ok(());
    }
    let snaps = load_turboquant_sidecars(base, table)?;
    if !snaps.is_empty() {
        store.restore_turboquant_segments(table, snaps);
    }
    Ok(())
}

fn restore_diskann_index(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    mut caches: Option<&mut StorageCaches>,
) -> Result<(), String> {
    if let Some(index) = load_table_diskann_sidecar(base, table, caches.as_deref_mut())? {
        store.restore_diskann(table, index);
        return Ok(());
    }
    if let Some(snap) = load_table_vector_sidecar(base, table, caches.as_deref_mut())? {
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
    mut caches: Option<&mut StorageCaches>,
) -> Result<(), String> {
    let shards = load_segment_hnsw_shards(base, table, num_segments, caches.as_deref_mut())?;
    for (seg, index) in shards {
        store.restore_segment_hnsw(table, seg, index);
    }
    if let Some(index) = load_table_hnsw_sidecar(base, table, caches.as_deref_mut())? {
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
    wal::checkpoint_after_manifest(base, table, &manifest.segments, &seg_dir)?;
    Ok(recovered)
}

pub fn load_table(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    segment_count: usize,
    mut caches: Option<&mut StorageCaches>,
) -> Result<usize, String> {
    replay_flush_wal(base, table)?;
    replay_compaction_wal(base, table)?;
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(0);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    if manifest.index_mode == IndexMode::SegmentOnly {
        return load_table_segment_only(base, table, store, &manifest);
    }
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let num_segments = num_segments_hint(segment_count.max(manifest.segments.len()));
    let merged_vectors = load_merged_segment_vector_map(base, table, caches.as_deref_mut())?;
    let table_vectors = if merged_vectors.is_empty() {
        load_table_vector_sidecar(base, table, caches.as_deref_mut())?
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
        let seg_vectors = load_segment_vector_sidecar(base, table, seg, caches.as_deref_mut())?
            .map(|s| s.to_map())
            .unwrap_or_default();
        for doc in read_segment_cached(&path, caches.as_deref_mut())? {
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
                    sparse: None,
                },
                num_segments,
            );
            n += 1;
        }
    }
    if n > 0 {
        restore_bm25_index(base, table, store, num_segments, caches.as_deref_mut())?;
    }
    Ok(n)
}

/// Register a segment-only table without loading texts or merged BM25 into RAM.
pub fn load_table_segment_only(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    manifest: &TableManifestFile,
) -> Result<usize, String> {
    store.ensure_table(table);
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let mut n = 0usize;
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if path.exists() {
            n += parquet_row_count(&path)?;
        }
    }
    Ok(n)
}

pub fn flush_batch(
    base: &Path,
    table: &str,
    docs: &[ColumnarDoc],
    since_id: u64,
) -> Result<String, String> {
    flush_batch_with_opts(
        base,
        table,
        docs,
        since_id,
        &toradb_core::IngestOptions::default(),
    )
}

pub fn flush_batch_with_opts(
    base: &Path,
    table: &str,
    docs: &[ColumnarDoc],
    since_id: u64,
    opts: &toradb_core::IngestOptions,
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
    write_segment_with_compression(
        &seg_path,
        docs,
        manifest.compression.as_ref(),
        &manifest.column_types,
    )?;
    if !opts.defer_bm25 {
        let seg_snap = snapshot_for_columnar_docs(docs);
        save_segment_bm25_sidecar(base, table, &seg_name, &seg_snap)?;
    }
    if !opts.defer_dense_rebuild {
        if let Some(vec_snap) = snapshot_for_columnar_vectors(docs) {
            save_segment_vector_sidecar(base, table, &seg_name, &vec_snap)?;
        }
        if let Some(qsnap) = snapshot_for_columnar_quant(docs) {
            save_segment_quant_sidecar(base, table, &seg_name, &qsnap)?;
        }
        if let Some((mode, bits)) = turboquant_config() {
            if let Some(tqsnap) = snapshot_for_columnar_turboquant(docs, mode, bits) {
                save_segment_turboquant_sidecar(base, table, &seg_name, &tqsnap)?;
            }
        }
    }
    wal::append_flush(
        base,
        table,
        &seg_name,
        since_id,
        docs.len(),
        !opts.defer_wal_fsync,
    )?;
    let mut manifest = manifest;
    let seg_name_clone = seg_name.clone();
    let max_id = since_id.saturating_add(docs.len() as u64).saturating_sub(1);
    let byte_size = seg_path.metadata().map(|m| m.len()).unwrap_or(0);
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let generation = manifest.next_generation();
    manifest.push_segment_meta(SegmentMeta {
        file: seg_name.clone(),
        min_id: since_id,
        max_id,
        tier: SegmentMeta::tier_for_bytes(byte_size),
        generation,
        created_at,
        byte_size,
        row_count: docs.len() as u64,
        deleted_count: 0,
    });
    manifest.save(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    if !opts.defer_wal_fsync {
        wal::checkpoint_after_manifest(base, table, &manifest.segments, &seg_dir)?;
    }
    Ok(seg_name_clone)
}

pub fn finalize_bulk_wal(base: &Path, table: &str) -> Result<(), String> {
    wal::sync_flush_log(base, table)?;
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(());
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    wal::checkpoint_after_manifest(base, table, &manifest.segments, &seg_dir)?;
    Ok(())
}

fn remove_segment_sidecars(base: &Path, table: &str, segment_parquet: &str) -> Result<(), String> {
    let paths = [
        segment_bm25_bin_path(base, table, segment_parquet),
        segment_bm25_lex_path(base, table, segment_parquet),
        segment_vectors_bin_path(base, table, segment_parquet),
        segment_quant_bin_path(base, table, segment_parquet),
        segment_turboquant_bin_path(base, table, segment_parquet),
    ];
    for p in &paths {
        if p.exists() {
            std::fs::remove_file(p).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Replay compaction WAL: drop removed segment names from manifest if files are gone.
pub fn replay_compaction_wal(base: &Path, table: &str) -> Result<usize, String> {
    let records = wal::read_compactions(base, table)?;
    if records.is_empty() {
        return Ok(0);
    }
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(0);
    }
    let mut manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let mut fixed = 0usize;
    for record in &records {
        for removed in &record.removed {
            if manifest.segments.contains(removed) && !seg_dir.join(removed).exists() {
                manifest.remove_segment(removed);
                fixed += 1;
            }
        }
        for (i, added) in record.added.iter().enumerate() {
            if !manifest.segments.contains(added) && seg_dir.join(added).exists() {
                manifest.push_segment(added.clone());
                fixed += 1;
            }
            if let Some(tier) = record.added_tiers.get(i).copied() {
                if let Some(m) = manifest.segment_meta.iter_mut().find(|m| m.file == *added) {
                    if m.tier == 0 && tier > 0 {
                        m.tier = tier;
                    }
                }
            }
        }
    }
    if fixed > 0 {
        manifest.save(&manifest_path)?;
    }
    let all_present = manifest.segments.iter().all(|s| seg_dir.join(s).exists());
    if all_present {
        wal::truncate_compactions(base, table)?;
    }
    Ok(fixed)
}

pub fn compact_table(
    base: &Path,
    table: &str,
    store: Option<&mut CorpusStore>,
    mode: CompactMode,
    policy: &CompactPolicy,
    mut caches: Option<&mut StorageCaches>,
) -> Result<CompactReport, String> {
    replay_compaction_wal(base, table)?;
    let report = compaction::compact_table_segments(base, table, policy, mode)?;
    if report.merges == 0 {
        return Ok(report);
    }
    let seg_dir = TableManifestFile::segments_dir(base, table);
    for removed in &report.removed {
        remove_segment_sidecars(base, table, removed)?;
        if let Some(caches) = caches.as_deref_mut() {
            caches.invalidate_segment(&seg_dir.join(removed));
            caches.invalidate_index_blob(&segment_bm25_bin_path(base, table, removed));
            caches.invalidate_index_blob(&segment_vectors_bin_path(base, table, removed));
        }
    }
    for added in &report.added {
        if let Some(caches) = caches.as_deref_mut() {
            caches.invalidate_segment(&seg_dir.join(added));
        }
    }
    rebuild_segment_sidecars(base, table, true, true)?;
    let num_segments = table_segment_count(base, table)?;
    if let Some(store) = store {
        save_table_indexes(base, table, store, num_segments)?;
    } else {
        let mut tmp = CorpusStore::default();
        let n = load_table(
            base,
            table,
            &mut tmp,
            num_segments as usize,
            caches.as_deref_mut(),
        )?;
        if n > 0 {
            save_table_indexes(base, table, &mut tmp, num_segments)?;
        }
    }
    let added_tiers: Vec<u8> = report.tier_transitions.iter().map(|(_, t)| *t).collect();
    wal::append_compaction(base, table, &report.removed, &report.added, &added_tiers)?;
    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, table))?;
    wal::checkpoint_after_manifest(base, table, &manifest.segments, &seg_dir)?;
    wal::truncate_compactions(base, table)?;
    rebuild_segment_id_ranges(base, table)?;
    Ok(report)
}

pub fn maybe_compact_after_flush(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    caches: Option<&mut StorageCaches>,
) -> Result<Option<CompactReport>, String> {
    let policy = CompactPolicy::from_env();
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(None);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    if !compaction::should_compact_tiered(&manifest, &seg_dir, &policy) {
        return Ok(None);
    }
    let report = compact_table(base, table, Some(store), CompactMode::Auto, &policy, caches)?;
    if report.merges > 0 {
        Ok(Some(report))
    } else {
        Ok(None)
    }
}

pub fn flush_new_docs(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    since_id: u64,
    num_segments: u32,
    caches: Option<&mut StorageCaches>,
    opts: toradb_core::IngestOptions,
) -> Result<(), String> {
    let columnar: Vec<ColumnarDoc> = store
        .docs_with_ids_since(table, since_id)
        .into_iter()
        .map(|(id, doc)| ingest_to_columnar(id, &doc))
        .collect();
    if columnar.is_empty() {
        return Ok(());
    }
    let _segment = flush_batch_with_opts(base, table, &columnar, since_id, &opts)?;
    if !opts.defer_table_indexes {
        save_table_indexes(base, table, store, num_segments)?;
    }
    if !opts.defer_compaction {
        let _ = maybe_compact_after_flush(base, table, store, caches)?;
    }
    Ok(())
}

/// Rebuild per-segment BM25 and/or vector sidecars from on-disk Parquet segments.
pub fn rebuild_segment_sidecars(
    base: &Path,
    table: &str,
    sparse: bool,
    vectors: bool,
) -> Result<(), String> {
    rebuild_segment_sidecars_with_progress(base, table, sparse, vectors, true, true)
}

pub fn rebuild_segment_sidecars_with_progress(
    base: &Path,
    table: &str,
    sparse: bool,
    vectors: bool,
    skip_unchanged: bool,
    report_progress: bool,
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
    let segments_total = manifest.segments.len() as u32;
    let mut build_manifest = read_build_manifest(base, table);

    if report_progress {
        mark_index_building(base, table, IndexBuildPhase::SegmentBm25, 0, segments_total)?;
    }

    let base_owned = base.to_path_buf();
    let table_owned = table.to_string();
    let segments: Vec<String> = manifest.segments.clone();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(ingest_thread_count())
        .build()
        .map_err(|e| e.to_string())?;

    let segments_done_atomic = AtomicU32::new(0);
    let base_progress = base.to_path_buf();
    let table_progress = table.to_string();

    let results: Vec<Result<(String, Option<SegmentBuildRecord>), String>> = pool.install(|| {
        segments
            .par_iter()
            .map(|seg| {
                let path = seg_dir.join(seg);
                if !path.exists() {
                    return Ok((seg.clone(), None));
                }
                if skip_unchanged
                    && sparse
                    && segment_sparse_up_to_date(
                        &base_owned,
                        &table_owned,
                        seg,
                        &path,
                        &build_manifest,
                    )
                {
                    let record = SegmentBuildRecord {
                        segment: seg.clone(),
                        sparse_done: true,
                        parquet_mtime_secs: index_build_status::parquet_mtime_secs(&path),
                    };
                    let done = segments_done_atomic.fetch_add(1, Ordering::Relaxed) + 1;
                    if report_progress {
                        let _ = mark_index_building(
                            &base_progress,
                            &table_progress,
                            IndexBuildPhase::SegmentBm25,
                            done,
                            segments_total,
                        );
                    }
                    return Ok((seg.clone(), Some(record)));
                }
                if sparse && !vectors {
                    let snap = snapshot_for_segment_bm25(&path)?;
                    save_segment_bm25_sidecar(&base_owned, &table_owned, seg, &snap)?;
                } else {
                    let docs = read_segment(&path)?;
                    if sparse {
                        let snap = snapshot_for_columnar_docs(&docs);
                        save_segment_bm25_sidecar(&base_owned, &table_owned, seg, &snap)?;
                    }
                    if vectors {
                        if let Some(snap) = snapshot_for_columnar_vectors(&docs) {
                            let vec_path = segment_vectors_bin_path(&base_owned, &table_owned, seg);
                            if !vec_path.exists() {
                                save_segment_vector_sidecar(&base_owned, &table_owned, seg, &snap)?;
                            }
                        }
                        if let Some(qsnap) = snapshot_for_columnar_quant(&docs) {
                            let quant_path = segment_quant_bin_path(&base_owned, &table_owned, seg);
                            if !quant_path.exists() {
                                save_segment_quant_sidecar(&base_owned, &table_owned, seg, &qsnap)?;
                            }
                        }
                        if let Some((mode, bits)) = turboquant_config() {
                            if let Some(tqsnap) =
                                snapshot_for_columnar_turboquant(&docs, mode, bits)
                            {
                                let tq_path =
                                    segment_turboquant_bin_path(&base_owned, &table_owned, seg);
                                if !tq_path.exists() {
                                    save_segment_turboquant_sidecar(
                                        &base_owned,
                                        &table_owned,
                                        seg,
                                        &tqsnap,
                                    )?;
                                }
                            }
                        }
                    }
                }
                let record = SegmentBuildRecord {
                    segment: seg.clone(),
                    sparse_done: sparse,
                    parquet_mtime_secs: index_build_status::parquet_mtime_secs(&path),
                };
                let done = segments_done_atomic.fetch_add(1, Ordering::Relaxed) + 1;
                if report_progress {
                    let _ = mark_index_building(
                        &base_progress,
                        &table_progress,
                        IndexBuildPhase::SegmentBm25,
                        done,
                        segments_total,
                    );
                }
                Ok((seg.clone(), Some(record)))
            })
            .collect()
    });

    for result in results {
        let (seg, record) = result?;
        if let Some(rec) = record {
            if let Some(entry) = build_manifest
                .segments
                .iter_mut()
                .find(|e| e.segment == seg)
            {
                *entry = rec;
            } else {
                build_manifest.segments.push(rec);
            }
        }
    }
    write_build_manifest(base, table, &build_manifest)?;

    if report_progress {
        mark_index_ready(base, table)?;
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
    if store.sparse_snapshot(table).is_none() {
        store.rebuild_sparse(table);
    }
    if let Some(snap) = store.sparse_snapshot(table) {
        save_sparse_sidecar(base, table, &snap)?;
    }
    if let Some(snap) = vector_snapshot_from_store(store, table) {
        save_table_vector_sidecar(base, table, &snap)?;
    }
    let dense = store.table(table).is_some_and(|t| t.has_vectors());
    if dense {
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
    }
    Ok(())
}

fn docs_to_columnar(since_id: u64, docs: &[IngestDoc]) -> Vec<ColumnarDoc> {
    docs.iter()
        .enumerate()
        .map(|(i, d)| ingest_to_columnar(since_id + i as u64, d))
        .collect()
}

/// Bulk path with in-memory corpus (legacy).
pub fn flush_ingest_batch(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    since_id: u64,
    docs: Vec<IngestDoc>,
    num_segments: u32,
    opts: toradb_core::IngestOptions,
) -> Result<usize, String> {
    if docs.is_empty() {
        return Ok(0);
    }
    let columnar = docs_to_columnar(since_id, &docs);
    let added = store.ingest_bulk_batch(table, since_id, docs, num_segments, opts);
    flush_batch_with_opts(base, table, &columnar, since_id, &opts)?;
    Ok(added)
}

/// Bulk path without retaining document text in memory.
pub fn flush_ingest_batch_disk_only(
    base: &Path,
    table: &str,
    since_id: u64,
    docs: &[IngestDoc],
    opts: &toradb_core::IngestOptions,
) -> Result<usize, String> {
    if docs.is_empty() {
        return Ok(0);
    }
    let columnar = docs_to_columnar(since_id, docs);
    flush_batch_with_opts(base, table, &columnar, since_id, opts)?;
    Ok(docs.len())
}

/// Flush one Arrow batch to a new Parquet segment without `IngestDoc` allocation.
pub fn flush_arrow_batch_disk_only(
    base: &Path,
    table: &str,
    since_id: u64,
    batch: &arrow::record_batch::RecordBatch,
    opts: &toradb_core::IngestOptions,
) -> Result<usize, String> {
    let columnar = crate::arrow_batch::record_batch_to_columnar(batch, since_id)?;
    if columnar.is_empty() {
        return Ok(0);
    }
    flush_batch_with_opts(base, table, &columnar, since_id, opts)?;
    Ok(columnar.len())
}

pub fn reload_table_texts_from_segments(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    num_segments: u32,
) -> Result<usize, String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    if !manifest_path.exists() {
        return Ok(0);
    }
    let manifest = TableManifestFile::load(&manifest_path)?;
    let seg_dir = TableManifestFile::segments_dir(base, table);
    let mut loaded = 0usize;
    for seg in &manifest.segments {
        let path = seg_dir.join(seg);
        if !path.exists() {
            continue;
        }
        let docs = read_segment(&path)?;
        for doc in docs {
            let ingest = IngestDoc {
                text: doc.text,
                metadata: doc.metadata,
                vector: doc.embedding,
                sparse: None,
            };
            store.insert_stored(table, doc.id, ingest, num_segments);
            loaded += 1;
        }
    }
    Ok(loaded)
}

/// After bulk ingest: build segment sidecars, merge BM25, write table-level sidecars.
pub fn finalize_bulk_table_indexes(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    num_segments: u32,
    mut caches: Option<&mut StorageCaches>,
    reload_texts: bool,
) -> Result<(), String> {
    let manifest_path = TableManifestFile::path_for_table(base, table);
    let segments_total = if manifest_path.exists() {
        TableManifestFile::load(&manifest_path)?.segments.len() as u32
    } else {
        0
    };

    mark_index_building(base, table, IndexBuildPhase::SegmentBm25, 0, segments_total)?;

    let result = (|| {
        finalize_bulk_wal(base, table)?;
        let vectors = store.table(table).is_some_and(|t| t.has_vectors());
        rebuild_segment_sidecars_with_progress(base, table, true, vectors, true, true)?;
        if table_index_mode(base, table)? == IndexMode::SegmentOnly {
            let _ = build_bm25_route_index(base, table);
        }

        let index_mode = table_index_mode(base, table)?;
        if index_mode != IndexMode::SegmentOnly {
            mark_index_building(
                base,
                table,
                IndexBuildPhase::MergeBm25,
                segments_total,
                segments_total,
            )?;
            restore_bm25_index(base, table, store, num_segments, caches.as_deref_mut())?;

            mark_index_building(
                base,
                table,
                IndexBuildPhase::TableIndexes,
                segments_total,
                segments_total,
            )?;
            save_table_indexes(base, table, store, num_segments)?;
        }

        if reload_texts && index_mode != IndexMode::SegmentOnly {
            mark_index_building(
                base,
                table,
                IndexBuildPhase::ReloadTexts,
                segments_total,
                segments_total,
            )?;
            reload_table_texts_from_segments(base, table, store, num_segments)?;
        }
        rebuild_segment_id_ranges(base, table)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            mark_index_ready(base, table)?;
            Ok(())
        }
        Err(e) => {
            let _ = mark_index_failed(base, table, &e);
            Err(e)
        }
    }
}

/// Resume or run index build outside an active bulk session (crash recovery).
pub fn resume_table_indexes(
    base: &Path,
    table: &str,
    store: &mut CorpusStore,
    num_segments: u32,
    caches: Option<&mut StorageCaches>,
    reload_texts: bool,
) -> Result<(), String> {
    finalize_bulk_table_indexes(base, table, store, num_segments, caches, reload_texts)
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

fn search_log_path(base: &Path, table: &str) -> PathBuf {
    base.join(table).join("_search_log.ndjson")
}

pub fn append_search_log(base: &Path, table: &str, record: &toradb_core::ProvenanceRecord) {
    let path = search_log_path(base, table);
    if let Ok(line) = serde_json::to_string(record) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
}

pub fn read_search_log(
    base: &Path,
    table: &str,
    limit: usize,
) -> Result<Vec<toradb_core::ProvenanceRecord>, String> {
    let path = search_log_path(base, table);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut records: Vec<toradb_core::ProvenanceRecord> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if records.len() > limit {
        let start = records.len() - limit;
        records = records[start..].to_vec();
    }
    Ok(records)
}
