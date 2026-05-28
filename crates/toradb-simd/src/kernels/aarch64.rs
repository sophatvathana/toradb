#[cfg(feature = "neon")]
use std::arch::aarch64::{
    float32x4_t, vaddvq_f32, vdupq_n_f32, vfmaq_f32, vld1q_f32, vsetq_lane_f32, vst1q_f32,
};

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

/// NEON ADC dot for 4-bit MSE codes. Processes 8 dims per iter (4 bytes of codes).
#[cfg(feature = "neon")]
#[target_feature(enable = "neon")]
pub unsafe fn tq_adc_mse_4bit_neon(query: &[f32], codes: &[u8], codebook: &[f32]) -> f32 {
    debug_assert_eq!(codebook.len(), 16);
    let cb = codebook.as_ptr();
    let n = query.len();
    let mut acc_lo: float32x4_t = vdupq_n_f32(0.0);
    let mut acc_hi: float32x4_t = vdupq_n_f32(0.0);
    let mut i = 0usize;
    while i + 8 <= n {
        let b0 = *codes.get_unchecked(i >> 1);
        let b1 = *codes.get_unchecked((i >> 1) + 1);
        let b2 = *codes.get_unchecked((i >> 1) + 2);
        let b3 = *codes.get_unchecked((i >> 1) + 3);

        let mut v_lo = vdupq_n_f32(0.0);
        v_lo = vsetq_lane_f32(*cb.add((b0 & 0xF) as usize), v_lo, 0);
        v_lo = vsetq_lane_f32(*cb.add(((b0 >> 4) & 0xF) as usize), v_lo, 1);
        v_lo = vsetq_lane_f32(*cb.add((b1 & 0xF) as usize), v_lo, 2);
        v_lo = vsetq_lane_f32(*cb.add(((b1 >> 4) & 0xF) as usize), v_lo, 3);

        let mut v_hi = vdupq_n_f32(0.0);
        v_hi = vsetq_lane_f32(*cb.add((b2 & 0xF) as usize), v_hi, 0);
        v_hi = vsetq_lane_f32(*cb.add(((b2 >> 4) & 0xF) as usize), v_hi, 1);
        v_hi = vsetq_lane_f32(*cb.add((b3 & 0xF) as usize), v_hi, 2);
        v_hi = vsetq_lane_f32(*cb.add(((b3 >> 4) & 0xF) as usize), v_hi, 3);

        let q_lo = vld1q_f32(query.as_ptr().add(i));
        let q_hi = vld1q_f32(query.as_ptr().add(i + 4));
        acc_lo = vfmaq_f32(acc_lo, q_lo, v_lo);
        acc_hi = vfmaq_f32(acc_hi, q_hi, v_hi);
        i += 8;
    }
    let mut sum = vaddvq_f32(acc_lo) + vaddvq_f32(acc_hi);
    while i < n {
        let b = *codes.get_unchecked(i >> 1);
        let c = ((b >> ((i & 1) * 4)) & 0xF) as usize;
        sum += query[i] * *cb.add(c);
        i += 1;
    }
    sum
}
