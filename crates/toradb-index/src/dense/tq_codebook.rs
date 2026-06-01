//! Precomputed Lloyd–Max scalar codebooks for TurboQuant.
pub fn codebook(bits: u8) -> &'static [f32] {
    match bits {
        1 => &CODEBOOK_1,
        2 => &CODEBOOK_2,
        3 => &CODEBOOK_3,
        4 => &CODEBOOK_4,
        _ => panic!("turboquant bits must be 1..=4"),
    }
}

/// Quantize a single value `x` (assumed N(0,1)) to the index of the nearest
/// centroid in `codebook(bits)`.
pub fn quantize(x: f32, bits: u8) -> u8 {
    let cb = codebook(bits);
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (i, &c) in cb.iter().enumerate() {
        let d = (x - c).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best as u8
}

/// Theoretical MSE per dimension (D = E[(X - Q(X))^2]) for unit-variance
/// Gaussian source. Source: Lloyd–Max tables.
pub fn theoretical_mse(bits: u8) -> f32 {
    match bits {
        1 => 0.3634,
        2 => 0.1175,
        3 => 0.03454,
        4 => 0.00950,
        _ => panic!("turboquant bits must be 1..=4"),
    }
}

// Lloyd–Max centroids for unit-variance Gaussian.
// 1 bit: ±sqrt(2/pi)
static CODEBOOK_1: [f32; 2] = [-0.7979, 0.7979];

// 2 bits
static CODEBOOK_2: [f32; 4] = [-1.5104, -0.4528, 0.4528, 1.5104];

// 3 bits (8 centroids)
static CODEBOOK_3: [f32; 8] = [
    -2.1519, -1.3439, -0.7560, -0.2451, 0.2451, 0.7560, 1.3439, 2.1519,
];

// 4 bits (16 centroids)
static CODEBOOK_4: [f32; 16] = [
    -2.7326, -2.0690, -1.6180, -1.2562, -0.9423, -0.6568, -0.3880, -0.1284, 0.1284, 0.3880, 0.6568,
    0.9423, 1.2562, 1.6180, 2.0690, 2.7326,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codebooks_sizes() {
        assert_eq!(codebook(1).len(), 2);
        assert_eq!(codebook(2).len(), 4);
        assert_eq!(codebook(3).len(), 8);
        assert_eq!(codebook(4).len(), 16);
    }

    #[test]
    fn codebooks_sorted() {
        for b in [1u8, 2, 3, 4] {
            let cb = codebook(b);
            for w in cb.windows(2) {
                assert!(w[0] < w[1], "bits={b} not sorted");
            }
        }
    }

    #[test]
    fn quantize_picks_nearest() {
        assert_eq!(quantize(-2.0, 2), 0);
        assert_eq!(quantize(-0.4, 2), 1);
        assert_eq!(quantize(0.5, 2), 2);
        assert_eq!(quantize(2.0, 2), 3);
    }
}
