use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use toradb_simd::dot_f32;

fn make_vec(n: usize, k: f32) -> Vec<f32> {
    (0..n).map(|i| (i as f32 * k).sin()).collect()
}

fn bench_dot(c: &mut Criterion) {
    let mut group = c.benchmark_group("dot_f32");
    group.warm_up_time(std::time::Duration::from_millis(400));
    group.measurement_time(std::time::Duration::from_secs(2));
    for dim in [128usize, 256, 384, 768, 1024] {
        let a = make_vec(dim, 0.0031);
        let b = make_vec(dim, 0.0073);
        group.throughput(Throughput::Elements(dim as u64));
        group.bench_with_input(BenchmarkId::from_parameter(dim), &dim, |bencher, _| {
            bencher.iter(|| black_box(dot_f32(black_box(&a), black_box(&b))));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_dot);
criterion_main!(benches);
