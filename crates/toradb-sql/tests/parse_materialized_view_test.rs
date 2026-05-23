use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_create_materialized_view() {
    let stmts = parse(
        "CREATE MATERIALIZED VIEW top_docs AS \
         SELECT id FROM docs SPARSE SEARCH body BM25('motor') LIMIT 5",
    )
    .unwrap();
    let Stmt::CreateMaterializedView(mv) = &stmts[0] else {
        panic!("create mv");
    };
    assert_eq!(mv.name, "top_docs");
    assert_eq!(mv.select.table, "docs");
    assert_eq!(mv.select.limit, 5);
}

#[test]
fn parses_refresh_materialized_view() {
    let stmts = parse("REFRESH MATERIALIZED VIEW top_docs").unwrap();
    let Stmt::RefreshMaterializedView { name } = &stmts[0] else {
        panic!("refresh");
    };
    assert_eq!(name, "top_docs");
}
