use toradb_sql::{
    ast::{CompareOp, Stmt, WherePred},
    parse,
};

#[test]
fn parse_where_eq_on_metadata() {
    let stmts = parse("SELECT tag, COUNT(*) FROM docs WHERE tag = 'patent' GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::Compare { column, op, value } = sel.where_clause.as_ref().expect("where") else {
        panic!("compare");
    };
    assert_eq!(column, "tag");
    assert_eq!(*op, CompareOp::Eq);
    assert_eq!(value, "patent");
}

#[test]
fn parse_where_ne_and_in() {
    let stmts = parse("SELECT tag, COUNT(*) FROM docs WHERE tag != 'science' GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::Compare { op, value, .. } = sel.where_clause.as_ref().unwrap() else {
        panic!("compare");
    };
    assert_eq!(*op, CompareOp::Ne);
    assert_eq!(value, "science");

    let stmts =
        parse("SELECT tag, COUNT(*) FROM docs WHERE tag IN ('patent', 'science') GROUP BY tag")
            .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::In { column, values } = sel.where_clause.as_ref().unwrap() else {
        panic!("in");
    };
    assert_eq!(column, "tag");
    assert_eq!(values, &["patent", "science"]);
}

#[test]
fn parse_where_numeric_compare() {
    let stmts = parse("SELECT tag, COUNT(*) FROM docs WHERE score > 10 GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::Compare { column, op, value } = sel.where_clause.as_ref().unwrap() else {
        panic!("compare");
    };
    assert_eq!(column, "score");
    assert_eq!(*op, CompareOp::Gt);
    assert_eq!(value, "10");
}
