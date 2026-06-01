use std::path::Path;

use toradb_core::{ColumnType, ColumnTypeSpec};
use toradb_engine::{persist, sql_exec, DagRunner};
use toradb_index::IngestDoc;
use toradb_sql::ast::Stmt;
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

fn run_search_ids(dag: &mut DagRunner, sql: &str) -> Vec<u64> {
    let stmts = parse(sql).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("expected select");
    };
    let sql_exec::SqlSelectResult::Search(out) = sql_exec::run_select(dag, sel).unwrap() else {
        panic!("expected search");
    };
    out.ids
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

fn exec_create_table(dag: &mut DagRunner, ddl: &str) {
    let stmts = parse(ddl).unwrap();
    let Stmt::CreateTable(t) = &stmts[0] else {
        panic!("expected CREATE TABLE");
    };
    let table = t.name.to_lowercase();
    dag.ensure_table(&table);
    let base = dag.db_path().expect("db path");
    persist::ensure_table_on_disk(base, &table).unwrap();
    let column_types: Vec<(String, ColumnTypeSpec)> = t
        .columns
        .iter()
        .map(|(name, ty)| (name.clone(), ColumnTypeSpec::parse(ty)))
        .collect();
    if !column_types.is_empty() {
        persist::set_table_column_types(base, &table, &column_types).unwrap();
    }
}

fn exec_alter_column_type(dag: &mut DagRunner, ddl: &str) {
    let stmts = parse(ddl).unwrap();
    let Stmt::AlterTableAlterColumnType {
        table,
        column,
        column_type,
        rewrite: _,
    } = &stmts[0]
    else {
        panic!("expected ALTER COLUMN TYPE");
    };
    let base = dag.db_path().expect("db path");
    let ty = ColumnTypeSpec::parse(column_type);
    persist::alter_table_column_type(base, table, column, ty).unwrap();
}

#[test]
fn typed_int_where_orders_numerically_not_lexically() {
    let dir = std::env::temp_dir().join("toradb_typed_int_where");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    exec_create_table(&mut dag, "CREATE TABLE docs (rank int) USING text");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("bucket", "a"), ("rank", "9")]),
            doc("b", &[("bucket", "b"), ("rank", "10")]),
            doc("c", &[("bucket", "c"), ("rank", "100")]),
        ],
    )
    .expect("add");

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
fn typed_int_in_list_matches_numerically() {
    let dir = std::env::temp_dir().join("toradb_typed_int_in");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    exec_create_table(&mut dag, "CREATE TABLE docs (rank int) USING text");
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("slot", "a"), ("rank", "9")]),
            doc("b", &[("slot", "b"), ("rank", "10")]),
            doc("c", &[("slot", "c"), ("rank", "100")]),
        ],
    )
    .expect("add");

    let rows = run_counts(
        &mut dag,
        "SELECT slot, COUNT(*) FROM docs WHERE rank IN (9, 10) GROUP BY slot",
    );
    let keys: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert!(keys.contains(&"a"));
    assert!(keys.contains(&"b"), "rank=10 must match IN (9, 10) with int type");
    assert!(!keys.contains(&"c"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn typed_date_where_orders_chronologically() {
    let dir = std::env::temp_dir().join("toradb_typed_date_where");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    exec_create_table(
        &mut dag,
        "CREATE TABLE docs (published date) USING text",
    );
    dag.add_documents(
        "docs",
        vec![
            doc("old", &[("slot", "old"), ("published", "2023-06-15")]),
            doc("new", &[("slot", "new"), ("published", "2024-03-01")]),
            doc("newest", &[("slot", "newest"), ("published", "2024-12-31")]),
        ],
    )
    .expect("add");

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
    exec_create_table(&mut dag, "CREATE TABLE docs (rank int) USING text");
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
fn alter_column_type_upgrades_legacy_table() {
    let dir = std::env::temp_dir().join("toradb_alter_column_type");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    dag.ensure_table("docs");
    persist::ensure_table_on_disk(Path::new(&dir), "docs").unwrap();
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("bucket", "low"), ("score", "5")]),
            doc("b", &[("bucket", "high"), ("score", "15")]),
        ],
    )
    .expect("add");
    exec_alter_column_type(
        &mut dag,
        "ALTER TABLE docs ALTER COLUMN score TYPE int",
    );

    let rows = run_counts(
        &mut dag,
        "SELECT bucket, COUNT(*) FROM docs WHERE score > 10 GROUP BY bucket",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "high");

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

#[test]
fn retrieval_where_filters_by_typed_rank() {
    let dir = std::env::temp_dir().join("toradb_retrieval_where");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    exec_create_table(&mut dag, "CREATE TABLE docs (rank int) USING text");
    dag.add_documents(
        "docs",
        vec![
            doc("rank nine metadata", &[("rank", "9")]),
            doc("rank ten metadata", &[("rank", "10")]),
            doc("rank hundred metadata", &[("rank", "100")]),
        ],
    )
    .expect("add");

    let ids = run_search_ids(
        &mut dag,
        "SELECT id FROM docs SPARSE SEARCH body BM25('rank') WHERE rank > 9 LIMIT 10",
    );
    assert_eq!(ids.len(), 2, "only rank 10 and 100 should match");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compound_where_and_filters_analytics() {
    let dir = std::env::temp_dir().join("toradb_compound_where");
    let _ = std::fs::remove_dir_all(&dir);
    let mut dag = DagRunner::open(&dir).expect("open");
    exec_create_table(
        &mut dag,
        "CREATE TABLE docs (rank int, slot text) USING text",
    );
    dag.add_documents(
        "docs",
        vec![
            doc("a", &[("rank", "10"), ("slot", "A")]),
            doc("b", &[("rank", "10"), ("slot", "B")]),
            doc("c", &[("rank", "5"), ("slot", "A")]),
        ],
    )
    .expect("add");

    let rows = run_counts(
        &mut dag,
        "SELECT slot, COUNT(*) FROM docs WHERE rank > 9 AND slot = 'A' GROUP BY slot",
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "A");

    let _ = std::fs::remove_dir_all(&dir);
}
