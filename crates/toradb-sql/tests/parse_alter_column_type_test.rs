use toradb_sql::ast::Stmt;
use toradb_sql::parse;

#[test]
fn parse_alter_column_type() {
    let stmts = parse("ALTER TABLE docs ALTER COLUMN rank TYPE int").unwrap();
    let Stmt::AlterTableAlterColumnType {
        table,
        column,
        column_type,
        rewrite,
    } = &stmts[0]
    else {
        panic!("expected AlterTableAlterColumnType");
    };
    assert_eq!(table, "docs");
    assert_eq!(column, "rank");
    assert_eq!(column_type, "int");
    assert!(!rewrite);
}

#[test]
fn parse_alter_column_type_vector_dim() {
    let stmts = parse("ALTER TABLE papers ALTER COLUMN embedding TYPE vector(384)").unwrap();
    let Stmt::AlterTableAlterColumnType { column_type, .. } = &stmts[0] else {
        panic!("expected alter column type");
    };
    assert_eq!(column_type, "vector(384)");
}

#[test]
fn parse_alter_column_type_rewrite() {
    let stmts = parse("ALTER TABLE docs ALTER COLUMN rank TYPE int REWRITE").unwrap();
    let Stmt::AlterTableAlterColumnType { rewrite, .. } = &stmts[0] else {
        panic!("expected alter column type");
    };
    assert!(*rewrite);
}
