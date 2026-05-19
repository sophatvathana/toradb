use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_show_tables() {
    let stmts = parse("SHOW TABLES").unwrap();
    assert!(matches!(stmts.as_slice(), [Stmt::ShowTables]));
}

#[test]
fn parses_describe_table() {
    let stmts = parse("DESCRIBE articles").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::Describe { name }] if name == "articles"
    ));
    let stmts = parse("DESC docs").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::Describe { name }] if name == "docs"
    ));
}
