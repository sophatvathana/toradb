use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::{ast::Stmt, parse};
use toradb_storage::columnar::{read_segment, TableManifestFile};
use toradb_storage::compaction::{CompactMode, CompactPolicy};

#[test]
fn compact_table_merges_small_segments() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let mut dag = DagRunner::open(base).unwrap();
    for i in 0..4 {
        dag.add_documents(
            "docs",
            vec![IngestDoc {
                text: format!("doc {i}"),
                metadata: Default::default(),
                vector: None,
            }],
        )
        .unwrap();
    }
    let manifest_path = TableManifestFile::path_for_table(base, "docs");
    let before = TableManifestFile::load(&manifest_path).unwrap();
    assert!(before.segments.len() >= 2);

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        max_segments: 2,
    };
    let report = persist::compact_table(
        base,
        "docs",
        Some(&mut dag.retrieval.store),
        CompactMode::Normal,
        &policy,
        None,
    )
    .unwrap();
    assert!(report.merges >= 1);
    let after = TableManifestFile::load(&manifest_path).unwrap();
    assert!(after.segments.len() < before.segments.len());
}

#[test]
fn compact_table_sql() {
    let dir = tempfile::tempdir().unwrap();
    let mut dag = DagRunner::open(dir.path()).unwrap();
    dag.add_documents(
        "t",
        vec![IngestDoc {
            text: "a".into(),
            metadata: Default::default(),
            vector: None,
        }],
    )
    .unwrap();
    dag.add_documents(
        "t",
        vec![IngestDoc {
            text: "b".into(),
            metadata: Default::default(),
            vector: None,
        }],
    )
    .unwrap();
    let report = dag.compact_table("t", true).unwrap();
    assert!(report.segments_after <= report.segments_before);
}

#[test]
fn parse_compact_table() {
    let stmts = parse("COMPACT TABLE docs FULL").unwrap();
    let Stmt::CompactTable { table, full } = &stmts[0] else {
        panic!("expected compact");
    };
    assert_eq!(table, "docs");
    assert!(full);
}

#[test]
fn compact_preserves_documents() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let mut dag = DagRunner::open(base).unwrap();
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "one".into(),
                metadata: Default::default(),
                vector: None,
            },
            IngestDoc {
                text: "two".into(),
                metadata: Default::default(),
                vector: None,
            },
        ],
    )
    .unwrap();
    dag.add_documents(
        "docs",
        vec![IngestDoc {
            text: "three".into(),
            metadata: Default::default(),
            vector: None,
        }],
    )
    .unwrap();
    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        ..Default::default()
    };
    persist::compact_table(
        base,
        "docs",
        Some(&mut dag.retrieval.store),
        CompactMode::Full,
        &policy,
        None,
    )
    .unwrap();
    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, "docs")).unwrap();
    assert_eq!(manifest.segments.len(), 1);
    let seg_dir = TableManifestFile::segments_dir(base, "docs");
    let docs = read_segment(&seg_dir.join(&manifest.segments[0])).unwrap();
    assert_eq!(docs.len(), 3);
}
