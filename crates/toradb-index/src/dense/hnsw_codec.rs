//! Compact on-disk HNSW graph snapshot (magic `THM1` + bincode payload).

use super::hnsw_index::HnswIndex;

pub const HNSW_MAGIC: &[u8; 4] = b"THM1";
pub const HNSW_VERSION: u8 = 1;

pub fn encode_index(index: &HnswIndex) -> Result<Vec<u8>, String> {
    let payload = bincode::serialize(index).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(5 + payload.len());
    out.extend_from_slice(HNSW_MAGIC);
    out.push(HNSW_VERSION);
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn decode_index(bytes: &[u8]) -> Result<HnswIndex, String> {
    if bytes.len() < 5 {
        return Err("hnsw sidecar too short".into());
    }
    if &bytes[..4] != HNSW_MAGIC {
        return Err("invalid hnsw sidecar magic".into());
    }
    if bytes[4] != HNSW_VERSION {
        return Err(format!("unsupported hnsw sidecar version {}", bytes[4]));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dense::hnsw_index::HnswIndex;

    #[test]
    fn roundtrip_hnsw_index() {
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
        assert_eq!(decoded.len(), 40);
        let mut q = vec![0.0; dim];
        q[39 % dim] = 1.0;
        let before = index.search(&q, 5);
        let after = decoded.search(&q, 5);
        assert_eq!(before.ids, after.ids);
    }
}
