use toradb_engine::{sql_exec, DagRunner};
use toradb_sql::{ast::Stmt, parse};

#[test]
fn explain_search_returns_plan_text() {
    let dir = std::env::temp_dir().join("toradb_sql_explain");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut dag = DagRunner::open(&dir).unwrap();
    dag.add_documents(
        "docs",
        vec![toradb_index::IngestDoc {
            text: "Nikola Tesla motor".into(),
            metadata: Default::default(),
            vector: None,
            sparse: None,
        }],
    )
    .unwrap();

    let stmts =
        parse("EXPLAIN SELECT id FROM docs SPARSE SEARCH body BM25('Tesla') LIMIT 5").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("search");
    };

    let text = out.explain_text.expect("plan text");
    assert!(
        text.contains("RetrievalScan"),
        "expected plan text, got: {text}"
    );
    assert!(
        text.contains("sparse=true"),
        "expected sparse=true in plan, got: {text}"
    );
    assert!(
        text.contains("table=docs"),
        "expected table=docs in plan, got: {text}"
    );
    assert!(out.ids.is_empty(), "EXPLAIN should not execute the search");
}

#[test]
fn explain_analytics_returns_plan_text() {
    let dir = std::env::temp_dir().join("toradb_sql_explain_analytics");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut dag = DagRunner::open(&dir).unwrap();
    dag.add_documents(
        "docs",
        vec![toradb_index::IngestDoc {
            text: "Nikola Tesla motor".into(),
            metadata: Default::default(),
            vector: None,
            sparse: None,
        }],
    )
    .unwrap();

    let stmts = parse("EXPLAIN SELECT COUNT(*) FROM docs").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("search");
    };
    let text = out.explain_text.expect("plan text");
    assert!(text.contains("Scan"), "expected plan text, got: {text}");
}
