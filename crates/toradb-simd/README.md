# toradb-simd

`toradb-simd` provides runtime-dispatched kernels for distance, quantized decompress, and bitmap popcount operations.

## Production feature flags

- `avx2`: enable AVX2 kernels on `x86_64`
- `avx512`: enable AVX-512 kernels on `x86_64`
- `neon`: enable NEON kernels on `aarch64`

Recommended builds:

- Mixed fleet default: no extra features (safe scalar fallback everywhere)
- x86_64 AVX2 baseline:
  - `cargo build -p toradb-simd --features avx2`
- x86_64 AVX-512 capable fleet:
  - `cargo build -p toradb-simd --features "avx2 avx512"`
- ARM64/NEON:
  - `cargo build -p toradb-simd --features neon`

At runtime, `dispatch::detect()` selects the best available kernel and falls back to scalar if unsupported.

## Benchmark proof

Use Criterion baselines to prove performance:

- `cargo bench -p toradb-simd --bench dot_f32_bench -- --save-baseline main`
- `cargo bench -p toradb-simd --bench dot_f32_bench -- --baseline main`

One-command helper:

- `./scripts/bench_simd.sh main`
