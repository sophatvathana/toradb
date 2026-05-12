use toradb_sql::{ast::Stmt, parse};

#[test]
fn parse_sparse_search_with_quoted_query() {
    let stmts = parse(
        "SELECT id, title FROM papers SPARSE SEARCH abstract BM25('vector database') LIMIT 10",
    )
    .expect("parse");
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    assert_eq!(sel.table, "papers");
    assert_eq!(sel.sparse_query.as_deref(), Some("vector database"));
    assert_eq!(sel.limit, 10);
}
