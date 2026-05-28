use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use toradb_simd::decompress::decompress_block;

fn bench_decompress(c: &mut Criterion) {
    let mut group = c.benchmark_group("decompress_block");
    group.warm_up_time(std::time::Duration::from_millis(400));
    group.measurement_time(std::time::Duration::from_secs(2));
    for dim in [128usize, 256, 384, 768, 1024] {
        let codes: Vec<u8> = (0..dim).map(|i| (i * 7 % 256) as u8).collect();
        let mut out = vec![0f32; dim];
        group.throughput(Throughput::Elements(dim as u64));
        group.bench_with_input(BenchmarkId::from_parameter(dim), &dim, |bencher, _| {
            bencher.iter(|| {
                decompress_block(black_box(&codes), black_box(-1.25), black_box(0.0125), black_box(&mut out))
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_decompress);
criterion_main!(benches);
