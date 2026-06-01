//! Compact on-disk embedding snapshot (magic `TVM1` + rkyv payload).

use std::collections::HashMap;

use crate::index_blob;

use toradb_core::DocId;

pub const VECTOR_MAGIC: &[u8; 4] = b"TVM1";

#[derive(
    Debug,
    Clone,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct VectorSnapshot {
    pub dim: u32,
    pub ids: Vec<DocId>,
    pub data: Vec<f32>,
}

impl VectorSnapshot {
    pub fn from_pairs(dim: u32, pairs: &[(DocId, Vec<f32>)]) -> Result<Self, String> {
        if dim == 0 {
            return Err("vector dim must be > 0".into());
        }
        let mut ids = Vec::with_capacity(pairs.len());
        let mut data = Vec::with_capacity(pairs.len() * dim as usize);
        for (id, vec) in pairs {
            if vec.len() != dim as usize {
                return Err(format!("embedding dim mismatch for doc {id}"));
            }
            ids.push(*id);
            data.extend_from_slice(vec);
        }
        Ok(Self { dim, ids, data })
    }

    pub fn to_map(&self) -> HashMap<DocId, Vec<f32>> {
        let dim = self.dim as usize;
        self.ids
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                let start = i * dim;
                let vec = self.data[start..start + dim].to_vec();
                (id, vec)
            })
            .collect()
    }
}

pub fn encode_snapshot(snap: &VectorSnapshot) -> Result<Vec<u8>, String> {
    index_blob::encode(VECTOR_MAGIC, snap)
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<VectorSnapshot, String> {
    index_blob::decode(VECTOR_MAGIC, bytes)
}

pub fn write_snapshot_file(path: &std::path::Path, snap: &VectorSnapshot) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = encode_snapshot(snap)?;
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_vector_snapshot() {
        let snap =
            VectorSnapshot::from_pairs(3, &[(0, vec![1.0, 0.0, 0.0]), (2, vec![0.0, 1.0, 0.0])])
                .unwrap();
        let bytes = encode_snapshot(&snap).unwrap();
        let decoded = decode_snapshot(&bytes).unwrap();
        assert_eq!(snap, decoded);
        let map = decoded.to_map();
        assert_eq!(map.get(&0).unwrap().as_slice(), [1.0, 0.0, 0.0]);
    }
}
