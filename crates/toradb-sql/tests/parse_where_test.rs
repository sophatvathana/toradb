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

#[test]
fn parse_where_between() {
    let stmts =
        parse("SELECT tag, COUNT(*) FROM docs WHERE rank BETWEEN 10 AND 20 GROUP BY tag").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::Between { column, low, high, negated } = sel.where_clause.as_ref().unwrap()
    else {
        panic!("between");
    };
    assert_eq!(column, "rank");
    assert_eq!(low, "10");
    assert_eq!(high, "20");
    assert!(!negated);
}

#[test]
fn parse_where_not_between() {
    let stmts = parse(
        "SELECT tag, COUNT(*) FROM docs WHERE published NOT BETWEEN '2024-01-01' AND '2024-12-31' GROUP BY tag",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::Between { column, negated, .. } = sel.where_clause.as_ref().unwrap() else {
        panic!("between");
    };
    assert_eq!(column, "published");
    assert!(negated);
}

#[test]
fn parse_where_like() {
    let stmts = parse(
        "SELECT id, title FROM docs SPARSE SEARCH body BM25('x') WHERE title LIKE '%tesla%'",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::Like { column, pattern, negated } = sel.where_clause.as_ref().unwrap() else {
        panic!("like");
    };
    assert_eq!(column, "title");
    assert_eq!(pattern, "%tesla%");
    assert!(!negated);
}

#[test]
fn parse_where_not_like() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('x') WHERE name NOT LIKE 'a_c'",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let WherePred::Like { column, negated, .. } = sel.where_clause.as_ref().unwrap() else {
        panic!("like");
    };
    assert_eq!(column, "name");
    assert!(negated);
}

#[test]
fn parse_select_distinct() {
    let stmts = parse(
        "SELECT DISTINCT tag FROM docs SPARSE SEARCH body BM25('x')",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.distinct);

    let stmts2 = parse("SELECT tag FROM docs SPARSE SEARCH body BM25('x')").unwrap();
    let Stmt::Select(sel2) = &stmts2[0] else {
        panic!("select");
    };
    assert!(!sel2.distinct);
}
