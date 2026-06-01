use toradb_engine::DagRunner;
use toradb_sql::{binder::Binder, catalog_store, parse};

#[test]
fn catalog_json_persists_create_table() {
    let dir = std::env::temp_dir().join("toradb_catalog_persist");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let stmts = parse("CREATE TABLE demo USING HYBRID").expect("parse");
    let mut binder = Binder::new();
    binder.bind(&stmts).expect("bind");
    catalog_store::save_catalog(&dir, &binder.catalog).expect("save");
    let cat = catalog_store::load_catalog(&dir).expect("load");
    assert!(cat.get("demo").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn create_table_ensure_on_disk_lists_in_catalog() {
    let dir = std::env::temp_dir().join(format!("toradb_create_table_disk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    toradb_engine::persist::ensure_table_on_disk(&dir, "passages").expect("ensure");
    let tables = toradb_engine::persist::list_tables(&dir).expect("list");
    assert!(tables.iter().any(|t| t == "passages"));

    let types = vec![
        (
            "id".to_string(),
            toradb_core::ColumnTypeSpec::new(toradb_core::ColumnType::Int),
        ),
        (
            "body".to_string(),
            toradb_core::ColumnTypeSpec::new(toradb_core::ColumnType::Text),
        ),
    ];
    toradb_engine::persist::set_table_column_types(&dir, "passages", &types).expect("types");
    let ordered = toradb_engine::persist::table_column_types_ordered(&dir, "passages");
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].0, "id");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn show_indexes_and_create_table_sql() {
    let dir = std::env::temp_dir().join("toradb_show_indexes");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "articles",
        vec![toradb_index::IngestDoc {
            text: "patent motor".into(),
            metadata: Default::default(),
            vector: None,
        }],
    )
    .expect("add");
    let stmts = parse("SHOW INDEXES FROM articles").expect("parse");
    let toradb_sql::ast::Stmt::ShowIndexes { table } = &stmts[0] else {
        panic!("show indexes");
    };
    assert_eq!(table, "articles");
    let _ = std::fs::remove_dir_all(&dir);
}
