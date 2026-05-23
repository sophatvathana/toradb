use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_order_by_score_desc() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('motor') ORDER BY score DESC LIMIT 5",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.order_by_score_desc, Some(true));
    assert_eq!(sel.limit, 5);
}

#[test]
fn parses_order_by_score_asc_default_limit() {
    let stmts = parse(
        "SELECT id FROM emb VECTOR SEARCH embedding ANN([1.0, 0.0]) ORDER BY score ASC",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.order_by_score_desc, Some(false));
}

#[test]
fn parses_order_by_score_defaults_to_desc() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('x') ORDER BY score LIMIT 1",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.order_by_score_desc, Some(true));
}
