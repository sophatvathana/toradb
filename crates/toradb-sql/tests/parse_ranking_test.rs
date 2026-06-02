use toradb_sql::{ast::Stmt, format_select, parse};

#[test]
fn parse_bm25_params() {
    let stmts =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla', k1=1.5, b=0.6)").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.bm25_k1, Some(1.5));
    assert_eq!(sel.bm25_b, Some(0.6));
    assert_eq!(sel.sparse_query.as_deref(), Some("tesla"));
}

#[test]
fn parse_bm25_params_partial() {
    let stmts = parse("SELECT id FROM docs SPARSE SEARCH body BM25('tesla', k1=2)").unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.bm25_k1, Some(2.0));
    assert_eq!(sel.bm25_b, None);
}

#[test]
fn parse_boost_clauses() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') BOOST(title, 2.0) BOOST(tag, 1.5) LIMIT 5",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.field_boosts.get("title"), Some(&2.0));
    assert_eq!(sel.field_boosts.get("tag"), Some(&1.5));
    assert_eq!(sel.limit, 5);
}

#[test]
fn parse_decay_clause() {
    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('tesla') DECAY(published, half_life=30)",
    )
    .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(sel.decay, Some(("published".to_string(), 30.0)));
}

#[test]
fn ranking_knobs_round_trip_through_format() {
    let sql = "SELECT id FROM docs SPARSE SEARCH body BM25('tesla', k1=1.5, b=0.6) \
               BOOST(title, 2) DECAY(published, half_life=30)";
    let stmts = parse(sql).unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let rendered = format_select(sel);
    assert!(rendered.contains("k1=1.5"), "{rendered}");
    assert!(rendered.contains("b=0.6"), "{rendered}");
    assert!(rendered.contains("BOOST(title, 2)"), "{rendered}");
    assert!(
        rendered.contains("DECAY(published, half_life=30)"),
        "{rendered}"
    );

    let reparsed = parse(&rendered).unwrap();
    let Stmt::Select(sel2) = &reparsed[0] else {
        panic!("select");
    };
    assert_eq!(sel2.bm25_k1, sel.bm25_k1);
    assert_eq!(sel2.bm25_b, sel.bm25_b);
    assert_eq!(sel2.field_boosts, sel.field_boosts);
    assert_eq!(sel2.decay, sel.decay);
}
