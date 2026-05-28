use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn sql_select_runs_sparse_search() {
    let dir = std::env::temp_dir().join("toradb_sql_select");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: Default::default(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radioactivity".into(),
                metadata: Default::default(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).expect("run")
    else {
        panic!("expected search");
    };
    assert!(!out.ids.is_empty());
    assert_eq!(out.ids[0], 0);
    let id_col = out
        .projected
        .iter()
        .find(|(n, _)| n == "id")
        .expect("id column");
    let toradb_engine::sql_exec::SqlProjectedColumn::U64(ids) = &id_col.1 else {
        panic!("id type");
    };
    assert_eq!(ids[0], 0);
    assert!(
        !out
            .projected
            .iter()
            .any(|(n, _)| n == "score"),
        "SELECT id should not include score"
    );

    let stmts = parse(
        "SELECT id, score, text FROM docs SPARSE SEARCH body BM25('Nikola') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel2) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(out2) =
        sql_exec::run_select(&mut dag, sel2).expect("run")
    else {
        panic!("search");
    };
    assert_eq!(out2.projected.len(), 3);
    let text_col = out2
        .projected
        .iter()
        .find(|(n, _)| n == "text")
        .expect("text");
    let toradb_engine::sql_exec::SqlProjectedColumn::Str(texts) = &text_col.1 else {
        panic!("text type");
    };
    assert!(texts[0].contains("Nikola"));

    let stmts = parse(
        "SELECT * FROM docs SPARSE SEARCH body BM25('Nikola') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel3) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel3
        .select_items
        .iter()
        .any(|e| matches!(e, toradb_sql::ast::SelectExpr::All)));
    let sql_exec::SqlSelectResult::Search(out3) =
        sql_exec::run_select(&mut dag, sel3).expect("run")
    else {
        panic!("search");
    };
    let names: Vec<_> = out3.projected.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["id", "score", "text"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sql_select_with_cte_supports_analytics() {
    let dir = std::env::temp_dir().join("toradb_sql_select_cte");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");
    let stmts = parse(
        "WITH filtered AS (SELECT id, tag FROM docs WHERE tag = 'science') SELECT tag, COUNT(*) FROM filtered GROUP BY tag",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["science".to_string()]);
    assert_eq!(out.value_rows, vec![vec![2.0]]);
    let _ = std::fs::remove_dir_all(&dir);
}
