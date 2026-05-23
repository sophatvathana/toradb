use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_from_join_on_metadata_keys() {
    let stmts = parse(
        "SELECT id FROM papers JOIN citations ON papers.paper_id = citations.paper_id \
         SPARSE SEARCH body BM25('motor') LIMIT 5",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.table, "papers");
    let join = sel.join.as_ref().expect("join");
    assert_eq!(join.right_table, "citations");
    assert_eq!(join.left_key, "paper_id");
    assert_eq!(join.right_key, "paper_id");
}

#[test]
fn parses_select_without_join() {
    let stmts = parse("SELECT id FROM docs SPARSE SEARCH body BM25('x') LIMIT 1").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.join.is_none());
}
