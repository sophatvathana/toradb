use toradb_engine::{materialized, sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn materialized_view_create_select_and_refresh() {
    let dir = std::env::temp_dir().join("toradb_sql_mv");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "docs",
            vec![
                IngestDoc {
                    text: "Nikola Tesla motor".into(),
                    metadata: Default::default(),
                    vector: None,
                    sparse: None,
                },
                IngestDoc {
                    text: "Marie Curie radioactivity".into(),
                    metadata: Default::default(),
                    vector: None,
                    sparse: None,
                },
            ],
        )
        .expect("add");
    }

    let create = parse(
        "CREATE MATERIALIZED VIEW top_docs AS \
         SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::CreateMaterializedView(mv) = &create[0] else {
        panic!("create");
    };

    let mut dag = DagRunner::open(&dir).expect("reopen");
    let base = dag.db_path().expect("path").to_path_buf();
    let rows =
        materialized::create_materialized_view(&mut dag, base.as_path(), &mv.name, &mv.select)
            .expect("create mv");
    assert!(rows > 0);
    assert!(rows <= 5);

    let read = parse("SELECT id FROM top_docs LIMIT 10").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &read[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(cached) =
        sql_exec::run_select(&mut dag, sel).expect("read mv")
    else {
        panic!("search");
    };
    assert_eq!(cached.ids.len(), rows);

    dag.add_documents(
        "docs",
        vec![IngestDoc {
            text: "Nikola Tesla coil".into(),
            metadata: Default::default(),
            vector: None,
            sparse: None,
        }],
    )
    .expect("add more");

    let refreshed = materialized::refresh_materialized_view(&mut dag, base.as_path(), "top_docs")
        .expect("refresh");
    assert!(refreshed >= rows);

    materialized::drop_materialized_view(base.as_path(), "top_docs").expect("drop");
    assert!(!materialized::is_materialized_view(
        base.as_path(),
        "top_docs"
    ));

    let drop_sql = parse("DROP MATERIALIZED VIEW top_docs").unwrap();
    let toradb_sql::ast::Stmt::DropMaterializedView { name } = &drop_sql[0] else {
        panic!("drop stmt");
    };
    assert_eq!(name, "top_docs");
    materialized::drop_materialized_view(base.as_path(), name).expect_err("already dropped");

    let _ = std::fs::remove_dir_all(&dir);
}
