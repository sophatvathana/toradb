#[cfg(all(feature = "io-uring", target_os = "linux"))]
#[test]
fn io_uring_reads_segment_bytes() {
    use toradb_storage::columnar::{read_segment_io_uring, write_segment, ColumnarDoc};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("seg.parquet");
    write_segment(
        &path,
        &[ColumnarDoc {
            id: 7,
            text: "io".into(),
            metadata: Default::default(),
            embedding: None,
        }],
    )
    .unwrap();
    let docs = read_segment_io_uring(&path).expect("io_uring read");
    assert_eq!(docs[0].id, 7);
}
