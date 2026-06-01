use toradb_sql::ast::{Stmt, WherePred};
use toradb_sql::parse;

#[test]
fn parse_where_and() {
    let stmts = parse(
        "SELECT slot, COUNT(*) FROM docs WHERE rank > 9 AND slot = 'A' GROUP BY slot",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    let pred = sel.where_clause.as_ref().expect("where");
    let WherePred::And(parts) = pred else {
        panic!("expected AND, got {pred:?}");
    };
    assert_eq!(parts.len(), 2);
}

#[test]
fn parse_where_or() {
    let stmts = parse("SELECT id FROM docs WHERE rank = 1 OR rank = 2 LIMIT 5").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    let WherePred::Or(parts) = sel.where_clause.as_ref().unwrap() else {
        panic!("expected OR");
    };
    assert_eq!(parts.len(), 2);
}

#[test]
fn parse_where_parentheses() {
    let stmts = parse(
        "SELECT id FROM docs WHERE (rank > 9 OR rank < 2) AND slot = 'A' LIMIT 5",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    let WherePred::And(parts) = sel.where_clause.as_ref().unwrap() else {
        panic!("expected AND");
    };
    assert_eq!(parts.len(), 2);
    assert!(matches!(&parts[0], WherePred::Or(_)));
}
