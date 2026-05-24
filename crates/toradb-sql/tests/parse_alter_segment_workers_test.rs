use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_alter_table_set_segment_workers() {
    let stmts = parse("ALTER TABLE docs SET SEGMENT_WORKERS = 8").unwrap();
    let Stmt::AlterTableSetSegmentWorkers { table, workers } = &stmts[0] else {
        panic!("alter segment workers");
    };
    assert_eq!(table, "docs");
    assert_eq!(*workers, 8);
}
