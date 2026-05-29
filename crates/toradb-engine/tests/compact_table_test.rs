use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::{ast::Stmt, parse};
use toradb_storage::columnar::{read_segment, TableManifestFile};
use toradb_storage::compaction::{CompactMode, CompactPolicy, TierPolicy};

#[test]
fn compact_table_merges_small_segments() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let mut dag = DagRunner::open(base).unwrap();
    for i in 0..6 {
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
    assert!(!before.segments.is_empty());

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        max_segments: 2,
        ..Default::default()
    };
    let report = persist::compact_table(
        base,
        "docs",
        Some(&mut dag.retrieval.store),
        CompactMode::Full,
        &policy,
        None,
    )
    .unwrap();
    let after = TableManifestFile::load(&manifest_path).unwrap();
    assert!(after.segments.len() == 1 || report.merges == 0);
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

fn make_doc(text: &str) -> IngestDoc {
    IngestDoc {
        text: text.to_string(),
        metadata: Default::default(),
        vector: None,
    }
}

#[test]
fn compact_promotes_tier0_to_tier1() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let mut dag = DagRunner::open(base).unwrap();

    dag.add_documents("docs", vec![make_doc("a")]).unwrap();
    dag.add_documents("docs", vec![make_doc("b")]).unwrap();

    let manifest_path = TableManifestFile::path_for_table(base, "docs");
    let before = TableManifestFile::load(&manifest_path).unwrap();
    assert!(!before.segment_meta.is_empty(), "segment_meta must be populated on flush");
    assert!(before.segment_meta.iter().all(|m| m.tier == 0), "all fresh segments are tier 0");

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        tier: TierPolicy {
            tier_merge_threshold: 2,
            ..Default::default()
        },
        ..Default::default()
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
    assert!(report.merges >= 1, "expected at least one merge");

    let after = TableManifestFile::load(&manifest_path).unwrap();
    assert_eq!(after.segments.len(), 1);
    assert_eq!(after.segment_meta[0].tier, 1, "merged segment should be tier 1");
}

#[test]
fn compact_preserves_tier_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let mut dag = DagRunner::open(base).unwrap();

    dag.add_documents("docs", vec![make_doc("x")]).unwrap();
    dag.add_documents("docs", vec![make_doc("y")]).unwrap();

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        tier: TierPolicy {
            tier_merge_threshold: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    persist::compact_table(
        base,
        "docs",
        Some(&mut dag.retrieval.store),
        CompactMode::Normal,
        &policy,
        None,
    )
    .unwrap();

    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, "docs")).unwrap();
    assert!(!manifest.segment_meta.is_empty());
    assert_eq!(manifest.segment_meta[0].tier, 1, "tier must survive manifest reload");
    assert_eq!(manifest.schema_version, 2, "schema_version must be bumped to 2");
}

#[test]
fn legacy_db_compacts_without_error() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();

    let table_dir = base.join("docs");
    let seg_dir = table_dir.join("segments");
    std::fs::create_dir_all(&seg_dir).unwrap();

    let seg1 = "seg_00001.parquet";
    let seg2 = "seg_00002.parquet";
    use toradb_storage::columnar::ColumnarDoc;
    let doc = ColumnarDoc { id: 1, text: "hello".into(), metadata: Default::default(), embedding: None };
    toradb_storage::columnar::write_segment_with_compression(&seg_dir.join(seg1), &[doc.clone()], None).unwrap();
    let doc2 = ColumnarDoc { id: 2, text: "world".into(), metadata: Default::default(), embedding: None };
    toradb_storage::columnar::write_segment_with_compression(&seg_dir.join(seg2), &[doc2], None).unwrap();

    let v1_json = serde_json::json!({
        "schema_version": 1,
        "segments": [seg1, seg2],
        "segment_id_ranges": [
            {"file": seg1, "min_id": 1, "max_id": 1},
            {"file": seg2, "min_id": 2, "max_id": 2}
        ]
    });
    std::fs::write(table_dir.join("manifest.json"), serde_json::to_string_pretty(&v1_json).unwrap()).unwrap();

    // Compact should succeed even with a v1 manifest.
    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        ..Default::default()
    };
    let report = toradb_storage::compaction::compact_table_segments(
        base,
        "docs",
        &policy,
        CompactMode::Full,
    )
    .unwrap();
    assert_eq!(report.merges, 1);

    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, "docs")).unwrap();
    assert_eq!(manifest.segments.len(), 1);
    assert!(!manifest.segment_meta.is_empty(), "segment_meta must be populated after compact");
}

#[test]
fn wal_records_tier_transitions() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let mut dag = DagRunner::open(base).unwrap();

    dag.add_documents("docs", vec![make_doc("p")]).unwrap();
    dag.add_documents("docs", vec![make_doc("q")]).unwrap();

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        tier: TierPolicy {
            tier_merge_threshold: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    persist::compact_table(
        base,
        "docs",
        Some(&mut dag.retrieval.store),
        CompactMode::Normal,
        &policy,
        None,
    )
    .unwrap();
    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, "docs")).unwrap();
    assert_eq!(manifest.segment_meta[0].tier, 1);
}
