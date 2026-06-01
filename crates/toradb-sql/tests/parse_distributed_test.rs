use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_distributed_on_sparse_search() {
    let stmts =
        parse("SELECT id FROM docs DISTRIBUTED SPARSE SEARCH body BM25('motor') LIMIT 10").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.distributed);
}
