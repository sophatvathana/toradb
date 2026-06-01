use toradb_engine::{persist, sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;
use toradb_storage::columnar::{read_segment, TableManifestFile};
use toradb_storage::compaction::{CompactMode, CompactPolicy};

fn search_ids(dag: &mut DagRunner, sql: &str) -> Vec<u64> {
    let stmts = parse(sql).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    match sql_exec::run_select(dag, sel).unwrap() {
        sql_exec::SqlSelectResult::Search(r) => r.ids,
        _ => panic!("expected search"),
    }
}

fn count_star(dag: &mut DagRunner, table: &str) -> f64 {
    let stmts = parse(&format!("SELECT COUNT(*) FROM {table}")).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    match sql_exec::run_select(dag, sel).unwrap() {
        sql_exec::SqlSelectResult::Aggregate(a) => a.value_rows[0][0],
        _ => panic!("expected aggregate"),
    }
}

fn delete(dag: &mut DagRunner, sql: &str) -> usize {
    let stmts = parse(sql).unwrap();
    let toradb_sql::ast::Stmt::Delete {
        table,
        where_clause,
    } = &stmts[0]
    else {
        panic!("expected delete");
    };
    sql_exec::run_delete(dag, table, where_clause.as_ref()).unwrap()
}

fn doc(text: &str, tag: &str) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: [("tag".to_string(), tag.to_string())].into(),
        vector: None,
    }
}

#[test]
fn delete_excludes_from_search_count_and_groupby() {
    let dir = std::env::temp_dir().join("toradb_delete_basic");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("nikola tesla one", "patent"),    // id 0
            doc("nikola tesla two", "patent"),    // id 1
            doc("nikola tesla three", "science"), // id 2
        ],
    )
    .expect("add");

    let before = search_ids(
        &mut dag,
        "SELECT id FROM docs SPARSE SEARCH body BM25('nikola tesla') LIMIT 10",
    );
    assert!(before.contains(&1));
    assert_eq!(count_star(&mut dag, "docs"), 3.0);

    let n = delete(&mut dag, "DELETE FROM docs WHERE id = 1");
    assert_eq!(n, 1);

    let after = search_ids(
        &mut dag,
        "SELECT id FROM docs SPARSE SEARCH body BM25('nikola tesla') LIMIT 10",
    );
    assert!(!after.contains(&1), "deleted id must not appear in search");
    assert_eq!(count_star(&mut dag, "docs"), 2.0);
    let stmts = parse("SELECT tag, COUNT(*) FROM docs GROUP BY tag").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!()
    };
    let sql_exec::SqlSelectResult::Aggregate(a) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!()
    };
    let patent = a
        .group_keys
        .iter()
        .zip(&a.value_rows)
        .find(|(k, _)| k.as_str() == "patent")
        .map(|(_, v)| v[0])
        .unwrap();
    assert_eq!(patent, 1.0);

    assert_eq!(delete(&mut dag, "DELETE FROM docs WHERE id = 1"), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_by_id_in_removes_all() {
    let dir = std::env::temp_dir().join("toradb_delete_in");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("tesla a", "x"),
            doc("tesla b", "x"),
            doc("tesla c", "x"),
            doc("tesla d", "x"),
        ],
    )
    .expect("add");

    assert_eq!(delete(&mut dag, "DELETE FROM docs WHERE id IN (0, 2)"), 2);
    let ids = search_ids(
        &mut dag,
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') LIMIT 10",
    );
    assert!(!ids.contains(&0) && !ids.contains(&2));
    assert!(ids.contains(&1) && ids.contains(&3));
    assert_eq!(count_star(&mut dag, "docs"), 2.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_survives_reopen() {
    let dir = std::env::temp_dir().join("toradb_delete_reopen");
    let _ = std::fs::remove_dir_all(&dir);
    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents("docs", vec![doc("tesla a", "x"), doc("tesla b", "x")])
            .expect("add");
        assert_eq!(delete(&mut dag, "DELETE FROM docs WHERE id = 0"), 1);
    }
    let mut dag = DagRunner::open(&dir).expect("reopen");
    let ids = search_ids(
        &mut dag,
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') LIMIT 10",
    );
    assert!(!ids.contains(&0), "deletion must survive restart");
    assert_eq!(count_star(&mut dag, "docs"), 1.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compaction_reclaims_tombstoned_rows() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let mut dag = DagRunner::open(base).expect("open");
    dag.add_documents("docs", vec![doc("tesla a", "x"), doc("tesla b", "x")])
        .expect("add"); // ids 0,1
    dag.add_documents("docs", vec![doc("tesla c", "x"), doc("tesla d", "x")])
        .expect("add"); // ids 2,3

    assert_eq!(delete(&mut dag, "DELETE FROM docs WHERE id = 1"), 1);
    let tombstones_path = base.join("docs").join("indexes").join("tombstones.bin");
    assert!(
        tombstones_path.exists(),
        "tombstone file should exist after delete"
    );

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
    .expect("compact");

    let manifest =
        TableManifestFile::load(&TableManifestFile::path_for_table(base, "docs")).unwrap();
    assert_eq!(manifest.segments.len(), 1, "segments merged into one");
    assert_eq!(
        manifest.segment_meta[0].deleted_count, 0,
        "merged segment has no tombstones"
    );
    let seg_dir = TableManifestFile::segments_dir(base, "docs");
    let merged = read_segment(&seg_dir.join(&manifest.segments[0])).unwrap();
    let ids: Vec<u64> = merged.iter().map(|d| d.id).collect();
    assert_eq!(merged.len(), 3, "one row physically removed");
    assert!(!ids.contains(&1), "deleted id physically gone from segment");
    assert!(
        !tombstones_path.exists(),
        "tombstone file removed after full reclaim"
    );

    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn delete_without_where_is_rejected() {
    let dir = std::env::temp_dir().join("toradb_delete_nowhere");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", vec![doc("a", "x")]).expect("add");

    let stmts = parse("DELETE FROM docs").unwrap();
    let toradb_sql::ast::Stmt::Delete {
        table,
        where_clause,
    } = &stmts[0]
    else {
        panic!()
    };
    let err = sql_exec::run_delete(&mut dag, table, where_clause.as_ref());
    assert!(err.is_err(), "DELETE without WHERE must be rejected");

    let _ = std::fs::remove_dir_all(&dir);
}
