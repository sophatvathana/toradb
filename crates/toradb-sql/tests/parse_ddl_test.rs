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
