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
