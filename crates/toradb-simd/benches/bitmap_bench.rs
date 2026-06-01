use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use toradb_simd::bitmap::popcnt_slice_u64;

fn bench_popcnt(c: &mut Criterion) {
    let mut group = c.benchmark_group("popcnt_slice_u64");
    group.warm_up_time(std::time::Duration::from_millis(400));
    group.measurement_time(std::time::Duration::from_secs(2));
    for n in [256usize, 1024, 4096, 16384] {
        let words: Vec<u64> = (0..n)
            .map(|i| (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            .collect();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bencher, _| {
            bencher.iter(|| black_box(popcnt_slice_u64(black_box(&words))));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_popcnt);
criterion_main!(benches);
