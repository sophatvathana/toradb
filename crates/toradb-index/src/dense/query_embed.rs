use std::hash::{Hash, Hasher};

use crate::sparse::bm25::tokenize;

/// Deterministic bag-of-terms hash embedding for demos when no external embedder is provided.
pub fn lexical_proxy_vector(query: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    if dim == 0 {
        return v;
    }
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return v;
    }
    let scale = 1.0 / (tokens.len() as f32).sqrt();
    for t in tokens {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        t.hash(&mut h);
        let idx = (h.finish() as usize) % dim;
        v[idx] += scale;
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_vector_has_expected_dim_and_unit_norm() {
        let v = lexical_proxy_vector("tesla coil resonant", 4);
        assert_eq!(v.len(), 4);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5 || norm < 1e-8);
    }
}
