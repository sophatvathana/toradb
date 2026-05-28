use std::sync::OnceLock;

use crate::kernels;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdLevel {
    Scalar,
    Avx2,
    Avx512,
    Neon,
}

pub type DotF32Fn = fn(&[f32], &[f32]) -> f32;
pub type DecompressBlockFn = fn(&[u8], f32, f32, &mut [f32]);
pub type PopcntU64Fn = fn(u64) -> u32;
pub type PopcntSliceU64Fn = fn(&[u64]) -> u64;

#[derive(Clone, Copy)]
pub struct DispatchTable {
    pub level: SimdLevel,
    pub dot_f32: DotF32Fn,
    pub decompress_block: DecompressBlockFn,
    pub popcnt_u64: PopcntU64Fn,
    pub popcnt_slice_u64: PopcntSliceU64Fn,
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
    #[cfg(not(all(target_arch = "aarch64", feature = "neon")))]
    {
        SimdLevel::Scalar
    }
}

fn scalar_table() -> DispatchTable {
    DispatchTable {
        level: SimdLevel::Scalar,
        dot_f32: kernels::scalar::dot_f32,
        decompress_block: kernels::scalar::decompress_block,
        popcnt_u64: kernels::scalar::popcnt_u64,
        popcnt_slice_u64: kernels::scalar::popcnt_slice_u64,
    }
}

#[cfg(all(target_arch = "x86_64", feature = "avx512"))]
fn dot_x86_avx512(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: selected only after runtime CPU feature check.
    unsafe { kernels::x86::dot_f32_avx512(a, b) }
}

#[cfg(all(target_arch = "x86_64", feature = "avx512"))]
fn decompress_x86_avx512(codes: &[u8], min: f32, scale: f32, out: &mut [f32]) {
    // SAFETY: selected only after runtime CPU feature check.
    unsafe { kernels::x86::decompress_block_avx512(codes, min, scale, out) }
}

#[cfg(all(target_arch = "x86_64", feature = "avx2"))]
fn dot_x86_avx2(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: selected only after runtime CPU feature check.
    unsafe { kernels::x86::dot_f32_avx2(a, b) }
}

#[cfg(all(target_arch = "x86_64", feature = "avx2"))]
fn decompress_x86_avx2(codes: &[u8], min: f32, scale: f32, out: &mut [f32]) {
    // SAFETY: selected only after runtime CPU feature check.
    unsafe { kernels::x86::decompress_block_avx2(codes, min, scale, out) }
}

#[cfg(all(target_arch = "aarch64", feature = "neon"))]
fn dot_aarch64_neon(a: &[f32], b: &[f32]) -> f32 {
    // SAFETY: NEON is baseline on aarch64; this path is cfg-gated.
    unsafe { kernels::aarch64::dot_f32_neon(a, b) }
}

fn build_table() -> DispatchTable {
    #[allow(unused_mut)]
    let mut t = scalar_table();
    match detect() {
        #[cfg(all(target_arch = "x86_64", feature = "avx512"))]
        SimdLevel::Avx512 => {
            t.level = SimdLevel::Avx512;
            t.dot_f32 = dot_x86_avx512;
            t.decompress_block = decompress_x86_avx512;
        }
        #[cfg(all(target_arch = "x86_64", feature = "avx2"))]
        SimdLevel::Avx2 => {
            t.level = SimdLevel::Avx2;
            t.dot_f32 = dot_x86_avx2;
            t.decompress_block = decompress_x86_avx2;
        }
        #[cfg(all(target_arch = "aarch64", feature = "neon"))]
        SimdLevel::Neon => {
            t.level = SimdLevel::Neon;
            t.dot_f32 = dot_aarch64_neon;
        }
        _ => {}
    }
    t
}

static DISPATCH_TABLE: OnceLock<DispatchTable> = OnceLock::new();

pub fn table() -> &'static DispatchTable {
    DISPATCH_TABLE.get_or_init(build_table)
}
