use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use toradb_simd::fht;

fn make_vec(n: usize, k: f32) -> Vec<f32> {
    (0..n).map(|i| (i as f32 * k).sin()).collect()
}

fn bench_fht(c: &mut Criterion) {
    let mut group = c.benchmark_group("fht");
    group.warm_up_time(std::time::Duration::from_millis(400));
    group.measurement_time(std::time::Duration::from_secs(2));
    for dim in [128usize, 256, 512, 1024, 2048] {
        let v0 = make_vec(dim, 0.0031);
        group.throughput(Throughput::Elements(dim as u64));
        group.bench_with_input(BenchmarkId::from_parameter(dim), &dim, |bencher, _| {
            bencher.iter(|| {
                let mut v = v0.clone();
                fht::apply_rotation(black_box(0xABCD_1234_DEAD_BEEF), black_box(&mut v));
                black_box(v);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_fht);
criterion_main!(benches);
