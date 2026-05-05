#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    Scalar,
    Avx2,
    Avx512,
    Neon,
}

pub fn detect() -> SimdLevel {
    #[cfg(all(target_arch = "x86_64", feature = "avx512"))]
    {
        if std::arch::is_x86_feature_detected!("avx512f") {
            return SimdLevel::Avx512;
        }
    }
    #[cfg(all(target_arch = "x86_64", feature = "avx2"))]
    {
        if std::arch::is_x86_feature_detected!("avx2") {
            return SimdLevel::Avx2;
        }
    }
    #[cfg(all(target_arch = "aarch64", feature = "neon"))]
    {
        return SimdLevel::Neon;
    }
    SimdLevel::Scalar
}
