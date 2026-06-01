use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn sql_search_join_filters_by_metadata_match() {
    let dir = std::env::temp_dir().join("toradb_sql_join");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "papers",
            vec![
                IngestDoc {
                    text: "Nikola Tesla alternating current motor".into(),
                    metadata: [("paper_id".into(), "p1".into())].into(),
                    vector: None,
                },
                IngestDoc {
                    text: "Nikola Tesla wireless power".into(),
                    metadata: [("paper_id".into(), "p2".into())].into(),
                    vector: None,
                },
            ],
        )
        .expect("papers");
        dag.add_documents(
            "citations",
            vec![IngestDoc {
                text: "citation for p1".into(),
                metadata: [("paper_id".into(), "p1".into())].into(),
                vector: None,
            }],
        )
        .expect("citations");
    }

    let stmts = parse(
        "SELECT id FROM papers JOIN citations ON papers.paper_id = citations.paper_id \
         SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let mut dag = DagRunner::open(&dir).expect("reopen");
    let sql_exec::SqlSelectResult::Search(joined) =
        sql_exec::run_select(&mut dag, sel).expect("joined")
    else {
        panic!("search");
    };

    let stmts_all =
        parse("SELECT id FROM papers SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 5")
            .unwrap();
    let toradb_sql::ast::Stmt::Select(sel_all) = &stmts_all[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(all) =
        sql_exec::run_select(&mut dag, sel_all).expect("all")
    else {
        panic!("search");
    };

    assert!(!joined.ids.is_empty());
    assert!(joined.ids.len() < all.ids.len());
    assert_eq!(joined.ids, vec![0]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sql_with_join_combines_cte_and_metadata_join() {
    let dir = std::env::temp_dir().join("toradb_sql_with_join");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        dag.add_documents(
            "papers",
            vec![IngestDoc {
                text: "Tesla coil paper".into(),
                metadata: [("paper_id".into(), "p1".into())].into(),
                vector: None,
            }],
        )
        .expect("papers");
        dag.add_documents(
            "citations",
            vec![IngestDoc {
                text: "citation".into(),
                metadata: [("paper_id".into(), "p1".into())].into(),
                vector: None,
            }],
        )
        .expect("citations");
    }

    let stmts = parse(
        "WITH cited AS (SELECT id, paper_id FROM papers) \
         SELECT id FROM cited JOIN citations ON cited.paper_id = citations.paper_id \
         SPARSE SEARCH body BM25('Tesla') LIMIT 5",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let mut dag = DagRunner::open(&dir).expect("reopen");
    let sql_exec::SqlSelectResult::Search(out) =
        sql_exec::run_select(&mut dag, sel).expect("with join")
    else {
        panic!("search");
    };
    assert_eq!(out.ids, vec![0]);

    let _ = std::fs::remove_dir_all(&dir);
}
