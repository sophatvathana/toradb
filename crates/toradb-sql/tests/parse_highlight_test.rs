use toradb_sql::{ast::Stmt, format_select, parse};

#[test]
fn parse_highlight_no_args() {
    let stmts = parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla') HIGHLIGHT").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.highlight);
    assert_eq!(sel.snippet_len, None);
}

#[test]
fn parse_highlight_with_len() {
    let stmts =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla') HIGHLIGHT(120) LIMIT 5")
            .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert!(sel.highlight);
    assert_eq!(sel.snippet_len, Some(120));
    assert_eq!(sel.limit, 5);
}

#[test]
fn highlight_round_trips_through_format() {
    for sql in [
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') HIGHLIGHT",
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') HIGHLIGHT(200)",
    ] {
        let stmts = parse(sql).unwrap();
        let Stmt::Select(sel) = &stmts[0] else {
            panic!("select");
        };
        let rendered = format_select(sel);
        let reparsed = parse(&rendered).unwrap();
        let Stmt::Select(sel2) = &reparsed[0] else {
            panic!("select");
        };
        assert_eq!(sel2.highlight, sel.highlight, "{rendered}");
        assert_eq!(sel2.snippet_len, sel.snippet_len, "{rendered}");
    }
}
