#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

BASELINE="${1:-main}"
OUT_DIR="target/criterion"
BENCHES=("dot_f32_bench" "decompress_bench" "bitmap_bench")

echo "Running SIMD benchmark baseline: $BASELINE"
for b in "${BENCHES[@]}"; do
  cargo bench -p toradb-simd --bench "$b" -- --save-baseline "$BASELINE"
done

for b in "${BENCHES[@]}"; do
  cargo bench -p toradb-simd --bench "$b" -- --baseline "$BASELINE"
done

if [ -d "$OUT_DIR" ]; then
  echo "SIMD_BENCH_RESULT=PASS (criterion output in $OUT_DIR)"
else
  echo "SIMD_BENCH_RESULT=FAIL (missing $OUT_DIR)"
  exit 1
fi
