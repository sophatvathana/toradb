//! On-disk DiskANN graph snapshot (magic `TDA1` + rkyv HNSW-style graph payload).

use super::hnsw_index::{should_use_hnsw, HnswIndex};
use super::vector_codec::VectorSnapshot;
use crate::index_blob;

pub const DISKANN_MAGIC: &[u8; 4] = b"TDA1";

pub fn encode_index(index: &HnswIndex) -> Result<Vec<u8>, String> {
    index_blob::encode(DISKANN_MAGIC, index)
}

pub fn decode_index(bytes: &[u8]) -> Result<HnswIndex, String> {
    index_blob::decode(DISKANN_MAGIC, bytes)
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
