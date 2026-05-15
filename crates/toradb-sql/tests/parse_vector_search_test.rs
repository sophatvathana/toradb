use toradb_sql::{ast::Stmt, parse};

#[test]
fn parse_vector_search_with_bracket_literal() {
    let stmts = parse(
        "SELECT id FROM papers VECTOR SEARCH embedding ANN([0.9, 0.1, 0.0, 0.0]) LIMIT 5",
    )
    .expect("parse");
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    assert!(sel.vector);
    assert_eq!(sel.vector_query.as_ref().map(|v| v.len()), Some(4));
    assert_eq!(sel.limit, 5);
}

#[test]
fn parse_hybrid_sparse_and_vector() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') VECTOR SEARCH emb ANN([1.0, 0.0]) LIMIT 3",
    )
    .expect("parse");
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    assert_eq!(sel.sparse_query.as_deref(), Some("tesla"));
    assert!(sel.vector);
    assert_eq!(sel.vector_query.as_ref().map(|v| v[0]), Some(1.0));
}
