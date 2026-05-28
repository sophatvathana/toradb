pub fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    (crate::dispatch::table().dot_f32)(a, b)
}

#[cfg(test)]
mod tests {
    use super::dot_f32;

    fn ref_dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn dot_matches_reference_across_dims() {
        for dim in [0usize, 1, 3, 7, 8, 15, 16, 31, 32, 127, 128, 255, 256] {
            let a: Vec<f32> = (0..dim).map(|i| (i as f32 * 0.37) - 11.0).collect();
            let b: Vec<f32> = (0..dim).map(|i| (i as f32 * -0.11) + 7.0).collect();
            let got = dot_f32(&a, &b);
            let expected = ref_dot(&a, &b);
            assert!((got - expected).abs() <= 5e-2, "dim={dim} got={got} expected={expected}");
        }
    }
}
