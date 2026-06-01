use criterion::{black_box, criterion_group, criterion_main, Criterion};
use toradb_storage::cache::{read_segment_cached, StorageCaches};
use toradb_storage::columnar::{read_segment, write_segment, ColumnarDoc};

fn bench_segment_read(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seg.parquet");
    write_segment(
        &path,
        &[ColumnarDoc {
            id: 1,
            text: "bench".into(),
            metadata: Default::default(),
            embedding: None,
        }],
    )
    .unwrap();
    let mut group = c.benchmark_group("segment_read");
    group.bench_function("cold", |b| {
        b.iter(|| read_segment(black_box(&path)).unwrap());
    });
    group.bench_function("cached", |b| {
        let mut caches = StorageCaches::default_from_env();
        b.iter(|| read_segment_cached(black_box(&path), Some(&mut caches)).unwrap());
    });
    group.finish();
}

criterion_group!(benches, bench_segment_read);
criterion_main!(benches);
