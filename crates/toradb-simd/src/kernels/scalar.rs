pub fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

pub fn decompress_block(codes: &[u8], min: f32, scale: f32, out: &mut [f32]) {
    let n = codes.len().min(out.len());
    for i in 0..n {
        out[i] = min + codes[i] as f32 * scale;
    }
}

pub fn popcnt_u64(word: u64) -> u32 {
    word.count_ones()
}

pub fn popcnt_slice_u64(words: &[u64]) -> u64 {
    words.iter().map(|w| w.count_ones() as u64).sum()
}

/// Scalar ADC dot for 4-bit codes. `codebook.len()` must be 16.
/// Codes are bit-packed 2 dims per byte (low nibble = first dim).
pub fn tq_adc_mse_4bit(query: &[f32], codes: &[u8], codebook: &[f32]) -> f32 {
    debug_assert_eq!(codebook.len(), 16);
    let mut acc = 0.0f32;
    let n = query.len();
    let mut i = 0;
    while i + 2 <= n {
        let b = codes[i >> 1];
        let lo = (b & 0x0F) as usize;
        let hi = (b >> 4) as usize;
        acc += query[i] * codebook[lo] + query[i + 1] * codebook[hi];
        i += 2;
    }
    if i < n {
        let b = codes[i >> 1];
        let lo = (b & 0x0F) as usize;
        acc += query[i] * codebook[lo];
    }
    acc
}

/// Scalar ADC dot for 3-bit codes. Storage is 4-bit-packed but
/// `codebook.len()` is 8 — the high bit of each nibble is always zero by
/// construction at encode time.
pub fn tq_adc_mse_3bit(query: &[f32], codes: &[u8], codebook: &[f32]) -> f32 {
    debug_assert_eq!(codebook.len(), 8);
    let mut acc = 0.0f32;
    let n = query.len();
    let mut i = 0;
    while i + 2 <= n {
        let b = codes[i >> 1];
        let lo = (b & 0x07) as usize;
        let hi = ((b >> 4) & 0x07) as usize;
        acc += query[i] * codebook[lo] + query[i + 1] * codebook[hi];
        i += 2;
    }
    if i < n {
        let b = codes[i >> 1];
        let lo = (b & 0x07) as usize;
        acc += query[i] * codebook[lo];
    }
    acc
}

/// Scalar ADC dot for 2-bit codes. `codebook.len()` must be 4.
pub fn tq_adc_mse_2bit(query: &[f32], codes: &[u8], codebook: &[f32]) -> f32 {
    debug_assert_eq!(codebook.len(), 4);
    let mut acc = 0.0f32;
    let n = query.len();
    let mut i = 0;
    while i + 4 <= n {
        let b = codes[i >> 2];
        let c0 = (b & 0x3) as usize;
        let c1 = ((b >> 2) & 0x3) as usize;
        let c2 = ((b >> 4) & 0x3) as usize;
        let c3 = ((b >> 6) & 0x3) as usize;
        acc += query[i] * codebook[c0]
            + query[i + 1] * codebook[c1]
            + query[i + 2] * codebook[c2]
            + query[i + 3] * codebook[c3];
        i += 4;
    }
    while i < n {
        let b = codes[i >> 2];
        let c = ((b >> ((i & 3) * 2)) & 0x3) as usize;
        acc += query[i] * codebook[c];
        i += 1;
    }
    acc
}

/// Scalar ADC dot for 1-bit codes. `codebook.len()` must be 2.
pub fn tq_adc_mse_1bit(query: &[f32], codes: &[u8], codebook: &[f32]) -> f32 {
    debug_assert_eq!(codebook.len(), 2);
    let mut acc = 0.0f32;
    let n = query.len();
    let mut i = 0;
    while i + 8 <= n {
        let b = codes[i >> 3];
        for k in 0..8 {
            let c = ((b >> k) & 0x1) as usize;
            acc += query[i + k] * codebook[c];
        }
        i += 8;
    }
    while i < n {
        let b = codes[i >> 3];
        let c = ((b >> (i & 7)) & 0x1) as usize;
        acc += query[i] * codebook[c];
        i += 1;
    }
    acc
}
