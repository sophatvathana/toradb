use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

#[test]
fn group_by_counts_metadata_tags() {
    let dir = std::env::temp_dir().join("toradb_olap_groupby");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla alternating current".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Nikola Tesla wireless power".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radioactivity".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse("SELECT tag, COUNT(*) FROM docs GROUP BY tag").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys.len(), 2);
    let patent_count = out
        .group_keys
        .iter()
        .zip(out.counts.iter())
        .find(|(k, _)| k.as_str() == "patent")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert_eq!(patent_count, 2);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_then_group_by_filters_docs() {
    let dir = std::env::temp_dir().join("toradb_olap_search_groupby");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla alternating current motor".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radioactivity".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT tag, COUNT(*) FROM docs SPARSE SEARCH body BM25('Nikola Tesla') GROUP BY tag",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["patent".to_string()]);
    assert_eq!(out.counts, vec![1]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_eq_filters_before_group_by() {
    let dir = std::env::temp_dir().join("toradb_olap_where");
    let _ = std::fs::remove_dir_all(&dir);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            IngestDoc {
                text: "Nikola Tesla AC".into(),
                metadata: [("tag".into(), "patent".into())].into(),
                vector: None,
            },
            IngestDoc {
                text: "Marie Curie radiation".into(),
                metadata: [("tag".into(), "science".into())].into(),
                vector: None,
            },
        ],
    )
    .expect("add");

    let stmts =
        parse("SELECT tag, COUNT(*) FROM docs WHERE tag = 'science' GROUP BY tag").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };

    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.group_keys, vec!["science".to_string()]);
    assert_eq!(out.counts, vec![1]);

    let _ = std::fs::remove_dir_all(&dir);
}
