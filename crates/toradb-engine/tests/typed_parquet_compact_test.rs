use std::collections::HashMap;

use toradb_core::{ColumnType, ColumnTypeSpec};
use toradb_engine::DagRunner;
use toradb_index::IngestDoc;
use toradb_storage::columnar::{write_segment_with_compression, ColumnarDoc, TableManifestFile};
use toradb_storage::compaction::{compact_table_segments, CompactMode, CompactPolicy};

#[test]
fn compact_full_migrates_legacy_segments_to_native_typed_layout() {
    let dir = std::env::temp_dir().join("toradb_typed_parquet_compact");
    let _ = std::fs::remove_dir_all(&dir);
    let table = "docs";
    let table_dir = dir.join(table);
    let seg_dir = table_dir.join("segments");
    std::fs::create_dir_all(&seg_dir).unwrap();

    let mut manifest = TableManifestFile::default();
    manifest.set_column_types(vec![("rank".to_string(), ColumnTypeSpec::new(ColumnType::Int))]);
    manifest.segments.push("seg_00001.parquet".into());
    manifest.record_segment_id_range("seg_00001.parquet", 0, 1);
    manifest.save(&table_dir.join("manifest.json")).unwrap();

    let mut meta = HashMap::new();
    meta.insert("rank".to_string(), "10".to_string());
    meta.insert("tag".to_string(), "extra".to_string());
    write_segment_with_compression(
        &seg_dir.join("seg_00001.parquet"),
        &[ColumnarDoc {
            id: 0,
            text: "rank ten".into(),
            metadata: meta,
            embedding: None,
        }],
        None,
        &[],
    )
    .unwrap();

    let policy = CompactPolicy::default();
    let report = compact_table_segments(&dir, table, &policy, CompactMode::Full).unwrap();
    assert!(report.merges >= 1 || report.segments_after <= report.segments_before);

    let mut dag = DagRunner::open(&dir).expect("open");
    dag.add_documents(
        table,
        vec![IngestDoc {
            text: "other".into(),
            metadata: [("rank".to_string(), "5".to_string())].into(),
            vector: None,
        }],
    )
    .ok();
    let rows = run_counts(&mut dag, table, "SELECT COUNT(*) FROM docs WHERE rank > 9");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, 1.0, "rank=10 should count after compact + native read");

    let _ = std::fs::remove_dir_all(&dir);
}

fn run_counts(dag: &mut DagRunner, table: &str, sql: &str) -> Vec<(String, f64)> {
    use toradb_engine::sql_exec;
    use toradb_sql::parse;
    let stmts = parse(sql).unwrap();
    let toradb_sql::ast::Stmt::Select(sel) = &stmts[0] else {
        panic!("select");
    };
    let sel = {
        let mut s = sel.clone();
        s.table = table.to_string();
        s
    };
    let sql_exec::SqlSelectResult::Aggregate(out) = sql_exec::run_select(dag, &sel).unwrap() else {
        panic!("aggregate");
    };
    out.group_keys
        .into_iter()
        .zip(out.value_rows.into_iter().map(|r| r[0]))
        .collect()
}
