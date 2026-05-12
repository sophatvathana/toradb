use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn sql_select_runs_sparse_search() {
    let dir = std::env::temp_dir().join("toradb_sql_select");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: Default::default(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radioactivity".into(),
                metadata: Default::default(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(&mut dag, sel).expect("run")
    else {
        panic!("expected search");
    };
    assert!(!out.ids.is_empty());
    assert_eq!(out.ids[0], 0);

    let _ = std::fs::remove_dir_all(&dir);
}
