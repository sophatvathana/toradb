//! Compact on-disk BM25 snapshot format (magic `TBM1` + bincode payload).

use crate::sparse::bm25::Bm25Snapshot;

pub const BM25_MAGIC: &[u8; 4] = b"TBM1";
pub const BM25_VERSION: u8 = 1;

pub fn encode_snapshot(snap: &Bm25Snapshot) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(snap).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(5 + payload.len());
    out.extend_from_slice(BM25_MAGIC);
    out.push(BM25_VERSION);
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<Bm25Snapshot, String> {
    if bytes.len() < 5 {
        return Err("bm25 sidecar too short".into());
    }
    if &bytes[..4] != BM25_MAGIC {
        return Err("invalid bm25 sidecar magic".into());
    }
    if bytes[4] != BM25_VERSION {
        return Err(format!("unsupported bm25 sidecar version {}", bytes[4]));
    }
    bincode::deserialize(&bytes[5..]).map_err(|e| e.to_string())
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
