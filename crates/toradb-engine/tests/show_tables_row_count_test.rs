use std::path::Path;

use toradb_engine::persist;
use toradb_index::{CorpusStore, IngestDoc};
use toradb_storage::columnar::{write_segment_with_compression, ColumnarDoc, TableManifestFile};

#[test]
fn table_row_count_from_segment_id_ranges_without_parquet_scan() {
    let dir = std::env::temp_dir().join(format!("toradb_row_count_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let table = "docs";
    let seg_dir = TableManifestFile::segments_dir(&dir, table);
    std::fs::create_dir_all(&seg_dir).unwrap();

    let docs: Vec<ColumnarDoc> = (0..50)
        .map(|i| ColumnarDoc {
            id: i,
            text: format!("doc {i}"),
            metadata: Default::default(),
            embedding: None,
        })
        .collect();
    let seg_name = "seg_00001.parquet";
    write_segment_with_compression(&seg_dir.join(seg_name), &docs, None, &[]).unwrap();

    let mut manifest = TableManifestFile::default();
    manifest.segments.push(seg_name.to_string());
    manifest.record_segment_id_range(seg_name, 0, 49);
    manifest
        .save(&TableManifestFile::path_for_table(&dir, table))
        .unwrap();

    let store = CorpusStore::default();
    let n = persist::table_row_count_on_disk(&store, &dir, table).expect("count");
    assert_eq!(n, 50);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn table_row_count_prefers_in_memory_corpus() {
    let dir = std::env::temp_dir().join(format!("toradb_row_count_mem_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let table = "tiny";
    let seg_dir = TableManifestFile::segments_dir(&dir, table);
    std::fs::create_dir_all(&seg_dir).unwrap();
    let docs = vec![ColumnarDoc {
        id: 1,
        text: "one".into(),
        metadata: Default::default(),
        embedding: None,
    }];
    write_segment_with_compression(&seg_dir.join("seg_00001.parquet"), &docs, None, &[]).unwrap();
    let mut manifest = TableManifestFile::default();
    manifest.segments.push("seg_00001.parquet".into());
    manifest.record_segment_id_range("seg_00001.parquet", 1, 1);
    manifest
        .save(&TableManifestFile::path_for_table(&dir, table))
        .unwrap();

    let mut store = CorpusStore::default();
    store.add_documents(
        table,
        vec![IngestDoc {
            text: "ram".into(),
            metadata: Default::default(),
            vector: None,
        }],
        1,
        toradb_core::IngestOptions::default(),
    );

    let n = persist::table_row_count_on_disk(&store, Path::new(&dir), table).expect("count");
    assert_eq!(n, 1);

    let _ = std::fs::remove_dir_all(&dir);
}
