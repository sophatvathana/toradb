use toradb_sql::{ast::Stmt, parse};

#[test]
fn parses_show_tables() {
    let stmts = parse("SHOW TABLES").unwrap();
    assert!(matches!(stmts.as_slice(), [Stmt::ShowTables]));
}

#[test]
fn parses_create_index() {
    let stmts = parse("CREATE INDEX emb_idx ON papers (embedding) USING HNSW").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::CreateIndex(idx)]
            if idx.name == "EMB_IDX"
                && idx.table == "papers"
                && idx.column == "EMBEDDING"
                && idx.using == "HNSW"
    ));
    let stmts = parse("CREATE INDEX text_idx ON docs (body) USING BM25").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::CreateIndex(idx)] if idx.using == "BM25"
    ));
    let stmts = parse("CREATE INDEX ann_idx ON emb (embedding) USING DISKANN").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::CreateIndex(idx)] if idx.using == "DISKANN"
    ));
}

#[test]
fn parses_create_table_with_typed_columns() {
    let stmts =
        parse("CREATE TABLE events (id int, published date, score float, body varchar(255)) USING text")
            .unwrap();
    let Stmt::CreateTable(t) = &stmts[0] else {
        panic!("create table");
    };
    assert_eq!(t.name, "events");
    assert_eq!(t.mode, "TEXT");
    let cols: Vec<(&str, &str)> = t
        .columns
        .iter()
        .map(|(n, ty)| (n.as_str(), ty.as_str()))
        .collect();
    assert_eq!(
        cols,
        vec![
            ("id", "INT"),
            ("published", "DATE"),
            ("score", "FLOAT"),
            ("body", "VARCHAR(255)"),
        ]
    );
}

#[test]
fn parses_create_table_without_columns() {
    let stmts = parse("CREATE TABLE docs USING hybrid").unwrap();
    let Stmt::CreateTable(t) = &stmts[0] else {
        panic!("create table");
    };
    assert!(t.columns.is_empty());
    assert_eq!(t.mode, "HYBRID");
}

#[test]
fn parses_drop_table() {
    let stmts = parse("DROP TABLE articles").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::DropTable { name }] if name == "articles"
    ));
}

#[test]
fn parses_describe_table() {
    let stmts = parse("DESCRIBE articles").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::Describe { name }] if name == "articles"
    ));
    let stmts = parse("DESC docs").unwrap();
    assert!(matches!(
        stmts.as_slice(),
        [Stmt::Describe { name }] if name == "docs"
    ));
}
