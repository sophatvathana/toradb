use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use toradb_simd::tq_adc;

fn make_query(n: usize) -> Vec<f32> {
    (0..n).map(|i| ((i as f32) * 0.0123).sin()).collect()
}

fn bench_adc_mse(c: &mut Criterion) {
    let mut group = c.benchmark_group("tq_adc_mse");
    group.warm_up_time(std::time::Duration::from_millis(400));
    group.measurement_time(std::time::Duration::from_secs(2));
    for &dim in &[128usize, 256, 512, 1024] {
        for &bits in &[1u8, 2, 4] {
            let codebook: Vec<f32> = (0..1u32 << bits)
                .map(|i| (i as f32) / (1u32 << bits) as f32 - 0.5)
                .collect();
            let raw: Vec<u8> = (0..dim).map(|i| (i % (1usize << bits)) as u8).collect();
            let codes = tq_adc::encode_codes(&raw, bits);
            let q = make_query(dim);
            group.throughput(Throughput::Elements(dim as u64));
            let label = format!("d{dim}_b{bits}");
            group.bench_with_input(BenchmarkId::from_parameter(label), &dim, |bencher, _| {
                bencher.iter(|| {
                    black_box(tq_adc::tq_adc_mse(
                        black_box(&q),
                        black_box(&codes),
                        black_box(bits),
                        black_box(&codebook),
                    ))
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, bench_adc_mse);
criterion_main!(benches);
