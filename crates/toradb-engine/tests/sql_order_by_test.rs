use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn sql_search_order_by_score_desc() {
    let dir = std::env::temp_dir().join("toradb_sql_order_by");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs = vec![
            IngestDoc {
                text: "unrelated physics".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            },
            IngestDoc {
                text: "Nikola Tesla Nikola Tesla alternating current motor".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            },
            IngestDoc {
                text: "Nikola Tesla once".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            },
        ];
        dag.add_documents("docs", docs).expect("add");
    }

    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') ORDER BY score DESC LIMIT 3",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let mut dag = DagRunner::open(&dir).expect("reopen");
    let sql_exec::SqlSelectResult::Search(result) =
        sql_exec::run_select(&mut dag, sel).expect("search")
    else {
        panic!("search");
    };

    let stmts_raw =
        parse("SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') LIMIT 10")
            .unwrap();
    let toradb_sql::ast::Stmt::Select(sel_raw) = &stmts_raw[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Search(raw) =
        sql_exec::run_select(&mut dag, sel_raw).expect("raw")
    else {
        panic!("search");
    };
    let mut ranked: Vec<_> = raw.ids.into_iter().zip(raw.scores).collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let expected_ids: Vec<_> = ranked.into_iter().map(|(id, _)| id).collect();

    let want = expected_ids.len().min(3);
    assert_eq!(result.ids.len(), want);
    for w in result.scores.windows(2) {
        assert!(
            w[0] >= w[1],
            "scores must be non-increasing: {:?}",
            result.scores
        );
    }
    assert_eq!(result.ids, expected_ids[..want]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sql_search_order_by_score_asc() {
    let dir = std::env::temp_dir().join("toradb_sql_order_by_asc");
    let _ = std::fs::remove_dir_all(&dir);

    {
        let mut dag = DagRunner::open(&dir).expect("open");
        let docs = vec![
            IngestDoc {
                text: "Nikola Tesla motor motor motor".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            },
            IngestDoc {
                text: "Nikola".into(),
                metadata: Default::default(),
                vector: None,
                sparse: None,
            },
        ];
        dag.add_documents("docs", docs).expect("add");
    }

    let stmts = parse(
        "SELECT id FROM docs SPARSE SEARCH body BM25('Nikola Tesla motor') ORDER BY score ASC LIMIT 2",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let mut dag = DagRunner::open(&dir).expect("reopen");
    let sql_exec::SqlSelectResult::Search(result) =
        sql_exec::run_select(&mut dag, sel).expect("search")
    else {
        panic!("search");
    };

    assert_eq!(result.ids.len(), 2);
    for w in result.scores.windows(2) {
        assert!(
            w[0] <= w[1],
            "scores must be non-decreasing: {:?}",
            result.scores
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
