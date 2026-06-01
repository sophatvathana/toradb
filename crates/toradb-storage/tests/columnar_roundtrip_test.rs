use std::collections::HashMap;
use toradb_storage::columnar::{
    read_segment, read_segment_texts, write_segment, ColumnarDoc, TableManifestFile,
};

#[test]
fn parquet_segment_roundtrip() {
    let dir = std::env::temp_dir().join("toradb_columnar_test");
    let _ = std::fs::remove_dir_all(&dir);
    let seg_path = dir.join("segments/seg_00001.parquet");

    let mut meta = HashMap::new();
    meta.insert("tag".into(), "patent".into());

    write_segment(
        &seg_path,
        &[
            ColumnarDoc {
                id: 0,
                text: "Nikola Tesla alternating current motor".into(),
                metadata: meta.clone(),
                embedding: None,
            },
            ColumnarDoc {
                id: 1,
                text: "Marie Curie radioactivity".into(),
                metadata: HashMap::new(),
                embedding: Some(vec![0.1, 0.2, 0.3]),
            },
        ],
    )
    .expect("write");

    let texts = read_segment_texts(&seg_path).expect("read texts");
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0].0, 0);
    assert_eq!(texts[0].1, "Nikola Tesla alternating current motor");

    let docs = read_segment(&seg_path).expect("read");
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].text, "Nikola Tesla alternating current motor");
    assert_eq!(
        docs[0].metadata.get("tag").map(String::as_str),
        Some("patent")
    );
    assert_eq!(
        docs[1].embedding.as_deref(),
        Some([0.1_f32, 0.2, 0.3].as_slice())
    );

    let manifest_path = TableManifestFile::path_for_table(&dir, "papers");
    let mut manifest = TableManifestFile::default();
    manifest.push_segment("seg_00001.parquet".into());
    manifest.save(&manifest_path).expect("save manifest");
    let loaded = TableManifestFile::load(&manifest_path).expect("load manifest");
    assert_eq!(loaded.segments.len(), 1);
}
