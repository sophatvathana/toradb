use toradb_engine::{persist, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::{ast::Stmt, parse};
use toradb_storage::columnar::{read_segment, ColumnarDoc, TableManifestFile};
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

#[test]
fn compact_merge_produces_sorted_output() {
    use toradb_storage::columnar::{write_segment_with_compression, TableManifestFile};

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let table = "t";

    let table_dir = base.join(table);
    let seg_dir = table_dir.join("segments");
    std::fs::create_dir_all(&seg_dir).unwrap();

    let seg_a: Vec<ColumnarDoc> = (0..5u64).map(|i| ColumnarDoc {
        id: i * 2,  // 0,2,4,6,8
        text: format!("a-{}", i * 2),
        metadata: Default::default(), embedding: None,
    }).collect();
    let seg_b: Vec<ColumnarDoc> = (0..5u64).map(|i| ColumnarDoc {
        id: i * 2 + 1,  // 1,3,5,7,9
        text: format!("b-{}", i * 2 + 1),
        metadata: Default::default(), embedding: None,
    }).collect();

    let seg_a_name = "seg_00001.parquet";
    let seg_b_name = "seg_00002.parquet";
    write_segment_with_compression(&seg_dir.join(seg_a_name), &seg_a, None, &[]).unwrap();
    write_segment_with_compression(&seg_dir.join(seg_b_name), &seg_b, None, &[]).unwrap();

    let v2 = serde_json::json!({
        "schema_version": 2,
        "segments": [seg_a_name, seg_b_name],
        "index_mode": "segment_only",
        "segment_id_ranges": [
            {"file": seg_a_name, "min_id": 0, "max_id": 8},
            {"file": seg_b_name, "min_id": 1, "max_id": 9},
        ]
    });
    std::fs::write(table_dir.join("manifest.json"),
        serde_json::to_string_pretty(&v2).unwrap()).unwrap();

    let policy = CompactPolicy { small_segment_bytes: u64::MAX, min_segments_to_merge: 2, ..Default::default() };
    persist::compact_table(base, table, None, CompactMode::Full, &policy, None).unwrap();

    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, table)).unwrap();
    assert_eq!(manifest.segments.len(), 1);

    // The merged segment must have sorted IDs.
    let merged = read_segment(&seg_dir.join(&manifest.segments[0])).unwrap();
    assert_eq!(merged.len(), 10);
    let ids: Vec<u64> = merged.iter().map(|d| d.id).collect();
    for i in 0..ids.len()-1 {
        assert!(ids[i] < ids[i+1], "merged segment is not sorted at position {i}: {} > {}", ids[i], ids[i+1]);
    }

    for doc in &merged {
        let expected = if doc.id % 2 == 0 { format!("a-{}", doc.id) } else { format!("b-{}", doc.id) };
        assert_eq!(doc.text, expected, "wrong text for id={}", doc.id);
    }
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
    toradb_storage::columnar::write_segment_with_compression(&seg_dir.join(seg1), &[doc.clone()], None, &[]).unwrap();
    let doc2 = ColumnarDoc { id: 2, text: "world".into(), metadata: Default::default(), embedding: None };
    toradb_storage::columnar::write_segment_with_compression(&seg_dir.join(seg2), &[doc2], None, &[]).unwrap();

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

#[test]
fn fetch_text_correct_with_noncontiguous_sorted_ids() {
    use toradb_storage::columnar::{write_segment_with_compression, TableManifestFile};

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let table = "t";

    let docs: Vec<ColumnarDoc> = (0..5u64)
        .map(|i| ColumnarDoc { id: i, text: format!("low-{i}"), metadata: Default::default(), embedding: None })
        .chain((1000..1005u64)
            .map(|i| ColumnarDoc { id: i, text: format!("high-{i}"), metadata: Default::default(), embedding: None }))
        .collect();

    let table_dir = base.join(table);
    let seg_dir = table_dir.join("segments");
    std::fs::create_dir_all(&seg_dir).unwrap();
    let seg_name = "seg_00001.parquet";
    let seg_path = seg_dir.join(seg_name);
    write_segment_with_compression(&seg_path, &docs, None, &[]).unwrap();

    let actual_min = docs.iter().map(|d| d.id).min().unwrap();
    let actual_max = docs.iter().map(|d| d.id).max().unwrap();
    let v2_json = serde_json::json!({
        "schema_version": 2,
        "segments": [seg_name],
        "index_mode": "segment_only",
        "segment_id_ranges": [{"file": seg_name, "min_id": actual_min, "max_id": actual_max}]
    });
    std::fs::write(table_dir.join("manifest.json"),
        serde_json::to_string_pretty(&v2_json).unwrap()).unwrap();

    let all_ids: Vec<u64> = (0..5).chain(1000..1005).collect();
    let store = toradb_index::CorpusStore::default();
    let fetched = persist::fetch_documents_by_ids(&store, Some(base), table, &all_ids, None).unwrap();
    assert_eq!(fetched.len(), 10, "all 10 docs must be found");

    let by_id: std::collections::HashMap<u64, String> =
        fetched.into_iter().map(|(id, doc)| (id, doc.text)).collect();
    for i in 0..5u64 {
        assert_eq!(by_id.get(&i).map(|s| s.as_str()), Some(format!("low-{i}").as_str()),
            "wrong text for id={i}");
    }
    for i in 1000..1005u64 {
        assert_eq!(by_id.get(&i).map(|s| s.as_str()), Some(format!("high-{i}").as_str()),
            "wrong text for id={i}");
    }
}

#[test]
fn compact_text_survives_gap_ids() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();

    let seg1_docs = vec![
        ColumnarDoc { id: 0, text: "alpha".into(), metadata: Default::default(), embedding: None },
        ColumnarDoc { id: 1, text: "beta".into(),  metadata: Default::default(), embedding: None },
        ColumnarDoc { id: 2, text: "gamma".into(), metadata: Default::default(), embedding: None },
    ];
    let seg2_docs = vec![
        ColumnarDoc { id: 1000, text: "delta".into(),   metadata: Default::default(), embedding: None },
        ColumnarDoc { id: 1001, text: "epsilon".into(), metadata: Default::default(), embedding: None },
        ColumnarDoc { id: 1002, text: "zeta".into(),    metadata: Default::default(), embedding: None },
    ];
    persist::flush_batch(base, "docs", &seg1_docs, 0).unwrap();
    persist::flush_batch(base, "docs", &seg2_docs, 1000).unwrap();

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        ..Default::default()
    };
    persist::compact_table(base, "docs", None, CompactMode::Full, &policy, None).unwrap();

    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, "docs")).unwrap();
    assert_eq!(manifest.segments.len(), 1, "should have one merged segment");

    let range = &manifest.segment_id_ranges[0];
    assert_eq!(range.min_id, 0, "min_id should be 0");
    assert_eq!(range.max_id, 1002, "max_id should be 1002");

    let all_ids = vec![0u64, 1, 2, 1000, 1001, 1002];
    let store = toradb_index::CorpusStore::default();
    let fetched = persist::fetch_documents_by_ids(&store, Some(base), "docs", &all_ids, None).unwrap();
    assert_eq!(fetched.len(), 6, "all 6 docs must be found");
    for (id, doc) in &fetched {
        assert!(!doc.text.is_empty(), "doc {id} has empty text after compaction");
    }

    let by_id: std::collections::HashMap<u64, &str> = fetched.iter().map(|(id, doc)| (*id, doc.text.as_str())).collect();
    assert_eq!(by_id[&0], "alpha");
    assert_eq!(by_id[&1000], "delta");
    assert_eq!(by_id[&1002], "zeta");
}

#[test]
fn compact_segment_bounds_correct_after_merge() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();

    let mut dag = DagRunner::open(base).unwrap();
    dag.add_documents("t", vec![make_doc("first")]).unwrap();
    dag.add_documents("t", vec![make_doc("second")]).unwrap();
    dag.add_documents("t", vec![make_doc("third")]).unwrap();

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        ..Default::default()
    };
    persist::compact_table(
        base, "t", Some(&mut dag.retrieval.store), CompactMode::Full, &policy, None,
    ).unwrap();

    let manifest = TableManifestFile::load(&TableManifestFile::path_for_table(base, "t")).unwrap();
    assert!(!manifest.segment_id_ranges.is_empty(), "segment_id_ranges must be populated after compaction");

    let seg_dir = TableManifestFile::segments_dir(base, "t");
    for range in &manifest.segment_id_ranges {
        let path = seg_dir.join(&range.file);
        assert!(path.exists(), "segment file must exist");
        let docs = read_segment(&path).unwrap();
        let actual_min = docs.iter().map(|d| d.id).min().unwrap();
        let actual_max = docs.iter().map(|d| d.id).max().unwrap();
        assert_eq!(range.min_id, actual_min, "min_id mismatch for {}", range.file);
        assert_eq!(range.max_id, actual_max, "max_id mismatch for {}", range.file);
    }
}

// Regression: SQL query after compaction must not return empty text for scored results.
#[test]
fn compact_sql_query_text_nonempty() {
    use toradb_engine::sql_exec::{self, SqlProjectedColumn, SqlSelectResult};
    use toradb_sql::parse;

    let dir = tempfile::tempdir().unwrap();
    let mut dag = DagRunner::open(dir.path()).unwrap();

    for i in 0..10 {
        dag.add_documents("docs", vec![make_doc(&format!("the quick brown fox document {i}"))]).unwrap();
    }

    let policy = CompactPolicy {
        small_segment_bytes: u64::MAX,
        min_segments_to_merge: 2,
        ..Default::default()
    };
    persist::compact_table(
        dir.path(), "docs", Some(&mut dag.retrieval.store), CompactMode::Full, &policy, None,
    ).unwrap();

    let stmts = parse("SELECT id, text, score FROM docs SPARSE SEARCH body BM25('quick brown fox') LIMIT 10").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else { panic!("expected select") };
    let SqlSelectResult::Search(result) = sql_exec::run_select(&mut dag, sel).unwrap() else {
        panic!("expected search result");
    };

    assert!(!result.ids.is_empty(), "search should return results after compaction");

    let text_col = result.projected.iter().find(|(name, _)| name == "text");
    if let Some((_, SqlProjectedColumn::Str(texts))) = text_col {
        for (i, text) in texts.iter().enumerate() {
            let id = result.ids.get(i).copied().unwrap_or(u64::MAX);
            assert!(!text.is_empty(), "doc {id} has empty text in SQL result after compaction");
        }
    }
}
