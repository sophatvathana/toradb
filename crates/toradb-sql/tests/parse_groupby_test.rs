use toradb_sql::{ast::SelectExpr, ast::Stmt, parse};

#[test]
fn parse_group_by_select_list() {
    let stmts = parse("SELECT tag, COUNT(*) FROM docs GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.table, "docs");
    assert_eq!(sel.group_by.as_deref(), Some("tag"));
    assert!(sel.select_items.contains(&SelectExpr::Column("tag".into())));
    assert!(sel.select_items.contains(&SelectExpr::CountStar));
}
