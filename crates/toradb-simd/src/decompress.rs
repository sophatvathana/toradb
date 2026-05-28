/// Decompress one 8-bit scalar-quantized vector block into `out`.
pub fn decompress_block(codes: &[u8], min: f32, scale: f32, out: &mut [f32]) {
    (crate::dispatch::table().decompress_block)(codes, min, scale, out)
}

#[cfg(test)]
mod tests {
    use super::decompress_block;

    #[test]
    fn decompress_matches_expected() {
        let codes: Vec<u8> = (0..64).map(|i| (i * 3 % 256) as u8).collect();
        let mut out = vec![0.0; codes.len()];
        decompress_block(&codes, -2.5, 0.125, &mut out);
        for (i, v) in out.iter().enumerate() {
            let expected = -2.5 + codes[i] as f32 * 0.125;
            assert!((*v - expected).abs() <= 1e-6, "i={i}");
        }
    }

    #[test]
    fn decompress_respects_output_len() {
        let codes: Vec<u8> = (0..16).collect();
        let mut out = vec![42.0; 8];
        decompress_block(&codes, 1.0, 0.5, &mut out);
        for (i, v) in out.iter().enumerate() {
            let expected = 1.0 + codes[i] as f32 * 0.5;
            assert!((*v - expected).abs() <= 1e-6, "i={i}");
        }
    }
}
