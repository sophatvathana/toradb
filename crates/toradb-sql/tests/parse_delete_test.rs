use toradb_sql::{
    ast::{CompareOp, Stmt, WherePred},
    parse,
};

#[test]
fn parses_delete_by_id_eq() {
    let stmts = parse("DELETE FROM docs WHERE id = 5").unwrap();
    let Stmt::Delete {
        table,
        where_clause,
    } = &stmts[0]
    else {
        panic!("expected delete");
    };
    assert_eq!(table, "docs");
    let WherePred::Compare { column, op, value } = where_clause.as_ref().unwrap() else {
        panic!("expected compare");
    };
    assert_eq!(column, "id");
    assert_eq!(*op, CompareOp::Eq);
    assert_eq!(value, "5");
}

#[test]
fn parses_delete_by_id_in() {
    let stmts = parse("DELETE FROM docs WHERE id IN (1, 2, 3)").unwrap();
    let Stmt::Delete {
        table,
        where_clause,
    } = &stmts[0]
    else {
        panic!("expected delete");
    };
    assert_eq!(table, "docs");
    let WherePred::In { column, values } = where_clause.as_ref().unwrap() else {
        panic!("expected in");
    };
    assert_eq!(column, "id");
    assert_eq!(values, &["1", "2", "3"]);
}

#[test]
fn parses_delete_without_where() {
    let stmts = parse("DELETE FROM docs").unwrap();
    let Stmt::Delete {
        table,
        where_clause,
    } = &stmts[0]
    else {
        panic!("expected delete");
    };
    assert_eq!(table, "docs");
    assert!(where_clause.is_none());
}
