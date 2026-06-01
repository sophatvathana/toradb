//! Seeded Fast Hadamard Transform (Subsampled Randomized Hadamard Transform).

pub fn next_pow2(n: usize) -> usize {
    if n <= 1 {
        return 1;
    }
    n.next_power_of_two()
}

#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// Fill `signs` with deterministic ±1.0 derived from `seed`.
pub fn rademacher_signs(seed: u64, signs: &mut [f32]) {
    let mut state = seed;
    let mut i = 0;
    while i < signs.len() {
        let word = splitmix64(&mut state);
        let mut bits = word;
        let take = (signs.len() - i).min(64);
        for k in 0..take {
            let s = if (bits & 1) == 1 { 1.0 } else { -1.0 };
            signs[i + k] = s;
            bits >>= 1;
        }
        i += take;
    }
}

/// In-place Walsh–Hadamard transform (no scaling). `v.len()` must be a power of two.
pub fn walsh_hadamard(v: &mut [f32]) {
    let n = v.len();
    debug_assert!(n.is_power_of_two() && n > 0);
    let mut h = 1usize;
    while h < n {
        let mut i = 0usize;
        while i < n {
            for j in i..i + h {
                let x = v[j];
                let y = v[j + h];
                v[j] = x + y;
                v[j + h] = x - y;
            }
            i += h * 2;
        }
        h *= 2;
    }
}

/// Apply `(1/sqrt(d)) * H * D` in-place. `v.len()` must be a power of two.
pub fn apply_rotation(seed: u64, v: &mut [f32]) {
    let n = v.len();
    debug_assert!(n.is_power_of_two() && n > 0);
    let mut signs = vec![0.0f32; n];
    rademacher_signs(seed, &mut signs);
    for i in 0..n {
        v[i] *= signs[i];
    }
    walsh_hadamard(v);
    let inv = 1.0f32 / (n as f32).sqrt();
    for x in v.iter_mut() {
        *x *= inv;
    }
}

/// Apply the inverse rotation: `D^T * (1/sqrt(d)) * H^T = D * (1/sqrt(d)) * H`.
/// Since `H` is symmetric and `D = D^T`, this is `walsh_hadamard` then sign-flip
/// then scale.
pub fn apply_rotation_inverse(seed: u64, v: &mut [f32]) {
    let n = v.len();
    debug_assert!(n.is_power_of_two() && n > 0);
    walsh_hadamard(v);
    let inv = 1.0f32 / (n as f32).sqrt();
    let mut signs = vec![0.0f32; n];
    rademacher_signs(seed, &mut signs);
    for i in 0..n {
        v[i] = v[i] * signs[i] * inv;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_is_involutive_within_eps() {
        for &n in &[2usize, 4, 8, 16, 32, 64, 128, 256, 1024] {
            let original: Vec<f32> = (0..n).map(|i| (i as f32 * 0.37) - 3.1).collect();
            let mut v = original.clone();
            apply_rotation(0xDEAD_BEEF_CAFE_F00D, &mut v);
            apply_rotation_inverse(0xDEAD_BEEF_CAFE_F00D, &mut v);
            for (i, (a, b)) in original.iter().zip(v.iter()).enumerate() {
                assert!((a - b).abs() < 1e-3, "n={n} i={i} {a} vs {b}");
            }
        }
    }

    #[test]
    fn rotation_preserves_norm() {
        let n = 256;
        let v0: Vec<f32> = (0..n).map(|i| (i as f32 * 0.13).sin()).collect();
        let n2: f32 = v0.iter().map(|x| x * x).sum();
        let mut v = v0.clone();
        apply_rotation(42, &mut v);
        let n2_rot: f32 = v.iter().map(|x| x * x).sum();
        assert!((n2 - n2_rot).abs() / n2 < 1e-4, "{n2} vs {n2_rot}");
    }

    #[test]
    fn rotation_preserves_inner_product() {
        let n = 128;
        let a0: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.21).cos()).collect();
        let b0: Vec<f32> = (0..n).map(|i| ((i as f32) * 0.07).sin() + 0.5).collect();
        let ip0: f32 = a0.iter().zip(b0.iter()).map(|(x, y)| x * y).sum();
        let mut a = a0.clone();
        let mut b = b0.clone();
        apply_rotation(7, &mut a);
        apply_rotation(7, &mut b);
        let ip1: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        assert!(
            (ip0 - ip1).abs() / ip0.abs().max(1e-6) < 1e-3,
            "{ip0} vs {ip1}"
        );
    }

    #[test]
    fn next_pow2_basic() {
        assert_eq!(next_pow2(0), 1);
        assert_eq!(next_pow2(1), 1);
        assert_eq!(next_pow2(2), 2);
        assert_eq!(next_pow2(3), 4);
        assert_eq!(next_pow2(1000), 1024);
        assert_eq!(next_pow2(1024), 1024);
    }
}
