use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_stream_prefix_on_select() {
    let stmts = parse(
        "STREAM SELECT id FROM docs SPARSE SEARCH body BM25('motor') LIMIT 100",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.stream);
}

#[test]
fn parses_stream_clause_on_select() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('motor') STREAM LIMIT 50",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.stream);
}
