use toradb_sql::parse;

#[test]
fn parse_create_table_hybrid() {
    let stmts = parse("CREATE TABLE papers USING HYBRID").unwrap();
    assert!(!stmts.is_empty());
}
