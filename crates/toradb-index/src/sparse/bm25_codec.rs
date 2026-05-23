//! Compact on-disk BM25 snapshot format (magic `TBM1` + rkyv payload).

use crate::index_blob;
use crate::sparse::bm25::Bm25Snapshot;

pub const BM25_MAGIC: &[u8; 4] = b"TBM1";

pub fn encode_snapshot(snap: &Bm25Snapshot) -> Result<Vec<u8>, String> {
    index_blob::encode(BM25_MAGIC, snap)
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    index_blob::decode(BM25_MAGIC, bytes)
}

pub fn write_snapshot_file(path: &std::path::Path, snap: &Bm25Snapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = encode_snapshot(snap)?;
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}
