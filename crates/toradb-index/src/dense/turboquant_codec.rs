//! TurboQuant on-disk vector snapshot (magic `TTQ1`).

use std::collections::HashMap;

use toradb_core::DocId;
use toradb_simd::fht;
use toradb_simd::tq_adc;

use crate::dense::tq_codebook;
use crate::index_blob;

pub const TURBOQUANT_MAGIC: &[u8; 4] = b"TTQ1";

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
#[repr(u8)]
pub enum TqMode {
    Mse = 0,
    Ip = 1,
}

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
pub struct TurboQuantSnapshot {
    pub mode: TqMode,
    pub bits: u8,
    pub orig_dim: u32,
    pub padded_dim: u32,
    pub rotation_seed: u64,
    pub qjl_seed: u64,
    /// Per-vector sigma used at encode time (= ||v_rot|| / sqrt(padded_dim)).
    pub sigmas: Vec<f32>,
    /// Per-vector QJL scale factor (only populated in `Ip` mode).
    pub qjl_scales: Vec<f32>,
    pub ids: Vec<DocId>,
    /// Bit-packed MSE codes. Layout per [`toradb_simd::tq_adc`].
    pub codes: Vec<u8>,
    /// Bit-packed QJL signs (1 byte per 8 padded dims). Empty for `Mse` mode.
    pub qjl_bits: Vec<u8>,
}

impl TurboQuantSnapshot {
    pub fn from_pairs(
        pairs: &[(DocId, Vec<f32>)],
        mode: TqMode,
        bits: u8,
        rotation_seed: u64,
        qjl_seed: u64,
    ) -> Result<Self, String> {
        if pairs.is_empty() {
            return Err("empty turboquant snapshot".into());
        }
        if !(1..=4).contains(&bits) {
            return Err("turboquant bits must be 1..=4".into());
        }
        let orig_dim = pairs[0].1.len();
        if orig_dim == 0 {
            return Err("vector dim must be > 0".into());
        }
        let padded_dim = fht::next_pow2(orig_dim);

        // MSE bits stored 1/2/4 bits/dim. For Ip mode we encode at (bits-1).
        let mse_bits = match mode {
            TqMode::Mse => bits,
            TqMode::Ip => {
                if bits < 2 {
                    return Err("Ip mode requires bits >= 2".into());
                }
                bits - 1
            }
        };

        let codes_bytes_per_vec = tq_adc::codes_byte_len(padded_dim, mse_bits);
        let qjl_bytes_per_vec = if matches!(mode, TqMode::Ip) {
            (padded_dim + 7) / 8
        } else {
            0
        };

        let mut ids = Vec::with_capacity(pairs.len());
        let mut sigmas = Vec::with_capacity(pairs.len());
        let mut qjl_scales = Vec::with_capacity(pairs.len());
        let mut codes = vec![0u8; codes_bytes_per_vec * pairs.len()];
        let mut qjl_bits = vec![0u8; qjl_bytes_per_vec * pairs.len()];

        let mut rotated = vec![0f32; padded_dim];
        let mut qjl_g = vec![0f32; padded_dim];
        if matches!(mode, TqMode::Ip) {
            fht::rademacher_signs(qjl_seed, &mut qjl_g);
        }
        let cb = tq_codebook::codebook(mse_bits);

        for (vec_idx, (id, vec)) in pairs.iter().enumerate() {
            if vec.len() != orig_dim {
                return Err(format!("embedding dim mismatch for doc {id}"));
            }
            rotated.iter_mut().for_each(|x| *x = 0.0);
            rotated[..orig_dim].copy_from_slice(vec);
            fht::apply_rotation(rotation_seed, &mut rotated);

            let norm2: f32 = rotated.iter().map(|x| x * x).sum();
            let sigma = (norm2 / padded_dim as f32).sqrt().max(f32::EPSILON);
            sigmas.push(sigma);
            ids.push(*id);

            // Quantize normalized coordinates.
            let mut code_vals = vec![0u8; padded_dim];
            for i in 0..padded_dim {
                let z = rotated[i] / sigma;
                code_vals[i] = tq_codebook::quantize(z, mse_bits);
            }
            let packed = tq_adc::encode_codes(&code_vals, mse_bits);
            let off = vec_idx * codes_bytes_per_vec;
            codes[off..off + packed.len()].copy_from_slice(&packed);

            if matches!(mode, TqMode::Ip) {
                // Residual in normalized space.
                let mut residual_norm2 = 0.0f32;
                let qoff = vec_idx * qjl_bytes_per_vec;
                for i in 0..padded_dim {
                    let z = rotated[i] / sigma;
                    let r = z - cb[code_vals[i] as usize];
                    residual_norm2 += r * r;
                    let sign_bit = if r * qjl_g[i] >= 0.0 { 1u8 } else { 0u8 };
                    qjl_bits[qoff + (i >> 3)] |= sign_bit << (i & 7);
                }
                // Unbiased estimator scale for the half-normal random variable
                // E[|X|] for X = r · g with ||r|| known: per-coord contribution
                // ≈ (||r||/sqrt(d)) * sigma * sqrt(pi/2).
                let scale = sigma * (residual_norm2 / padded_dim as f32).sqrt()
                    * (std::f32::consts::PI / 2.0).sqrt();
                qjl_scales.push(scale);
            } else {
                qjl_scales.push(0.0);
            }
        }

        Ok(Self {
            mode,
            bits: mse_bits,
            orig_dim: orig_dim as u32,
            padded_dim: padded_dim as u32,
            rotation_seed,
            qjl_seed,
            sigmas,
            qjl_scales,
            ids,
            codes,
            qjl_bits,
        })
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    fn codes_bytes_per_vec(&self) -> usize {
        tq_adc::codes_byte_len(self.padded_dim as usize, self.bits)
    }

    fn qjl_bytes_per_vec(&self) -> usize {
        if matches!(self.mode, TqMode::Ip) {
            ((self.padded_dim as usize) + 7) / 8
        } else {
            0
        }
    }

    /// Rotate a query into the same space the codes were quantized in.
    pub fn rotate_query(&self, query: &[f32]) -> Vec<f32> {
        let padded = self.padded_dim as usize;
        let mut out = vec![0f32; padded];
        let n = query.len().min(self.orig_dim as usize);
        out[..n].copy_from_slice(&query[..n]);
        fht::apply_rotation(self.rotation_seed, &mut out);
        out
    }

    /// Compute ADC dot estimate `<query, vectors[index]>` in the original
    /// (pre-rotation) space, using the rotated query.
    pub fn adc_dot(&self, query_rot: &[f32], index: usize) -> f32 {
        debug_assert_eq!(query_rot.len(), self.padded_dim as usize);
        let cb = tq_codebook::codebook(self.bits);
        let stride = self.codes_bytes_per_vec();
        let off = index * stride;
        let codes = &self.codes[off..off + stride];
        let sigma = self.sigmas[index];

        let mse_dot_norm = tq_adc::tq_adc_mse(query_rot, codes, self.bits, cb);
        let mse_dot = sigma * mse_dot_norm;

        if matches!(self.mode, TqMode::Ip) {
            let qstride = self.qjl_bytes_per_vec();
            let qoff = index * qstride;
            let qjl_bits = &self.qjl_bits[qoff..qoff + qstride];
            let mut g = vec![0f32; self.padded_dim as usize];
            fht::rademacher_signs(self.qjl_seed, &mut g);
            let qjl_dot =
                tq_adc::tq_adc_qjl(query_rot, qjl_bits, &g, self.qjl_scales[index]);
            mse_dot + qjl_dot
        } else {
            mse_dot
        }
    }

    /// Reconstruct an approximate vector in the original space (for re-rank
    /// fallback or debugging). Cost: O(padded_dim log padded_dim).
    pub fn decompress_vector(&self, index: usize) -> Vec<f32> {
        let padded = self.padded_dim as usize;
        let cb = tq_codebook::codebook(self.bits);
        let stride = self.codes_bytes_per_vec();
        let off = index * stride;
        let codes = &self.codes[off..off + stride];
        let sigma = self.sigmas[index];

        let mut rot = vec![0f32; padded];
        for i in 0..padded {
            let c = tq_adc_read_code(codes, self.bits, i) as usize;
            rot[i] = sigma * cb[c];
        }
        fht::apply_rotation_inverse(self.rotation_seed, &mut rot);
        rot.truncate(self.orig_dim as usize);
        rot
    }

    pub fn to_map(&self) -> HashMap<DocId, Vec<f32>> {
        let mut out = HashMap::with_capacity(self.ids.len());
        for (i, &id) in self.ids.iter().enumerate() {
            out.insert(id, self.decompress_vector(i));
        }
        out
    }
}

#[inline]
fn tq_adc_read_code(codes: &[u8], bits: u8, i: usize) -> u8 {
    match bits {
        1 => (codes[i >> 3] >> (i & 7)) & 0x1,
        2 => (codes[i >> 2] >> ((i & 3) * 2)) & 0x3,
        3 | 4 => (codes[i >> 1] >> ((i & 1) * 4)) & 0xF,
        _ => unreachable!(),
    }
}

pub fn encode_snapshot(snap: &TurboQuantSnapshot) -> Result<Vec<u8>, String> {
    index_blob::encode(TURBOQUANT_MAGIC, snap)
}

pub fn decode_snapshot(bytes: &[u8]) -> Result<TurboQuantSnapshot, String> {
    index_blob::decode(TURBOQUANT_MAGIC, bytes)
}

pub fn write_snapshot_file(
    path: &std::path::Path,
    snap: &TurboQuantSnapshot,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_corpus(n: usize, dim: usize) -> Vec<(DocId, Vec<f32>)> {
        (0..n)
            .map(|i| {
                let v: Vec<f32> = (0..dim)
                    .map(|j| ((i * 31 + j * 7) as f32 * 0.013).sin())
                    .collect();
                (i as u64, v)
            })
            .collect()
    }

    #[test]
    fn roundtrip_mse_snapshot() {
        let pairs = make_corpus(8, 128);
        let snap = TurboQuantSnapshot::from_pairs(&pairs, TqMode::Mse, 4, 1, 2).unwrap();
        let bytes = encode_snapshot(&snap).unwrap();
        let decoded = decode_snapshot(&bytes).unwrap();
        assert_eq!(snap, decoded);
        assert_eq!(decoded.len(), 8);
        let approx = decoded.decompress_vector(0);
        assert_eq!(approx.len(), 128);
    }

    #[test]
    fn adc_mse_correlates_with_true_dot() {
        let pairs = make_corpus(64, 256);
        let snap =
            TurboQuantSnapshot::from_pairs(&pairs, TqMode::Mse, 4, 0xABCD, 0).unwrap();
        let query: Vec<f32> =
            (0..256).map(|j| ((j as f32) * 0.021).cos()).collect();
        let qrot = snap.rotate_query(&query);

        let mut estimates = Vec::new();
        let mut truths = Vec::new();
        for i in 0..pairs.len() {
            let est = snap.adc_dot(&qrot, i);
            let truth: f32 = query
                .iter()
                .zip(pairs[i].1.iter())
                .map(|(a, b)| a * b)
                .sum();
            estimates.push(est);
            truths.push(truth);
        }
        // Pearson correlation
        let mean_e: f32 = estimates.iter().sum::<f32>() / estimates.len() as f32;
        let mean_t: f32 = truths.iter().sum::<f32>() / truths.len() as f32;
        let mut num = 0.0;
        let mut de = 0.0;
        let mut dt = 0.0;
        for i in 0..estimates.len() {
            let a = estimates[i] - mean_e;
            let b = truths[i] - mean_t;
            num += a * b;
            de += a * a;
            dt += b * b;
        }
        let corr = num / (de.sqrt() * dt.sqrt() + 1e-9);
        assert!(corr > 0.9, "ADC/truth correlation too low: {corr}");
    }

    #[test]
    fn adc_ip_estimator_correlates() {
        let pairs = make_corpus(64, 256);
        let snap =
            TurboQuantSnapshot::from_pairs(&pairs, TqMode::Ip, 3, 0xABCD, 0xF00D).unwrap();
        let query: Vec<f32> =
            (0..256).map(|j| ((j as f32) * 0.021).cos()).collect();
        let qrot = snap.rotate_query(&query);
        let est: Vec<f32> = (0..pairs.len()).map(|i| snap.adc_dot(&qrot, i)).collect();
        let truth: Vec<f32> = pairs
            .iter()
            .map(|(_, v)| query.iter().zip(v.iter()).map(|(a, b)| a * b).sum::<f32>())
            .collect();
        // Top-1 of estimate should match top-1 of truth in most cases; weaker
        // check: argmax agrees within top-5.
        let argmax_est =
            est.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
        let mut indexed: Vec<(usize, f32)> = truth.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let top5: Vec<usize> = indexed.iter().take(5).map(|(i, _)| *i).collect();
        assert!(
            top5.contains(&argmax_est),
            "argmax({argmax_est}) not in top5({top5:?})"
        );
    }
}
