use toradb_core::{ColumnType, ColumnTypeSpec};
use toradb_engine::{persist, sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

fn run(dag: &mut DagRunner, sql: &str) -> sql_exec::SqlSearchResult {
    let stmts = parse(sql).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    match sql_exec::run_select(dag, sel).unwrap() {
        sql_exec::SqlSelectResult::Search(r) => r,
        _ => panic!("expected search/scan result"),
    }
}

fn col(r: &sql_exec::SqlSearchResult, name: &str) -> Vec<String> {
    for (cname, data) in &r.projected {
        if cname == name {
            if let sql_exec::SqlProjectedColumn::Str(v) = data {
                return v.clone();
            }
        }
    }
    panic!("string column {name} not found");
}

fn doc(text: &str, kv: &[(&str, &str)]) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        vector: None,
    }
}

#[test]
fn scan_select_by_id_eq() {
    let dir = std::env::temp_dir().join("toradb_scan_id_eq");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![doc("alpha", &[("tag", "a")]), doc("beta", &[("tag", "b")]), doc("gamma", &[("tag", "c")])],
    )
    .expect("add");

    let r = run(&mut dag, "SELECT id, text FROM docs WHERE id = 1");
    assert_eq!(r.ids, vec![1]);
    assert_eq!(col(&r, "text"), vec!["beta"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_select_by_id_in() {
    let dir = std::env::temp_dir().join("toradb_scan_id_in");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![doc("a", &[]), doc("b", &[]), doc("c", &[]), doc("d", &[])],
    )
    .expect("add");

    let r = run(&mut dag, "SELECT id FROM docs WHERE id IN (0, 2, 3)");
    let mut ids = r.ids.clone();
    ids.sort();
    assert_eq!(ids, vec![0, 2, 3]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_select_by_metadata_and_order_by() {
    let dir = std::env::temp_dir().join("toradb_scan_meta_order");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("p1", &[("kind", "patent"), ("rank", "30")]),
            doc("p2", &[("kind", "patent"), ("rank", "10")]),
            doc("s1", &[("kind", "science"), ("rank", "20")]),
        ],
    )
    .expect("add");
    persist::set_table_column_types(
        dag.db_path().unwrap(),
        "docs",
        &[("rank".to_string(), ColumnTypeSpec::new(ColumnType::Int))],
    )
    .unwrap();

    let r = run(
        &mut dag,
        "SELECT id, rank FROM docs WHERE kind = 'patent' ORDER BY rank ASC",
    );
    assert_eq!(col(&r, "rank"), vec!["10", "30"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_select_distinct() {
    let dir = std::env::temp_dir().join("toradb_scan_distinct");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("cat", "x")]),
            doc("b", &[("cat", "x")]),
            doc("c", &[("cat", "y")]),
        ],
    )
    .expect("add");

    let r = run(&mut dag, "SELECT DISTINCT cat FROM docs");
    let mut cats = col(&r, "cat");
    cats.sort();
    assert_eq!(cats, vec!["x", "y"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_select_excludes_deleted() {
    let dir = std::env::temp_dir().join("toradb_scan_deleted");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", vec![doc("a", &[]), doc("b", &[]), doc("c", &[])]).expect("add");

    let stmts = parse("DELETE FROM docs WHERE id = 1").unwrap();
    let toradb_sql::ast::Stmt::Delete { table, where_clause } = &stmts[0] else { panic!() };
    sql_exec::run_delete(&mut dag, table, where_clause.as_ref()).unwrap();

    let direct = run(&mut dag, "SELECT id FROM docs WHERE id = 1");
    assert!(direct.ids.is_empty(), "deleted id not returned by direct lookup");

    let all = run(&mut dag, "SELECT id FROM docs WHERE id IN (0, 1, 2)");
    let mut ids = all.ids.clone();
    ids.sort();
    assert_eq!(ids, vec![0, 2]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scan_select_star_all_rows() {
    let dir = std::env::temp_dir().join("toradb_scan_star");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents("docs", vec![doc("a", &[]), doc("b", &[])]).expect("add");

    let r = run(&mut dag, "SELECT * FROM docs LIMIT 100");
    assert_eq!(r.ids.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
