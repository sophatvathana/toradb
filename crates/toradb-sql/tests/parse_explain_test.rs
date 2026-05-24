use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_explain_select() {
    let stmts = parse(
        "EXPLAIN SELECT id FROM docs SPARSE SEARCH body BM25('motor') LIMIT 5",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    assert!(sel.explain);
    assert!(!sel.stream);
}
