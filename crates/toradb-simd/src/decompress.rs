/// Decompress one 8-bit scalar-quantized vector block into `out`.
pub fn decompress_block(codes: &[u8], min: f32, scale: f32, out: &mut [f32]) {
    let n = codes.len().min(out.len());
    for i in 0..n {
        out[i] = min + codes[i] as f32 * scale;
    }
}
