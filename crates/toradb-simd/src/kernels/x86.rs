#[cfg(feature = "avx2")]
use std::arch::x86_64::{
    __m256, _mm256_add_ps, _mm256_fmadd_ps, _mm256_loadu_ps, _mm256_mul_ps, _mm256_set1_ps,
    _mm256_set_ps, _mm256_setzero_ps, _mm256_storeu_ps,
};
#[cfg(feature = "avx512")]
use std::arch::x86_64::{
    __m512, _mm512_add_ps, _mm512_loadu_ps, _mm512_mul_ps, _mm512_set1_ps, _mm512_set_ps,
    _mm512_setzero_ps, _mm512_storeu_ps,
};

#[cfg(feature = "avx2")]
#[target_feature(enable = "avx2")]
pub unsafe fn dot_f32_avx2(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut i = 0usize;
    let mut acc: __m256 = _mm256_setzero_ps();
    while i + 8 <= n {
        // SAFETY: i + 8 <= n and pointers are valid for 8 f32 values.
        let va = unsafe { _mm256_loadu_ps(a.as_ptr().add(i)) };
        // SAFETY: i + 8 <= n and pointers are valid for 8 f32 values.
        let vb = unsafe { _mm256_loadu_ps(b.as_ptr().add(i)) };
        acc = _mm256_add_ps(acc, _mm256_mul_ps(va, vb));
        i += 8;
    }

    let mut tmp = [0f32; 8];
    // SAFETY: tmp is valid for 8 f32 values.
    unsafe { _mm256_storeu_ps(tmp.as_mut_ptr(), acc) };
    let mut sum: f32 = tmp.iter().sum();
    while i < n {
        sum += a[i] * b[i];
        i += 1;
    }
    sum
}

#[cfg(feature = "avx2")]
#[target_feature(enable = "avx2")]
pub unsafe fn decompress_block_avx2(codes: &[u8], min: f32, scale: f32, out: &mut [f32]) {
    let n = codes.len().min(out.len());
    let base = _mm256_set1_ps(min);
    let step = _mm256_set1_ps(scale);
    let mut i = 0usize;
    while i + 8 <= n {
        let c = _mm256_set_ps(
            codes[i + 7] as f32,
            codes[i + 6] as f32,
            codes[i + 5] as f32,
            codes[i + 4] as f32,
            codes[i + 3] as f32,
            codes[i + 2] as f32,
            codes[i + 1] as f32,
            codes[i] as f32,
        );
        let y = _mm256_add_ps(base, _mm256_mul_ps(c, step));
        // SAFETY: i + 8 <= n and out has at least n elements.
        unsafe { _mm256_storeu_ps(out.as_mut_ptr().add(i), y) };
        i += 8;
    }
    while i < n {
        out[i] = min + codes[i] as f32 * scale;
        i += 1;
    }
}

/// AVX2 ADC dot for 4-bit MSE codes. Processes 8 dims per iter (4 bytes of codes).
/// `codebook.len()` must be 16.
#[cfg(feature = "avx2")]
#[target_feature(enable = "avx2,fma")]
pub unsafe fn tq_adc_mse_4bit_avx2(query: &[f32], codes: &[u8], codebook: &[f32]) -> f32 {
    debug_assert_eq!(codebook.len(), 16);
    let cb = codebook.as_ptr();
    let n = query.len();
    let mut acc: __m256 = _mm256_setzero_ps();
    let mut i = 0usize;
    while i + 8 <= n {
        let b0 = *codes.get_unchecked(i >> 1);
        let b1 = *codes.get_unchecked((i >> 1) + 1);
        let b2 = *codes.get_unchecked((i >> 1) + 2);
        let b3 = *codes.get_unchecked((i >> 1) + 3);
        // Lane order matches _mm256_loadu_ps (low at index 0).
        let v = _mm256_set_ps(
            *cb.add(((b3 >> 4) & 0xF) as usize),
            *cb.add((b3 & 0xF) as usize),
            *cb.add(((b2 >> 4) & 0xF) as usize),
            *cb.add((b2 & 0xF) as usize),
            *cb.add(((b1 >> 4) & 0xF) as usize),
            *cb.add((b1 & 0xF) as usize),
            *cb.add(((b0 >> 4) & 0xF) as usize),
            *cb.add((b0 & 0xF) as usize),
        );
        let q = _mm256_loadu_ps(query.as_ptr().add(i));
        acc = _mm256_fmadd_ps(q, v, acc);
        i += 8;
    }
    let mut tmp = [0f32; 8];
    _mm256_storeu_ps(tmp.as_mut_ptr(), acc);
    let mut sum: f32 = tmp.iter().sum();
    // Tail: scalar handles whatever's left.
    while i < n {
        let b = *codes.get_unchecked(i >> 1);
        let c = ((b >> ((i & 1) * 4)) & 0xF) as usize;
        sum += query[i] * *cb.add(c);
        i += 1;
    }
    sum
}

#[cfg(feature = "avx512")]
#[target_feature(enable = "avx512f")]
pub unsafe fn decompress_block_avx512(codes: &[u8], min: f32, scale: f32, out: &mut [f32]) {
    let n = codes.len().min(out.len());
    let base = _mm512_set1_ps(min);
    let step = _mm512_set1_ps(scale);
    let mut i = 0usize;
    while i + 16 <= n {
        let c = _mm512_set_ps(
            codes[i + 15] as f32,
            codes[i + 14] as f32,
            codes[i + 13] as f32,
            codes[i + 12] as f32,
            codes[i + 11] as f32,
            codes[i + 10] as f32,
            codes[i + 9] as f32,
            codes[i + 8] as f32,
            codes[i + 7] as f32,
            codes[i + 6] as f32,
            codes[i + 5] as f32,
            codes[i + 4] as f32,
            codes[i + 3] as f32,
            codes[i + 2] as f32,
            codes[i + 1] as f32,
            codes[i] as f32,
        );
        let y = _mm512_add_ps(base, _mm512_mul_ps(c, step));
        // SAFETY: i + 16 <= n and out has at least n elements.
        unsafe { _mm512_storeu_ps(out.as_mut_ptr().add(i), y) };
        i += 16;
    }
    while i < n {
        out[i] = min + codes[i] as f32 * scale;
        i += 1;
    }
}

#[cfg(feature = "avx512")]
#[target_feature(enable = "avx512f")]
pub unsafe fn dot_f32_avx512(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut i = 0usize;
    let mut acc: __m512 = _mm512_setzero_ps();
    while i + 16 <= n {
        // SAFETY: i + 16 <= n and pointers are valid for 16 f32 values.
        let va = unsafe { _mm512_loadu_ps(a.as_ptr().add(i)) };
        // SAFETY: i + 16 <= n and pointers are valid for 16 f32 values.
        let vb = unsafe { _mm512_loadu_ps(b.as_ptr().add(i)) };
        acc = _mm512_add_ps(acc, _mm512_mul_ps(va, vb));
        i += 16;
    }

    let mut tmp = [0f32; 16];
    // SAFETY: tmp is valid for 16 f32 values.
    unsafe { _mm512_storeu_ps(tmp.as_mut_ptr(), acc) };
    let mut sum: f32 = tmp.iter().sum();
    while i < n {
        sum += a[i] * b[i];
        i += 1;
    }
    sum
}
