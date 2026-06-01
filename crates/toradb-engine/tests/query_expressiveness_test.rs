use toradb_core::{ColumnType, ColumnTypeSpec};
use toradb_engine::{persist, sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

fn search(dag: &mut DagRunner, sql: &str) -> sql_exec::SqlSearchResult {
    let stmts = parse(sql).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    match sql_exec::run_select(dag, sel).unwrap() {
        sql_exec::SqlSelectResult::Search(r) => r,
        _ => panic!("expected search result"),
    }
}

/// Pull a projected string column out of a search result by name.
fn col<'a>(r: &'a sql_exec::SqlSearchResult, name: &str) -> Vec<String> {
    for (cname, data) in &r.projected {
        if cname == name {
            if let sql_exec::SqlProjectedColumn::Str(v) = data {
                return v.clone();
            }
        }
    }
    panic!("column {name} not found / not string");
}

fn doc(text: &str, kv: &[(&str, &str)]) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: kv
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        vector: None,
    }
}

#[test]
fn order_by_typed_date_is_chronological_not_lexical() {
    let dir = std::env::temp_dir().join("toradb_qe_order_date");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("tesla coil one", &[("published", "2024-03-01")]),
            doc("tesla coil two", &[("published", "2023-06-15")]),
            doc("tesla coil three", &[("published", "2024-12-31")]),
        ],
    )
    .expect("add");
    persist::set_table_column_types(
        dag.db_path().unwrap(),
        "docs",
        &[(
            "published".to_string(),
            ColumnTypeSpec::new(ColumnType::Date),
        )],
    )
    .unwrap();

    let r = search(
        &mut dag,
        "SELECT id, published FROM docs SPARSE SEARCH body BM25('tesla coil') ORDER BY published ASC LIMIT 10",
    );
    let dates = col(&r, "published");
    assert_eq!(dates, vec!["2023-06-15", "2024-03-01", "2024-12-31"]);

    // DESC reverses.
    let r2 = search(
        &mut dag,
        "SELECT id, published FROM docs SPARSE SEARCH body BM25('tesla coil') ORDER BY published DESC LIMIT 10",
    );
    let dates2 = col(&r2, "published");
    assert_eq!(dates2, vec!["2024-12-31", "2024-03-01", "2023-06-15"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn order_by_untyped_column_is_lexical() {
    // No declared type → lexical fallback. "10" < "9" lexically.
    let dir = std::env::temp_dir().join("toradb_qe_order_lex");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("tesla a", &[("rank", "9")]),
            doc("tesla b", &[("rank", "10")]),
            doc("tesla c", &[("rank", "100")]),
        ],
    )
    .expect("add");

    let r = search(
        &mut dag,
        "SELECT id, rank FROM docs SPARSE SEARCH body BM25('tesla') ORDER BY rank ASC LIMIT 10",
    );
    // Lexical: "10" < "100" < "9".
    assert_eq!(col(&r, "rank"), vec!["10", "100", "9"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_like_filters_substring() {
    let dir = std::env::temp_dir().join("toradb_qe_like");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("tesla one", &[("title", "Nikola Tesla")]),
            doc("tesla two", &[("title", "Thomas Edison")]),
            doc("tesla three", &[("title", "Tesla Motors")]),
        ],
    )
    .expect("add");

    let r = search(
        &mut dag,
        "SELECT id, title FROM docs SPARSE SEARCH body BM25('tesla') WHERE title LIKE '%Tesla%' LIMIT 10",
    );
    let titles = col(&r, "title");
    assert!(titles.iter().all(|t| t.contains("Tesla")));
    assert!(titles.iter().any(|t| t == "Nikola Tesla"));
    assert!(titles.iter().any(|t| t == "Tesla Motors"));
    assert!(!titles.iter().any(|t| t == "Thomas Edison"));

    // NOT LIKE inverts.
    let r2 = search(
        &mut dag,
        "SELECT id, title FROM docs SPARSE SEARCH body BM25('tesla') WHERE title NOT LIKE '%Tesla%' LIMIT 10",
    );
    assert_eq!(col(&r2, "title"), vec!["Thomas Edison"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn distinct_dedupes_projected_column() {
    let dir = std::env::temp_dir().join("toradb_qe_distinct");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("tesla a", &[("category", "physics")]),
            doc("tesla b", &[("category", "physics")]),
            doc("tesla c", &[("category", "chemistry")]),
            doc("tesla d", &[("category", "physics")]),
        ],
    )
    .expect("add");

    let r = search(
        &mut dag,
        "SELECT DISTINCT category FROM docs SPARSE SEARCH body BM25('tesla') LIMIT 10",
    );
    let mut cats = col(&r, "category");
    cats.sort();
    assert_eq!(cats, vec!["chemistry", "physics"]);

    let _ = std::fs::remove_dir_all(&dir);
}
