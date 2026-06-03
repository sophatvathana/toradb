use toradb_engine::{sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

fn doc(text: &str, kv: &[(&str, &str)]) -> IngestDoc {
    IngestDoc {
        text: text.into(),
        metadata: kv
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        vector: None,
        sparse: None,
    }
}

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

fn str_col(r: &sql_exec::SqlSearchResult, name: &str) -> Vec<String> {
    for (cname, data) in &r.projected {
        if cname == name {
            if let sql_exec::SqlProjectedColumn::Str(v) = data {
                return v.clone();
            }
        }
    }
    panic!(
        "string column {name} not found in {:?}",
        r.projected.iter().map(|(c, _)| c).collect::<Vec<_>>()
    );
}

fn fresh(name: &str) -> (std::path::PathBuf, DagRunner) {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    let dag = DagRunner::open(&dir).expect("open");
    (dir, dag)
}

#[test]
fn projection_functions_and_dependency_fetch() {
    let (dir, mut dag) = fresh("toradb_scalar_proj");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("category", "Books"), ("title", "  Hello  ")]),
            doc("b", &[("category", "Music"), ("title", "Wide")]),
        ],
    )
    .expect("add");

    let r = search(
        &mut dag,
        "SELECT id, upper(category), length(trim(title)) FROM docs ORDER BY id ASC",
    );
    assert_eq!(str_col(&r, "upper(category)"), vec!["BOOKS", "MUSIC"]);
    assert_eq!(str_col(&r, "length(trim(title))"), vec!["5", "4"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_function_filtering() {
    let (dir, mut dag) = fresh("toradb_scalar_where");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("status", "Active"), ("title", "short")]),
            doc(
                "b",
                &[("status", "inactive"), ("title", "a much longer title")],
            ),
            doc("c", &[("status", "ACTIVE"), ("title", "tiny")]),
        ],
    )
    .expect("add");

    let r = search(
        &mut dag,
        "SELECT id FROM docs WHERE lower(status) = 'active' ORDER BY id ASC",
    );
    assert_eq!(r.ids, vec![0, 2]);

    // length(title) > 5 matches only the long title (the 2nd doc, id 1).
    let r = search(&mut dag, "SELECT id FROM docs WHERE length(title) > 5");
    assert_eq!(r.ids, vec![1]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn order_by_function() {
    let (dir, mut dag) = fresh("toradb_scalar_order");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("title", "bbb")]),  // len 3
            doc("b", &[("title", "a")]),    // len 1
            doc("c", &[("title", "cccc")]), // len 4
            doc("d", &[("title", "dd")]),   // len 2
        ],
    )
    .expect("add");

    // ids are 0-based: id 0='bbb'(3) 1='a'(1) 2='cccc'(4) 3='dd'(2)
    let r = search(&mut dag, "SELECT id FROM docs ORDER BY length(title) DESC");
    assert_eq!(r.ids, vec![2, 0, 3, 1]); // 4,3,2,1 chars

    let r = search(&mut dag, "SELECT id FROM docs ORDER BY length(title) ASC");
    assert_eq!(r.ids, vec![1, 3, 0, 2]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn group_by_function_and_aggregate_over_function() {
    let (dir, mut dag) = fresh("toradb_scalar_group");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("created", "2024-01-05"), ("amount", "-10")]),
            doc("b", &[("created", "2024-01-20"), ("amount", "5")]),
            doc("c", &[("created", "2024-02-02"), ("amount", "-3")]),
        ],
    )
    .expect("add");

    let stmts = parse(
        "SELECT date_trunc('month', created), SUM(abs(amount)) FROM docs \
         GROUP BY date_trunc('month', created)",
    )
    .unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };

    // Two month buckets; abs sums: Jan = 10 + 5 = 15, Feb = 3.
    let jan = out
        .group_keys
        .iter()
        .position(|k| k.starts_with("2024-01"))
        .map(|i| out.value_rows[i][0]);
    let feb = out
        .group_keys
        .iter()
        .position(|k| k.starts_with("2024-02"))
        .map(|i| out.value_rows[i][0]);
    assert_eq!(jan, Some(15.0));
    assert_eq!(feb, Some(3.0));

    let _ = std::fs::remove_dir_all(&dir);
}

fn col_names(r: &sql_exec::SqlSearchResult) -> Vec<String> {
    r.projected.iter().map(|(c, _)| c.clone()).collect()
}

#[test]
fn aliases_rename_output_columns() {
    let (dir, mut dag) = fresh("toradb_scalar_alias");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("category", "Books"), ("title", "Hi")]),
            doc("b", &[("category", "Music"), ("title", "Yo")]),
        ],
    )
    .expect("add");

    let r = search(
        &mut dag,
        "SELECT category AS cat, upper(category) AS shout, title t FROM docs ORDER BY id ASC",
    );
    let names = col_names(&r);
    assert!(names.contains(&"cat".to_string()), "names: {names:?}");
    assert!(names.contains(&"shout".to_string()), "names: {names:?}");
    assert!(names.contains(&"t".to_string()), "names: {names:?}");
    // values land under the alias key
    assert_eq!(str_col(&r, "cat"), vec!["Books", "Music"]);
    assert_eq!(str_col(&r, "shout"), vec!["BOOKS", "MUSIC"]);
    assert_eq!(str_col(&r, "t"), vec!["Hi", "Yo"]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn aggregate_alias_renames_value_column() {
    let (dir, mut dag) = fresh("toradb_scalar_aggalias");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("tag", "x"), ("amount", "10")]),
            doc("b", &[("tag", "x"), ("amount", "5")]),
        ],
    )
    .expect("add");

    let stmts = parse("SELECT tag, SUM(amount) AS total FROM docs GROUP BY tag").unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(&mut dag, sel).unwrap()
    else {
        panic!("aggregate");
    };
    assert_eq!(out.value_columns, vec!["total".to_string()]);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn distinct_dedupes_by_expression_not_alias() {
    let (dir, mut dag) = fresh("toradb_scalar_distinct_alias");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("title", "HELLO")]),
            doc("b", &[("title", "hello")]), // same lowercased value
            doc("c", &[("title", "world")]),
        ],
    )
    .expect("add");

    // DISTINCT must dedupe on lower(title), collapsing the two "hello" rows → 2 rows.
    let r = search(&mut dag, "SELECT DISTINCT lower(title) AS name FROM docs");
    assert!(col_names(&r).contains(&"name".to_string()));
    assert_eq!(r.ids.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
