use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_limit_offset_on_sparse_search() {
    let stmts = parse(
        "SELECT id FROM papers SPARSE SEARCH body BM25('motor') LIMIT 5 OFFSET 10",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.limit, 5);
    assert_eq!(sel.offset, 10);
}

#[test]
fn parses_offset_after_limit_vector_search() {
    let stmts = parse(
        "SELECT id FROM emb VECTOR SEARCH embedding ANN([1.0, 0.0]) LIMIT 3 OFFSET 1",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.limit, 3);
    assert_eq!(sel.offset, 1);
    assert!(sel.vector);
}
