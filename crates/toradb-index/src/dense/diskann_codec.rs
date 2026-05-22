//! On-disk DiskANN graph snapshot (magic `TDA1` + bincode HNSW-style graph payload).

use super::hnsw_index::{should_use_hnsw, HnswIndex};
use super::vector_codec::VectorSnapshot;

pub const DISKANN_MAGIC: &[u8; 4] = b"TDA1";
pub const DISKANN_VERSION: u8 = 1;

pub fn encode_index(index: &HnswIndex) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(index).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(5 + payload.len());
    out.extend_from_slice(DISKANN_MAGIC);
    out.push(DISKANN_VERSION);
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn decode_index(bytes: &[u8]) -> Result<HnswIndex, String> {
    if bytes.len() < 5 {
        return Err("diskann sidecar too short".into());
    }
    if &bytes[..4] != DISKANN_MAGIC {
        return Err("invalid diskann sidecar magic".into());
    }
    if bytes[4] != DISKANN_VERSION {
        return Err(format!("unsupported diskann sidecar version {}", bytes[4]));
    }
    bincode::deserialize(&bytes[5..]).map_err(|e| e.to_string())
}

pub fn write_index_file(path: &std::path::Path, index: &HnswIndex) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = encode_index(index)?;
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Build a graph index from a mmap-friendly vector snapshot (on-disk embeddings).
pub fn build_index_from_snapshot(snap: &VectorSnapshot) -> Option<HnswIndex> {
    let dim = snap.dim as usize;
    if dim == 0 {
        return None;
    }
    let mut ids = Vec::with_capacity(snap.ids.len());
    let mut vectors = Vec::with_capacity(snap.ids.len());
    for (i, &id) in snap.ids.iter().enumerate() {
        let start = i * dim;
        let end = start + dim;
        if end > snap.data.len() {
            return None;
        }
        ids.push(id);
        vectors.push(snap.data[start..end].to_vec());
    }
    let index = HnswIndex::build(ids, vectors)?;
    if should_use_hnsw(index.len()) {
        Some(index)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_diskann_index() {
        let dim = 4;
        let ids: Vec<u64> = (0..40).collect();
        let vectors: Vec<Vec<f32>> = ids
            .iter()
            .map(|i| {
                let mut v = vec![0.0; dim];
                v[*i as usize % dim] = 1.0;
                v
            })
            .collect();
        let index = HnswIndex::build(ids, vectors).expect("build");
        let bytes = encode_index(&index).unwrap();
        let decoded = decode_index(&bytes).unwrap();
        let mut q = vec![0.0; dim];
        q[39 % dim] = 1.0;
        assert_eq!(index.search(&q, 5).ids, decoded.search(&q, 5).ids);
    }
}
