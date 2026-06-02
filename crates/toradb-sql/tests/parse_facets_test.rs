use toradb_sql::{ast::Stmt, format_select, parse};

#[test]
fn parse_single_facet() {
    let stmts =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla') FACETS (category)").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.facets, vec!["category".to_string()]);
}

#[test]
fn parse_multiple_facets_and_lowercases() {
    let stmts =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla') FACETS (Category, Brand)")
            .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(
        sel.facets,
        vec!["category".to_string(), "brand".to_string()]
    );
}

#[test]
fn facets_coexist_with_where_and_limit() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') WHERE tag = 'patent' \
         FACETS (category) LIMIT 5",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.facets, vec!["category".to_string()]);
    assert!(sel.where_clause.is_some());
    assert_eq!(sel.limit, 5);
}

#[test]
fn empty_facets_list_is_an_error() {
    assert!(parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla') FACETS ()").is_err());
}

#[test]
fn facets_round_trip_through_format() {
    let stmts =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla') FACETS (category, brand)")
            .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let rendered = format_select(sel);
    assert!(
        rendered.contains("FACETS (category, brand)"),
        "rendered: {rendered}"
    );
    // Re-parse the rendered SQL and confirm facets survive.
    let stmts2 = parse(&rendered).unwrap();
    let Stmt::Select(sel2) = &stmts2[0] else {
        panic!("select");
    };
    assert_eq!(sel2.facets, sel.facets);
}
