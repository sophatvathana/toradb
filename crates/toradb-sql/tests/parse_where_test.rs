use toradb_sql::{ast::Stmt, parse};

#[test]
fn parse_where_eq_on_metadata() {
    let stmts = parse("SELECT tag, COUNT(*) FROM docs WHERE tag = 'patent' GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let pred = sel.where_eq.as_ref().expect("where");
    assert_eq!(pred.column, "tag");
    assert_eq!(pred.value, "patent");
    assert_eq!(sel.group_by.as_deref(), Some("tag"));
}
