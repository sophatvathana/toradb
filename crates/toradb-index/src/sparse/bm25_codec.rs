//! BM25 sidecar on-disk format (`TBM2` + rkyv payload).

use crate::index_blob;
use crate::sparse::bm25::Bm25Snapshot;

pub const BM25_MAGIC: &[u8; 4] = b"TBM2";

pub fn encode_snapshot(snap: &Bm25Snapshot) -> Result<Vec<u8>, String> {
    index_blob::encode(BM25_MAGIC, snap)
}

const BM25_MAGIC_V1: &[u8; 4] = b"TBM1";

pub fn decode_snapshot(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    if bytes.len() >= 4 {
        if &bytes[..4] == BM25_MAGIC {
            return index_blob::decode(BM25_MAGIC, bytes);
        }
        if &bytes[..4] == BM25_MAGIC_V1 {
            return index_blob::decode(BM25_MAGIC_V1, bytes);
        }
    }
    Err("invalid bm25 sidecar magic (expected TBM2 or TBM1)".into())
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
