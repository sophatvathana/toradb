use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn sql_vector_search_returns_nearest_embedding() {
    let dir = std::env::temp_dir().join("toradb_sql_vector");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "papers",
        vec![
            IngestDoc {
                text: "Nikola Tesla coil".into(),
                metadata: Default::default(),
                vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
                sparse: None,
            },
            IngestDoc {
                text: "Marie Curie radiation".into(),
                metadata: Default::default(),
                vector: Some(vec![0.0, 1.0, 0.0, 0.0]),
                sparse: None,
            },
        ],
    )
    .expect("add");

    let stmts =
        parse("SELECT id FROM papers VECTOR SEARCH embedding ANN([0.95, 0.05, 0.0, 0.0]) LIMIT 1")
            .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).expect("run")
    else {
        panic!("expected search");
    };
    assert_eq!(out.ids, vec![0]);

    let _ = std::fs::remove_dir_all(&dir);
}
