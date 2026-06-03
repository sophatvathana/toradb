use toradb_sql::ast::{CompareOp, Expr, SelectExpr, Stmt, WherePred};
use toradb_sql::parser::parse;

fn select(sql: &str) -> toradb_sql::ast::SelectStmt {
    let stmts = parse(sql).expect("parse ok");
    match stmts.into_iter().next().unwrap() {
        Stmt::Select(s) => s,
        _ => panic!("expected SELECT"),
    }
}

#[test]
fn select_scalar_function_and_nesting() {
    let sel = select("SELECT lower(title), round(abs(score)) FROM docs");
    assert_eq!(
        sel.select_items[0],
        SelectExpr::Func {
            expr: Expr::Func {
                name: "lower".into(),
                args: vec![Expr::Column("title".into())],
            },
            alias: None,
        }
    );
    assert_eq!(
        sel.select_items[1],
        SelectExpr::Func {
            expr: Expr::Func {
                name: "round".into(),
                args: vec![Expr::Func {
                    name: "abs".into(),
                    args: vec![Expr::Column("score".into())],
                }],
            },
            alias: None,
        }
    );
}

#[test]
fn aggregate_vs_scalar_disambiguation() {
    let sel = select("SELECT count(*), length(title), sum(abs(x)) FROM docs GROUP BY title");
    // COUNT(*) is an aggregate
    assert!(matches!(
        sel.select_items[0],
        SelectExpr::Aggregate {
            func: toradb_sql::ast::AggFunc::CountStar,
            arg: None,
            alias: None,
        }
    ));
    // length(...) is a scalar function
    assert!(matches!(sel.select_items[1], SelectExpr::Func { .. }));
    // SUM(abs(x)) is an aggregate over a nested scalar function
    assert_eq!(
        sel.select_items[2],
        SelectExpr::Aggregate {
            func: toradb_sql::ast::AggFunc::Sum,
            arg: Some(Expr::Func {
                name: "abs".into(),
                args: vec![Expr::Column("x".into())],
            }),
            alias: None,
        }
    );
}

#[test]
fn where_function_vs_bare_column() {
    let f = select("SELECT id FROM docs WHERE lower(status) = 'active'");
    assert_eq!(
        f.where_clause,
        Some(WherePred::ExprCompare {
            lhs: Expr::Func {
                name: "lower".into(),
                args: vec![Expr::Column("status".into())],
            },
            op: CompareOp::Eq,
            value: "active".into(),
        })
    );

    let b = select("SELECT id FROM docs WHERE status = 'active'");
    assert_eq!(
        b.where_clause,
        Some(WherePred::Compare {
            column: "status".into(),
            op: CompareOp::Eq,
            value: "active".into(),
        })
    );
}

#[test]
fn group_by_and_order_by_functions() {
    let g = select("SELECT count(*) FROM docs GROUP BY date_trunc('month', created_at)");
    assert_eq!(g.group_by, vec!["date_trunc(month,created_at)".to_string()]);
    assert!(g.group_by_exprs[0].is_some());

    let o = select("SELECT id FROM docs ORDER BY length(title) DESC");
    let ob = o.order_by.unwrap();
    assert_eq!(ob.column, "length(title)");
    assert!(ob.descending);
    assert!(ob.key.is_some());

    // bare column order-by keeps key = None
    let o2 = select("SELECT id FROM docs ORDER BY title ASC");
    let ob2 = o2.order_by.unwrap();
    assert_eq!(ob2.column, "title");
    assert!(ob2.key.is_none());
}

#[test]
fn arity_and_unknown_errors() {
    assert!(parse("SELECT lower() FROM docs").is_err());
    assert!(parse("SELECT substr(x) FROM docs").is_err());
    assert!(parse("SELECT now(x) FROM docs").is_err());
    assert!(parse("SELECT bogusfn(x) FROM docs").is_err());
}

#[test]
fn column_alias_explicit_and_bare() {
    // explicit AS
    let sel = select("SELECT category AS cat FROM docs");
    assert_eq!(
        sel.select_items[0],
        SelectExpr::Column {
            name: "category".into(),
            alias: Some("cat".into()),
        }
    );
    assert_eq!(sel.select_items[0].output_name().as_deref(), Some("cat"));

    // bare alias (no AS)
    let sel = select("SELECT category cat FROM docs");
    assert_eq!(
        sel.select_items[0],
        SelectExpr::Column {
            name: "category".into(),
            alias: Some("cat".into()),
        }
    );

    // no alias keeps the column name
    let sel = select("SELECT category FROM docs");
    assert_eq!(sel.select_items[0].output_name().as_deref(), Some("category"));
}

#[test]
fn function_and_aggregate_alias() {
    let sel = select("SELECT lower(title) AS name FROM docs");
    assert_eq!(
        sel.select_items[0],
        SelectExpr::Func {
            expr: Expr::Func {
                name: "lower".into(),
                args: vec![Expr::Column("title".into())],
            },
            alias: Some("name".into()),
        }
    );
    assert_eq!(sel.select_items[0].output_name().as_deref(), Some("name"));

    let sel = select("SELECT SUM(amount) AS total FROM docs GROUP BY tag");
    assert_eq!(
        sel.select_items[0],
        SelectExpr::Aggregate {
            func: toradb_sql::ast::AggFunc::Sum,
            arg: Some(Expr::Column("amount".into())),
            alias: Some("total".into()),
        }
    );
}

#[test]
fn no_false_alias_before_from() {
    // `b` must be a column, not an alias of `a`.
    let sel = select("SELECT a, b FROM docs");
    assert_eq!(sel.select_items.len(), 2);
    assert_eq!(
        sel.select_items[1],
        SelectExpr::Column {
            name: "b".into(),
            alias: None,
        }
    );
}

#[test]
fn as_requires_alias_name() {
    assert!(parse("SELECT col AS FROM docs").is_err());
}

#[test]
fn float_literal_argument() {
    let sel = select("SELECT round(x, 2) FROM docs");
    assert_eq!(
        sel.select_items[0],
        SelectExpr::Func {
            expr: Expr::Func {
                name: "round".into(),
                args: vec![Expr::Column("x".into()), Expr::Literal("2".into())],
            },
            alias: None,
        }
    );
}
