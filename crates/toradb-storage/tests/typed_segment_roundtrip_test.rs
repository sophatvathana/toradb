use std::collections::HashMap;

use toradb_core::{ColumnType, ColumnTypeSpec};
use toradb_storage::columnar::{
    read_segment, write_segment_with_compression, ColumnarDoc, TableManifestFile,
};

#[test]
fn native_int_column_roundtrip_and_overflow_json() {
    let dir = tempfile::tempdir().unwrap();
    let table_dir = dir.path().join("docs");
    std::fs::create_dir_all(&table_dir).unwrap();
    let mut manifest = TableManifestFile::default();
    manifest.set_column_types(vec![
        ("rank".to_string(), ColumnTypeSpec::new(ColumnType::Int)),
        ("tag".to_string(), ColumnTypeSpec::new(ColumnType::Text)),
    ]);
    manifest.save(&table_dir.join("manifest.json")).unwrap();

    let path = table_dir.join("segments/seg_00001.parquet");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut meta = HashMap::new();
    meta.insert("rank".to_string(), "10".to_string());
    meta.insert("tag".to_string(), "news".to_string());
    meta.insert("extra".to_string(), "overflow".to_string());
    write_segment_with_compression(
        &path,
        &[ColumnarDoc {
            id: 0,
            text: "hello".into(),
            metadata: meta,
            embedding: None,
        }],
        None,
        &manifest.column_types,
    )
    .unwrap();

    let docs = read_segment(&path).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].metadata.get("rank").map(String::as_str), Some("10"));
    assert_eq!(
        docs[0].metadata.get("tag").map(String::as_str),
        Some("news")
    );
    assert_eq!(
        docs[0].metadata.get("extra").map(String::as_str),
        Some("overflow")
    );
}

#[test]
fn legacy_segment_without_typed_columns_still_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.parquet");
    let mut meta = HashMap::new();
    meta.insert("rank".to_string(), "9".to_string());
    write_segment_with_compression(
        &path,
        &[ColumnarDoc {
            id: 1,
            text: "x".into(),
            metadata: meta.clone(),
            embedding: None,
        }],
        None,
        &[],
    )
    .unwrap();

    let docs = read_segment(&path).unwrap();
    assert_eq!(docs[0].metadata.get("rank").map(String::as_str), Some("9"));
}
