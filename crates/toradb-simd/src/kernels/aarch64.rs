#[cfg(feature = "neon")]
use std::arch::aarch64::{float32x4_t, vdupq_n_f32, vfmaq_f32, vld1q_f32, vst1q_f32};

#[cfg(feature = "neon")]
#[target_feature(enable = "neon")]
pub unsafe fn dot_f32_neon(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut i = 0usize;
    let mut acc: float32x4_t = vdupq_n_f32(0.0);
    while i + 4 <= n {
        // SAFETY: i + 4 <= n and pointers are valid for 4 f32 values.
        let va = unsafe { vld1q_f32(a.as_ptr().add(i)) };
        // SAFETY: i + 4 <= n and pointers are valid for 4 f32 values.
        let vb = unsafe { vld1q_f32(b.as_ptr().add(i)) };
        acc = vfmaq_f32(acc, va, vb);
        i += 4;
    }

    let mut tmp = [0f32; 4];
    // SAFETY: tmp is valid for 4 f32 values.
    unsafe { vst1q_f32(tmp.as_mut_ptr(), acc) };
    let mut sum: f32 = tmp.iter().sum();
    while i < n {
        sum += a[i] * b[i];
        i += 1;
    }
    sum
}
