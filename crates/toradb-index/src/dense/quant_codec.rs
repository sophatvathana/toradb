//! 8-bit scalar-quantized vector sidecar (magic `TQM1`).

use std::collections::HashMap;

use toradb_core::DocId;
use toradb_simd::decompress;

use crate::index_blob;

pub const QUANT_MAGIC: &[u8; 4] = b"TQM1";

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
pub struct QuantVectorSnapshot {
    pub dim: u32,
    pub ids: Vec<DocId>,
    pub mins: Vec<f32>,
    pub scales: Vec<f32>,
    pub codes: Vec<u8>,
}

impl QuantVectorSnapshot {
    pub fn from_pairs(pairs: &[(DocId, Vec<f32>)]) -> Result<Self, String> {
        if pairs.is_empty() {
            return Err("empty quant snapshot".into());
        }
        let dim = pairs[0].1.len() as u32;
        if dim == 0 {
            return Err("vector dim must be > 0".into());
        }
        let mut ids = Vec::with_capacity(pairs.len());
        let mut mins = Vec::with_capacity(pairs.len());
        let mut scales = Vec::with_capacity(pairs.len());
        let mut codes = Vec::with_capacity(pairs.len() * dim as usize);
        for (id, vec) in pairs {
            if vec.len() != dim as usize {
                return Err(format!("embedding dim mismatch for doc {id}"));
            }
            let min = vec.iter().copied().fold(f32::INFINITY, f32::min);
            let max = vec.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let scale = ((max - min) / 255.0).max(f32::EPSILON);
            ids.push(*id);
            mins.push(min);
            scales.push(scale);
            for v in vec {
                let code = ((v - min) / scale).clamp(0.0, 255.0) as u8;
                codes.push(code);
            }
        }
        Ok(Self {
            dim,
            ids,
            mins,
            scales,
            codes,
        })
    }

    pub fn decompress_vector(&self, index: usize) -> Result<Vec<f32>, String> {
        let dim = self.dim as usize;
        let start = index * dim;
        let end = start + dim;
        if end > self.codes.len() {
            return Err("quant index out of range".into());
        }
        let mut out = vec![0f32; dim];
        let block = &self.codes[start..end];
        decompress::decompress_block(block, self.mins[index], self.scales[index], &mut out);
        Ok(out)
    }

    pub fn to_map(&self) -> HashMap<DocId, Vec<f32>> {
        let mut out = HashMap::new();
        for (i, &id) in self.ids.iter().enumerate() {
            if let Ok(vec) = self.decompress_vector(i) {
                out.insert(id, vec);
            }
        }
        out
    }
}

pub fn encode_snapshot(snap: &QuantVectorSnapshot) -> Result<Vec<u8>, String> {
    index_blob::encode(QUANT_MAGIC, snap)
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<QuantVectorSnapshot, String> {
    index_blob::decode(QUANT_MAGIC, bytes)
}

pub fn write_snapshot_file(
    path: &std::path::Path,
    snap: &QuantVectorSnapshot,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = encode_snapshot(snap)?;
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}
