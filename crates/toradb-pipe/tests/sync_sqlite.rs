use std::sync::{Arc, Mutex};

use toradb_engine::DagRunner;
use toradb_pipe::{
    install_drivers, open_sql_source, run_pipeline, ColumnMapping, NullReporter, Pipeline,
    SyncMode,
};

async fn make_source_db(dir: &std::path::Path, rows: &[(i64, &str, &str, &str)]) -> String {
    use sqlx::AnyPool;
    install_drivers();
    let db_file = dir.join("source.db");
    let url = format!("sqlite://{}?mode=rwc", db_file.display());
    let pool = AnyPool::connect(&url).await.expect("connect source");
    sqlx::query(
        "CREATE TABLE docs (id INTEGER PRIMARY KEY, body TEXT, tag TEXT, emb TEXT)",
    )
    .execute(&pool)
    .await
    .expect("create table");
    for (id, body, tag, emb) in rows {
        sqlx::query("INSERT INTO docs (id, body, tag, emb) VALUES (?, ?, ?, ?)")
            .bind(*id)
            .bind(*body)
            .bind(*tag)
            .bind(*emb)
            .execute(&pool)
            .await
            .expect("insert");
    }
    pool.close().await;
    url
}

fn full_pipeline(query: &str) -> Pipeline {
    Pipeline {
        id: "p1".into(),
        name: "test".into(),
        connection_id: "c1".into(),
        query: query.into(),
        target_table: "docs".into(),
        mapping: ColumnMapping {
            text_column: "body".into(),
            metadata_columns: vec!["tag".into()],
            vector_column: Some("emb".into()),
            id_column: Some("id".into()),
            cursor_column: None,
        },
        embedder: None,
        mode: SyncMode::Full,
        last_cursor: None,
        schedule: None,
        drop_table_on_full: true,
        enabled: true,
        batch_size: 2,
        created_at: 0,
    }
}

#[tokio::test]
async fn full_sync_maps_text_metadata_vector() {
    let dir = tempfile::tempdir().unwrap();
    let url = make_source_db(
        dir.path(),
        &[
            (1, "alpha", "x", "[0.1,0.2,0.3]"),
            (2, "beta", "y", ""),
            (3, "gamma", "x", "[0.4,0.5,0.6]"),
            (4, "delta", "z", ""),
            (5, "epsilon", "x", ""),
        ],
    )
    .await;

    let tdb_dir = dir.path().join("tdb");
    let dag = Arc::new(Mutex::new(DagRunner::open(&tdb_dir).expect("open tdb")));

    let pipeline = full_pipeline("SELECT id, body, tag, emb FROM docs");
    let source = open_sql_source(&url, &pipeline).await.expect("open source");
    let outcome = run_pipeline(dag.clone(), source, &pipeline, Arc::new(NullReporter))
        .await
        .expect("run");

    assert_eq!(outcome.rows, 5, "should sync all 5 rows");
    assert_eq!(outcome.state, "done");

    let mut d = dag.lock().unwrap();
    d.ensure_table_queryable("docs").expect("queryable");
    assert_eq!(d.table_row_count("docs").unwrap(), 5);
    let docs: std::collections::HashMap<u64, _> = d
        .fetch_documents("docs", &[0, 1, 2, 3, 4])
        .unwrap()
        .into_iter()
        .collect();
    let alpha = docs.values().find(|doc| doc.text == "alpha").expect("alpha doc");
    assert_eq!(alpha.metadata.get("tag").map(String::as_str), Some("x"));
    assert_eq!(alpha.vector, Some(vec![0.1, 0.2, 0.3]));
    let beta = docs.values().find(|doc| doc.text == "beta").expect("beta doc");
    assert!(
        beta.vector.as_ref().map(|v| v.is_empty()).unwrap_or(true),
        "beta should have no populated vector, got {:?}",
        beta.vector
    );
}

#[tokio::test]
async fn incremental_sync_advances_watermark() {
    let dir = tempfile::tempdir().unwrap();
    let url = make_source_db(
        dir.path(),
        &[(1, "one", "a", ""), (2, "two", "a", ""), (3, "three", "b", "")],
    )
    .await;

    let tdb_dir = dir.path().join("tdb");
    let dag = Arc::new(Mutex::new(DagRunner::open(&tdb_dir).expect("open tdb")));

    let mut pipeline = full_pipeline("SELECT id, body, tag, emb FROM docs");
    pipeline.mode = SyncMode::Incremental;
    pipeline.drop_table_on_full = false;
    pipeline.mapping.cursor_column = Some("id".into());

    let source = open_sql_source(&url, &pipeline).await.unwrap();
    let out1 = run_pipeline(dag.clone(), source, &pipeline, Arc::new(NullReporter))
        .await
        .unwrap();
    assert_eq!(out1.rows, 3);
    assert_eq!(out1.cursor_after.as_deref(), Some("3"));
    pipeline.last_cursor = out1.cursor_after;

    {
        use sqlx::AnyPool;
        let pool = AnyPool::connect(&url).await.unwrap();
        for (id, body) in [(4, "four"), (5, "five")] {
            sqlx::query("INSERT INTO docs (id, body, tag, emb) VALUES (?, ?, 'c', '')")
                .bind(id as i64)
                .bind(body)
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
    }

    let source = open_sql_source(&url, &pipeline).await.unwrap();
    let out2 = run_pipeline(dag.clone(), source, &pipeline, Arc::new(NullReporter))
        .await
        .unwrap();
    assert_eq!(out2.rows, 2, "incremental should pull only new rows");
    assert_eq!(out2.cursor_after.as_deref(), Some("5"));

    let d = dag.lock().unwrap();
    d.ensure_table_queryable("docs").unwrap();
    assert_eq!(d.table_row_count("docs").unwrap(), 5);
}

#[tokio::test]
async fn cdc_changelog_polls_incrementally() {
    let dir = tempfile::tempdir().unwrap();
    let url = {
        use sqlx::AnyPool;
        install_drivers();
        let db = dir.path().join("src.db");
        let url = format!("sqlite://{}?mode=rwc", db.display());
        let pool = AnyPool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE changes (seq INTEGER PRIMARY KEY, body TEXT, op TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO changes VALUES (1,'created a','insert'),(2,'created b','insert')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        url
    };

    let tdb = dir.path().join("tdb");
    let dag = Arc::new(Mutex::new(DagRunner::open(&tdb).unwrap()));

    let mut pipeline = full_pipeline("SELECT seq, body, op FROM changes");
    pipeline.mode = SyncMode::Cdc;
    pipeline.drop_table_on_full = false;
    pipeline.target_table = "changes".into();
    pipeline.mapping = toradb_pipe::ColumnMapping {
        text_column: "body".into(),
        metadata_columns: vec!["op".into()],
        vector_column: None,
        id_column: Some("seq".into()),
        cursor_column: Some("seq".into()),
    };

    let source = open_sql_source(&url, &pipeline).await.unwrap();
    let out1 = run_pipeline(dag.clone(), source, &pipeline, Arc::new(NullReporter))
        .await
        .unwrap();
    assert_eq!(out1.rows, 2);
    assert_eq!(out1.cursor_after.as_deref(), Some("2"));
    pipeline.last_cursor = out1.cursor_after;

    {
        use sqlx::AnyPool;
        let pool = AnyPool::connect(&url).await.unwrap();
        sqlx::query("INSERT INTO changes VALUES (3,'updated a','update')")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    }

    let source = open_sql_source(&url, &pipeline).await.unwrap();
    let out2 = run_pipeline(dag.clone(), source, &pipeline, Arc::new(NullReporter))
        .await
        .unwrap();
    assert_eq!(out2.rows, 1, "CDC should pick up only the new change");
    assert_eq!(out2.cursor_after.as_deref(), Some("3"));
}

#[tokio::test]
async fn sqlite_datetime_column_syncs() {
    use sqlx::sqlite::SqlitePool;
    toradb_pipe::install_drivers();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("dt.db");
    let url = format!("sqlite://{}?mode=rwc", db.display());
    let pool = SqlitePool::connect(&url).await.unwrap();
    sqlx::query(
        "CREATE TABLE pages (id INTEGER PRIMARY KEY, body TEXT, scraped_at DATETIME)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO pages (id, body, scraped_at) VALUES (1, 'hello', '2024-06-01 12:00:00')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let tdb_dir = dir.path().join("tdb");
    let dag = Arc::new(Mutex::new(DagRunner::open(&tdb_dir).expect("open tdb")));
    let mut pipeline = full_pipeline("SELECT id, body, scraped_at FROM pages");
    pipeline.mapping.metadata_columns = vec!["scraped_at".into()];

    let source = open_sql_source(&url, &pipeline).await.expect("open source");
    let outcome = run_pipeline(dag.clone(), source, &pipeline, Arc::new(NullReporter))
        .await
        .expect("run");
    assert_eq!(outcome.rows, 1);

    let mut d = dag.lock().unwrap();
    d.ensure_table_queryable("docs").unwrap();
    let (_, doc) = d.fetch_documents("docs", &[0]).unwrap().into_iter().next().unwrap();
    assert_eq!(doc.text, "hello");
    assert_eq!(
        doc.metadata.get("scraped_at").map(String::as_str),
        Some("2024-06-01 12:00:00")
    );
}
