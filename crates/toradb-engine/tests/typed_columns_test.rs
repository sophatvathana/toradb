use toradb_core::ColumnType;
use toradb_engine::{persist, sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::parse;

fn run_counts(dag: &mut DagRunner, sql: &str) -> Vec<(String, f64)> {
    let stmts = parse(sql).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(dag, sel).unwrap() else {
        panic!("expected aggregate");
    };
    out.group_keys
        .into_iter()
        .zip(out.value_rows.into_iter().map(|r| r[0]))
        .collect()
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
fn typed_int_where_orders_numerically_not_lexically() {
    let dir = std::env::temp_dir().join("toradb_typed_int_where");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("bucket", "a"), ("rank", "9")]),
            doc("b", &[("bucket", "b"), ("rank", "10")]),
            doc("c", &[("bucket", "c"), ("rank", "100")]),
        ],
    )
    .expect("add");
    persist::set_table_column_types(
        dag.db_path().unwrap(),
        "docs",
        &[("rank".to_string(), ColumnType::Int)],
    )
    .unwrap();

    let rows = run_counts(
        &mut dag,
        "SELECT bucket, COUNT(*) FROM docs WHERE rank > 9 GROUP BY bucket",
    );
    let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"b"), "rank=10 should pass rank > 9");
    assert!(keys.contains(&"c"), "rank=100 should pass rank > 9");
    assert!(!keys.contains(&"a"), "rank=9 should fail rank > 9");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn typed_date_where_orders_chronologically() {
    let dir = std::env::temp_dir().join("toradb_typed_date_where");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("old", &[("slot", "old"), ("published", "2023-06-15")]),
            doc("new", &[("slot", "new"), ("published", "2024-03-01")]),
            doc("newest", &[("slot", "newest"), ("published", "2024-12-31")]),
        ],
    )
    .expect("add");
    persist::set_table_column_types(
        dag.db_path().unwrap(),
        "docs",
        &[("published".to_string(), ColumnType::Date)],
    )
    .unwrap();

    let rows = run_counts(
        &mut dag,
        "SELECT slot, COUNT(*) FROM docs WHERE published >= '2024-01-01' GROUP BY slot",
    );
    let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"new"));
    assert!(keys.contains(&"newest"));
    assert!(!keys.contains(&"old"), "2023 date should be excluded");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn typed_between_inclusive_range() {
    let dir = std::env::temp_dir().join("toradb_typed_between");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("slot", "a"), ("rank", "5")]),
            doc("b", &[("slot", "b"), ("rank", "10")]),
            doc("c", &[("slot", "c"), ("rank", "20")]),
            doc("d", &[("slot", "d"), ("rank", "25")]),
        ],
    )
    .expect("add");
    persist::set_table_column_types(
        dag.db_path().unwrap(),
        "docs",
        &[("rank".to_string(), ColumnType::Int)],
    )
    .unwrap();

    let rows = run_counts(
        &mut dag,
        "SELECT slot, COUNT(*) FROM docs WHERE rank BETWEEN 10 AND 20 GROUP BY slot",
    );
    let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"b"), "10 is inclusive low bound");
    assert!(keys.contains(&"c"), "20 is inclusive high bound");
    assert!(!keys.contains(&"a") && !keys.contains(&"d"), "5 and 25 out of range");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn untyped_column_keeps_legacy_heuristic() {
    let dir = std::env::temp_dir().join("toradb_untyped_legacy");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("bucket", "low"), ("score", "5")]),
            doc("b", &[("bucket", "high"), ("score", "15")]),
        ],
    )
    .expect("add");
    let rows = run_counts(
        &mut dag,
        "SELECT bucket, COUNT(*) FROM docs WHERE score > 10 GROUP BY bucket",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "high");

    let _ = std::fs::remove_dir_all(&dir);
}
