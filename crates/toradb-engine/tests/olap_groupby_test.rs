use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn group_by_counts_metadata_tags() {
    let dir = std::env::temp_dir().join("toradb_olap_groupby");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla alternating current".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Nikola Tesla wireless power".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radioactivity".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse("SELECT tag, COUNT(*) FROM docs GROUP BY tag").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys.len(), 2);
    let patent_count = out
        .group_keys
        .iter()
        .zip(out.value_rows.iter())
        .find(|(k, _)| k.as_str() == "patent")
        .map(|(_, c)| c[0])
        .unwrap_or(0.0);
    assert_eq!(patent_count, 2.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trivial_count_without_group_by() {
    let dir = std::env::temp_dir().join("toradb_olap_count_no_group");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "a".into(),
                metadata: [("tag".into(), "x".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "b".into(),
                metadata: [("tag".into(), "y".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "c".into(),
                metadata: [("tag".into(), "x".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse("SELECT COUNT(*) FROM docs").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_by_columns, vec!["_all".to_string()]);
    assert_eq!(out.group_keys, vec!["_all".to_string()]);
    assert_eq!(out.value_rows, vec![vec![3.0]]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn group_by_id_uses_doc_id_not_metadata() {
    let dir = std::env::temp_dir().join("toradb_olap_groupby_id");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "a".into(),
                metadata: Default::default(),
                vector: None,
            },
            IngestDoc {
                text: "b".into(),
                metadata: Default::default(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse("SELECT id, COUNT(*) FROM docs GROUP BY id").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["0".to_string(), "1".to_string()]);
    assert_eq!(out.value_rows, vec![vec![1.0], vec![1.0]]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_then_group_by_filters_docs() {
    let dir = std::env::temp_dir().join("toradb_olap_search_groupby");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radioactivity".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT tag, COUNT(*) FROM docs SPARSE SEARCH body BM25('Nikola Tesla') GROUP BY tag",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["patent".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn vector_search_then_group_by_filters_docs() {
    let dir = std::env::temp_dir().join("toradb_olap_vector_groupby");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "papers",
        vec![
            IngestDoc {
                text: "Nikola Tesla coil".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: Some(vec![1.0, 0.0]),
            },
            IngestDoc {
                text: "Marie Curie radiation".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: Some(vec![0.0, 1.0]),
            },
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT tag, COUNT(*) FROM papers VECTOR SEARCH emb ANN([1.0, 0.0]) GROUP BY tag LIMIT 10",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert!(out.group_keys.contains(&"patent".to_string()));
    assert!(out.group_keys.contains(&"science".to_string()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hybrid_sparse_vector_search_then_group_by() {
    let dir = std::env::temp_dir().join("toradb_olap_hybrid_groupby");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: Some(vec![1.0, 0.0]),
            },
            IngestDoc {
                text: "Nikola Tesla wireless power transmission".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: Some(vec![0.95, 0.05]),
            },
            IngestDoc {
                text: "Marie Curie radioactivity".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: Some(vec![0.0, 1.0]),
            },
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT tag, COUNT(*) FROM docs \
         SPARSE SEARCH body BM25('Nikola Tesla') \
         VECTOR SEARCH emb ANN([1.0, 0.0]) \
         GROUP BY tag LIMIT 10",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys.len(), 1);
    assert_eq!(out.group_keys[0], "patent");
    assert!(out.value_rows[0][0] >= 1.0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sum_aggregate_on_numeric_metadata() {
    let dir = std::env::temp_dir().join("toradb_olap_sum");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "doc a".into(),
                metadata: [("tag".into(), "patent".into()), ("score".into(), "10".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "doc b".into(),
                metadata: [("tag".into(), "patent".into()), ("score".into(), "20".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "doc c".into(),
                metadata: [("tag".into(), "science".into()), ("score".into(), "5".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse("SELECT tag, SUM(score) FROM docs GROUP BY tag").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.value_columns, vec!["sum_score".to_string()]);
    let patent_sum = out
        .group_keys
        .iter()
        .zip(out.value_rows.iter())
        .find(|(k, _)| k.as_str() == "patent")
        .map(|(_, v)| v[0])
        .unwrap_or(0.0);
    assert!((patent_sum - 30.0).abs() < f64::EPSILON);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_eq_filters_before_group_by() {
    let dir = std::env::temp_dir().join("toradb_olap_where");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla AC".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radiation".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts =
        parse("SELECT tag, COUNT(*) FROM docs WHERE tag = 'science' GROUP BY tag").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["science".to_string()]);
    assert_eq!(out.value_rows, vec![vec![1.0]]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_in_filters_groups() {
    let dir = std::env::temp_dir().join("toradb_olap_in");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "a".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "b".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "c".into(),
                metadata: [("tag".into(), "other".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT tag, COUNT(*) FROM docs WHERE tag IN ('patent', 'science') GROUP BY tag",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys.len(), 2);
    assert!(!out.group_keys.contains(&"other".to_string()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_gt_numeric_metadata() {
    let dir = std::env::temp_dir().join("toradb_olap_gt");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "low".into(),
                metadata: [("bucket".into(), "low".into()), ("score".into(), "5".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "high".into(),
                metadata: [("bucket".into(), "high".into()), ("score".into(), "15".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts =
        parse("SELECT bucket, COUNT(*) FROM docs WHERE score > 10 GROUP BY bucket").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["high".to_string()]);
    assert_eq!(out.value_rows, vec![vec![1.0]]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn filtered_count_with_where() {
    let dir = std::env::temp_dir().join("toradb_olap_count_where");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "a".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "b".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "c".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse("SELECT COUNT(*) FROM docs WHERE tag = 'science'").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["_all".to_string()]);
    assert_eq!(out.value_rows, vec![vec![2.0]]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn multi_group_and_multi_aggregate_with_having() {
    let dir = std::env::temp_dir().join("toradb_olap_multi_group_having");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "a".into(),
                metadata: [("tag".into(), "science".into()), ("source".into(), "book".into()), ("score".into(), "3".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "b".into(),
                metadata: [("tag".into(), "science".into()), ("source".into(), "book".into()), ("score".into(), "4".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "c".into(),
                metadata: [("tag".into(), "science".into()), ("source".into(), "web".into()), ("score".into(), "1".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");
    let stmts = parse(
        "SELECT tag, source, COUNT(*), SUM(score) FROM docs GROUP BY tag, source HAVING count > 1",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_by_columns, vec!["tag".to_string(), "source".to_string()]);
    assert_eq!(out.value_columns, vec!["count".to_string(), "sum_score".to_string()]);
    assert_eq!(out.group_keys, vec!["science|book".to_string()]);
    assert_eq!(out.value_rows, vec![vec![2.0, 7.0]]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn aggregate_offset_applies_after_sort() {
    let dir = std::env::temp_dir().join("toradb_olap_offset");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc { text: "a".into(), metadata: [("tag".into(), "a".into())].into(), vector: None },
            IngestDoc { text: "b".into(), metadata: [("tag".into(), "b".into())].into(), vector: None },
            IngestDoc { text: "c".into(), metadata: [("tag".into(), "c".into())].into(), vector: None },
        ],
    )
    .expect("add");
    let stmts = parse("SELECT tag, COUNT(*) FROM docs GROUP BY tag OFFSET 1 LIMIT 1").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["b".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}
