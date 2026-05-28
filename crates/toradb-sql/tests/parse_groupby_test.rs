use toradb_sql::{ast::AggFunc, ast::SelectExpr, ast::Stmt, parse};

#[test]
fn parse_group_by_select_list() {
    let stmts = parse("SELECT tag, COUNT(*) FROM docs GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.table, "docs");
    assert_eq!(sel.group_by, vec!["tag".to_string()]);
    assert!(sel.select_items.contains(&SelectExpr::Column("tag".into())));
    assert!(sel.select_items.contains(&SelectExpr::Aggregate {
        func: AggFunc::CountStar,
        column: None,
    }));
}

#[test]
fn parse_multi_group_by_and_having() {
    let stmts = parse("SELECT tag, source, COUNT(*), SUM(score) FROM docs GROUP BY tag, source HAVING count > 1").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.group_by, vec!["tag".to_string(), "source".to_string()]);
    assert!(sel.having_clause.is_some());
    assert_eq!(
        sel.select_items
            .iter()
            .filter(|i| matches!(i, SelectExpr::Aggregate { .. }))
            .count(),
        2
    );
}

#[test]
fn parse_with_cte_select() {
    let stmts = parse(
        "WITH top_docs AS (SELECT id, tag FROM docs WHERE tag = 'science') SELECT tag, COUNT(*) FROM top_docs GROUP BY tag",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.ctes.len(), 1);
    assert_eq!(sel.table, "top_docs");
}

#[test]
fn parse_sum_aggregate() {
    let stmts = parse("SELECT tag, SUM(score) FROM docs GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.select_items.contains(&SelectExpr::Aggregate {
        func: AggFunc::Sum,
        column: Some("score".into()),
    }));
}
