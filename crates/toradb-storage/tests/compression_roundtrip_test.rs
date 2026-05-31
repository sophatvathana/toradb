use toradb_core::CompressionConfig;
use toradb_storage::columnar::{read_segment, write_segment_with_compression, ColumnarDoc};

#[test]
fn compressed_parquet_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seg.parquet");
    let cfg = CompressionConfig {
        enabled: true,
        block_size: 4096,
    };
    let docs = vec![ColumnarDoc {
        id: 1,
        text: "hello compressed".into(),
        metadata: Default::default(),
        embedding: Some(vec![0.1, 0.2, 0.3]),
    }];
    write_segment_with_compression(&path, &docs, Some(&cfg), &[]).expect("write");
    let out = read_segment(&path).expect("read");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].text, "hello compressed");
}
