//! Asymmetric distance computation (ADC) for TurboQuant codes.

#[inline]
#[cfg_attr(not(test), allow(dead_code))]
fn read_code(codes: &[u8], bits: u8, i: usize) -> u8 {
    match bits {
        1 => (codes[i >> 3] >> (i & 7)) & 0x1,
        2 => (codes[i >> 2] >> ((i & 3) * 2)) & 0x3,
        3 | 4 => (codes[i >> 1] >> ((i & 1) * 4)) & 0xF,
        _ => unreachable!("bits must be 1..=4"),
    }
}

pub fn codes_byte_len(padded_dim: usize, bits: u8) -> usize {
    let bits_total = padded_dim * bits as usize;
    let stored_bits_per_dim = match bits {
        1 => 1,
        2 => 2,
        3 | 4 => 4,
        _ => panic!("bits must be 1..=4"),
    };
    let _ = bits_total;
    (padded_dim * stored_bits_per_dim + 7) / 8
}

pub fn encode_codes(values: &[u8], bits: u8) -> Vec<u8> {
    let n = values.len();
    let mut out = vec![0u8; codes_byte_len(n, bits)];
    match bits {
        1 => {
            for i in 0..n {
                out[i >> 3] |= (values[i] & 0x1) << (i & 7);
            }
        }
        2 => {
            for i in 0..n {
                out[i >> 2] |= (values[i] & 0x3) << ((i & 3) * 2);
            }
        }
        3 | 4 => {
            for i in 0..n {
                out[i >> 1] |= (values[i] & 0xF) << ((i & 1) * 4);
            }
        }
        _ => panic!("bits must be 1..=4"),
    }
    out
}

/// MSE ADC dot product
/// inlined scalar helpers in [`crate::kernels::scalar`].
pub fn tq_adc_mse(query_rot: &[f32], codes: &[u8], bits: u8, codebook: &[f32]) -> f32 {
    debug_assert!((1..=4).contains(&bits));
    match bits {
        1 => {
            debug_assert_eq!(codebook.len(), 2);
            crate::kernels::scalar::tq_adc_mse_1bit(query_rot, codes, codebook)
        }
        2 => {
            debug_assert_eq!(codebook.len(), 4);
            crate::kernels::scalar::tq_adc_mse_2bit(query_rot, codes, codebook)
        }
        // 3-bit storage is 4-bit-packed but codebook is 8-entry.
        3 => {
            debug_assert_eq!(codebook.len(), 8);
            crate::kernels::scalar::tq_adc_mse_3bit(query_rot, codes, codebook)
        }
        4 => {
            debug_assert_eq!(codebook.len(), 16);
            (crate::dispatch::table().tq_adc_mse_4bit)(query_rot, codes, codebook)
        }
        _ => unreachable!(),
    }
}

/// Quantized Johnson–Lindenstrauss residual contribution.
pub fn tq_adc_qjl(
    query_rot: &[f32],
    qjl_bits: &[u8],
    qjl_g: &[f32],
    qjl_scale: f32,
) -> f32 {
    let n = query_rot.len();
    let mut acc = 0.0f32;
    for i in 0..n {
        let bit = (qjl_bits[i >> 3] >> (i & 7)) & 1;
        let stored_sign = if bit == 1 { 1.0f32 } else { -1.0f32 };
        acc += query_rot[i] * qjl_g[i] * stored_sign;
    }
    acc * qjl_scale
}

pub fn tq_adc_ip(
    query_rot: &[f32],
    codes_mse: &[u8],
    bits: u8,
    codebook: &[f32],
    qjl_bits: &[u8],
    qjl_g: &[f32],
    qjl_scale: f32,
) -> f32 {
    tq_adc_mse(query_rot, codes_mse, bits, codebook)
        + tq_adc_qjl(query_rot, qjl_bits, qjl_g, qjl_scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip_all_bits() {
        for bits in [1u8, 2, 3, 4] {
            let max = (1u8 << bits.min(4)) - 1;
            let max = if bits == 3 { 7 } else { max };
            let vals: Vec<u8> = (0..73).map(|i| ((i as u8) ^ 0x5A) % (max + 1)).collect();
            let packed = encode_codes(&vals, bits);
            for i in 0..vals.len() {
                let got = read_code(&packed, bits, i);
                let expected = vals[i] & if bits >= 3 { 0xF } else { (1u8 << bits) - 1 };
                assert_eq!(got, expected, "bits={bits} i={i}");
            }
        }
    }

    #[test]
    fn adc_mse_matches_reference() {
        let bits = 2u8;
        let codebook = [-1.0f32, -0.3, 0.3, 1.0];
        let codes_vals: Vec<u8> = (0..32).map(|i| ((i * 7) % 4) as u8).collect();
        let packed = encode_codes(&codes_vals, bits);
        let q: Vec<f32> = (0..32).map(|i| (i as f32 * 0.1) - 1.5).collect();
        let got = tq_adc_mse(&q, &packed, bits, &codebook);
        let expected: f32 = (0..32)
            .map(|i| q[i] * codebook[codes_vals[i] as usize])
            .sum();
        assert!((got - expected).abs() < 1e-5, "{got} vs {expected}");
    }
}
