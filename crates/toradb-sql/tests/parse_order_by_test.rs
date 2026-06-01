use toradb_sql::{
    ast::{OrderBy, Stmt},
    parse,
};

fn order_by(sql: &str) -> Option<OrderBy> {
    let stmts = parse(sql).unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    sel.order_by.clone()
}

#[test]
fn parses_order_by_score_desc() {
    let stmts =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('motor') ORDER BY score DESC LIMIT 5")
            .unwrap();
    let Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    assert_eq!(
        sel.order_by,
        Some(OrderBy {
            column: "score".into(),
            descending: true
        })
    );
    assert_eq!(sel.limit, 5);
}

#[test]
fn parses_order_by_score_asc() {
    assert_eq!(
        order_by("SELECT id FROM emb VECTOR SEARCH embedding ANN([1.0, 0.0]) ORDER BY score ASC"),
        Some(OrderBy {
            column: "score".into(),
            descending: false
        })
    );
}

#[test]
fn parses_order_by_score_defaults_to_desc() {
    assert_eq!(
        order_by("SELECT id FROM docs SPARSE SEARCH body BM25('x') ORDER BY score LIMIT 1"),
        Some(OrderBy {
            column: "score".into(),
            descending: true
        })
    );
}

#[test]
fn parses_order_by_metadata_column_defaults_to_asc() {
    assert_eq!(
        order_by("SELECT id FROM docs SPARSE SEARCH body BM25('x') ORDER BY published LIMIT 5"),
        Some(OrderBy {
            column: "published".into(),
            descending: false
        })
    );
}

#[test]
fn parses_order_by_metadata_column_desc() {
    assert_eq!(
        order_by(
            "SELECT id, published FROM docs SPARSE SEARCH body BM25('x') ORDER BY published DESC"
        ),
        Some(OrderBy {
            column: "published".into(),
            descending: true
        })
    );
}
