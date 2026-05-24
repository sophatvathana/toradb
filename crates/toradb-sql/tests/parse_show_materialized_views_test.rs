use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_show_materialized_views() {
    let stmts = parse("SHOW MATERIALIZED VIEWS").unwrap();
    assert!(matches!(stmts.as_slice(), [Stmt::ShowMaterializedViews]));
}
