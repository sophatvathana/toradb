use toradb_core::IndexMode;
use toradb_sql::{
    ast::{CreateIndexStmt, CreateTableStmt, Stmt},
    binder::Binder,
};

#[test]
fn bind_create_index_updates_table_mode() {
    let mut binder = Binder::new();
    binder
        .bind(&[Stmt::CreateTable(CreateTableStmt {
            namespace: None,
            name: "PAPERS".into(),
            mode: "TEXT".into(),
            columns: vec![],
        })])
        .unwrap();
    binder
        .bind(&[Stmt::CreateIndex(CreateIndexStmt {
            name: "VEC_IDX".into(),
            namespace: None,
            table: "papers".into(),
            column: "EMBEDDING".into(),
            using: "HNSW".into(),
        })])
        .unwrap();
    let manifest = binder.catalog.get("PAPERS").expect("table");
    assert_eq!(manifest.index_mode, IndexMode::Vector);
}
