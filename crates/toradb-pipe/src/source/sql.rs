use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};
use sqlx::{AnyPool, Column, Row, ValueRef};

use super::{Batch, Cursor, FieldInfo, RawRow, Source};

enum DbPool {
    Any(AnyPool),
    Sqlite(SqlitePool),
}

impl DbPool {
    async fn close(self) {
        match self {
            DbPool::Any(p) => p.close().await,
            DbPool::Sqlite(p) => p.close().await,
        }
    }
}

pub fn install_drivers() {
    sqlx::any::install_default_drivers();
}

async fn connect(url: &str) -> Result<DbPool, String> {
    install_drivers();
    match dialect_of(url) {
        Dialect::Sqlite => SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(15))
            .connect(url)
            .await
            .map(DbPool::Sqlite)
            .map_err(|e| e.to_string()),
        _ => AnyPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(15))
            .connect(url)
            .await
            .map(DbPool::Any)
            .map_err(|e| e.to_string()),
    }
}

pub async fn test_connection(url: &str) -> Result<(), String> {
    let pool = connect(url).await?;
    let res: Result<(), String> = match &pool {
        DbPool::Any(p) => sqlx::query("SELECT 1")
            .fetch_optional(p)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
        DbPool::Sqlite(p) => sqlx::query("SELECT 1")
            .fetch_optional(p)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string()),
    };
    pool.close().await;
    res
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
    Postgres,
    MySql,
    Sqlite,
}

fn dialect_of(url: &str) -> Dialect {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("postgres") {
        Dialect::Postgres
    } else if lower.starts_with("mysql") || lower.starts_with("mariadb") {
        Dialect::MySql
    } else {
        Dialect::Sqlite
    }
}

pub async fn list_tables(url: &str) -> Result<Vec<String>, String> {
    let pool = connect(url).await?;
    let sql = match dialect_of(url) {
        Dialect::Postgres => {
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema NOT IN ('pg_catalog','information_schema') \
             ORDER BY table_name"
        }
        Dialect::MySql => {
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = DATABASE() ORDER BY table_name"
        }
        Dialect::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type IN ('table','view') \
             AND name NOT LIKE 'sqlite_%' ORDER BY name"
        }
    };
    let out = match &pool {
        DbPool::Any(p) => {
            let rows = sqlx::query(&sql)
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;
            rows.iter()
                .filter_map(|r| r.try_get::<String, _>(0).ok())
                .collect()
        }
        DbPool::Sqlite(p) => {
            let rows = sqlx::query(&sql)
                .fetch_all(p)
                .await
                .map_err(|e| e.to_string())?;
            rows.iter()
                .filter_map(|r| r.try_get::<String, _>(0).ok())
                .collect()
        }
    };
    pool.close().await;
    Ok(out)
}

pub async fn list_columns(url: &str, table: &str) -> Result<Vec<(String, String)>, String> {
    let table = safe_ident(table)?;
    let pool = connect(url).await?;
    let result = match (dialect_of(url), &pool) {
        (Dialect::Sqlite, DbPool::Sqlite(p)) => {
            let sql = format!("PRAGMA table_info({table})");
            sqlx::query(&sql).fetch_all(p).await.map(|rows| {
                rows.iter()
                    .filter_map(|r| {
                        let name: String = r.try_get("name").ok()?;
                        let ty: String = r.try_get("type").unwrap_or_default();
                        Some((name, ty))
                    })
                    .collect::<Vec<_>>()
            })
        }
        (Dialect::Postgres | Dialect::MySql, DbPool::Any(p)) => {
            // information_schema is portable across PG and MySQL.
            let sql = "SELECT column_name, data_type FROM information_schema.columns \
                       WHERE table_name = ? ORDER BY ordinal_position";
            sqlx::query(sql).bind(table).fetch_all(p).await.map(|rows| {
                rows.iter()
                    .filter_map(|r| {
                        let name: String = r.try_get(0).ok()?;
                        let ty: String = r.try_get(1).unwrap_or_default();
                        Some((name, ty))
                    })
                    .collect::<Vec<_>>()
            })
        }
        _ => {
            pool.close().await;
            return Err("pool dialect mismatch".into());
        }
    };
    pool.close().await;
    result.map_err(|e| e.to_string())
}

pub async fn validate_sqlite(url: &str) -> Result<(), String> {
    let pool = connect(url).await?;
    let DbPool::Sqlite(p) = &pool else {
        pool.close().await;
        return Err("expected sqlite:// URL".into());
    };
    let res = sqlx::query("SELECT count(*) FROM sqlite_master")
        .fetch_optional(p)
        .await
        .map_err(|e| e.to_string());
    pool.close().await;
    res.map(|_| ())
}

fn stringify_sqlite(row: &SqliteRow, idx: usize) -> Option<String> {
    if let Ok(raw) = row.try_get_raw(idx) {
        if raw.is_null() {
            return None;
        }
    }
    if let Ok(s) = row.try_get::<String, _>(idx) {
        return Some(s);
    }
    if let Ok(i) = row.try_get::<i64, _>(idx) {
        return Some(i.to_string());
    }
    if let Ok(i) = row.try_get::<i32, _>(idx) {
        return Some(i.to_string());
    }
    if let Ok(f) = row.try_get::<f64, _>(idx) {
        return Some(f.to_string());
    }
    if let Ok(b) = row.try_get::<bool, _>(idx) {
        return Some(b.to_string());
    }
    if let Ok(dt) = row.try_get::<NaiveDateTime, _>(idx) {
        return Some(dt.to_string());
    }
    if let Ok(dt) = row.try_get::<DateTime<Utc>, _>(idx) {
        return Some(dt.to_rfc3339());
    }
    if let Ok(d) = row.try_get::<NaiveDate, _>(idx) {
        return Some(d.to_string());
    }
    if let Ok(bytes) = row.try_get::<Vec<u8>, _>(idx) {
        return Some(hex_encode(&bytes));
    }
    None
}

pub fn stringify(row: &AnyRow, idx: usize) -> Option<String> {
    if let Ok(raw) = row.try_get_raw(idx) {
        if raw.is_null() {
            return None;
        }
    }
    if let Ok(s) = row.try_get::<String, _>(idx) {
        return Some(s);
    }
    if let Ok(b) = row.try_get::<bool, _>(idx) {
        return Some(b.to_string());
    }
    if let Ok(i) = row.try_get::<i64, _>(idx) {
        return Some(i.to_string());
    }
    if let Ok(i) = row.try_get::<i32, _>(idx) {
        return Some(i.to_string());
    }
    if let Ok(f) = row.try_get::<f64, _>(idx) {
        return Some(f.to_string());
    }
    if let Ok(bytes) = row.try_get::<Vec<u8>, _>(idx) {
        return Some(hex_encode(&bytes));
    }
    None
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("\\x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn row_to_raw_any(row: &AnyRow) -> RawRow {
    let mut values = std::collections::HashMap::new();
    for (idx, col) in row.columns().iter().enumerate() {
        if let Some(v) = stringify(row, idx) {
            values.insert(col.name().to_string(), v);
        }
    }
    RawRow { values }
}

fn row_to_raw_sqlite(row: &SqliteRow) -> RawRow {
    let mut values = std::collections::HashMap::new();
    for (idx, col) in row.columns().iter().enumerate() {
        if let Some(v) = stringify_sqlite(row, idx) {
            values.insert(col.name().to_string(), v);
        }
    }
    RawRow { values }
}

fn safe_ident(name: &str) -> Result<&str, String> {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(name)
    } else {
        Err(format!("unsafe column name: {name:?}"))
    }
}

pub struct SqlSource {
    pool: DbPool,
    query: String,
    cursor_column: Option<String>,
    id_column: Option<String>,
}

impl SqlSource {
    pub async fn open(
        url: &str,
        query: String,
        cursor_column: Option<String>,
        id_column: Option<String>,
    ) -> Result<Self, String> {
        if let Some(c) = &cursor_column {
            safe_ident(c)?;
        }
        if let Some(c) = &id_column {
            safe_ident(c)?;
        }
        let pool = connect(url).await?;
        Ok(Self {
            pool,
            query,
            cursor_column,
            id_column,
        })
    }

    pub async fn close(self) {
        self.pool.close().await;
    }

    async fn fetch_all(&self, sql: &str, bind: Option<&str>) -> Result<Vec<RawRow>, String> {
        match &self.pool {
            DbPool::Any(pool) => {
                let rows = if let Some(v) = bind {
                    sqlx::query(sql).bind(v).fetch_all(pool).await
                } else {
                    sqlx::query(sql).fetch_all(pool).await
                }
                .map_err(|e| e.to_string())?;
                Ok(rows.iter().map(row_to_raw_any).collect())
            }
            DbPool::Sqlite(pool) => {
                let rows = if let Some(v) = bind {
                    sqlx::query(sql).bind(v).fetch_all(pool).await
                } else {
                    sqlx::query(sql).fetch_all(pool).await
                }
                .map_err(|e| e.to_string())?;
                Ok(rows.iter().map(row_to_raw_sqlite).collect())
            }
        }
    }

    async fn fetch_optional_columns(&self, sql: &str) -> Result<Vec<FieldInfo>, String> {
        match &self.pool {
            DbPool::Any(pool) => {
                if let Some(r) = sqlx::query(sql)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    return Ok(r
                        .columns()
                        .iter()
                        .map(|c| FieldInfo {
                            name: c.name().to_string(),
                            data_type: c.type_info().to_string(),
                        })
                        .collect());
                }
            }
            DbPool::Sqlite(pool) => {
                if let Some(r) = sqlx::query(sql)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    return Ok(r
                        .columns()
                        .iter()
                        .map(|c| FieldInfo {
                            name: c.name().to_string(),
                            data_type: c.type_info().to_string(),
                        })
                        .collect());
                }
            }
        }
        Ok(Vec::new())
    }
}

#[async_trait]
impl Source for SqlSource {
    async fn test(&self) -> Result<(), String> {
        match &self.pool {
            DbPool::Any(p) => sqlx::query("SELECT 1")
                .fetch_optional(p)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            DbPool::Sqlite(p) => sqlx::query("SELECT 1")
                .fetch_optional(p)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
        }
    }

    async fn introspect(&self) -> Result<Vec<FieldInfo>, String> {
        let sql = format!("SELECT * FROM ({}) AS _src LIMIT 1", self.query);
        self.fetch_optional_columns(&sql).await
    }

    async fn fetch_batch(&mut self, cursor: &Cursor, n: usize) -> Result<Batch, String> {
        let limit = n as i64;
        if let Some(cursor_col) = &self.cursor_column {
            let col = safe_ident(cursor_col)?;
            let last = match cursor {
                Cursor::Value(v) => Some(v.as_str()),
                _ => None,
            };
            let sql = if last.is_some() {
                format!(
                    "SELECT * FROM ({}) AS _src WHERE {col} > ? ORDER BY {col} ASC LIMIT {limit}",
                    self.query
                )
            } else {
                format!(
                    "SELECT * FROM ({}) AS _src ORDER BY {col} ASC LIMIT {limit}",
                    self.query
                )
            };
            let raw = self.fetch_all(&sql, last).await?;
            let next = raw
                .iter()
                .filter_map(|r| r.get(col).map(str::to_string))
                .max()
                .map(Cursor::Value)
                .unwrap_or_else(|| cursor.clone());
            Ok(Batch { rows: raw, next })
        } else {
            let offset = match cursor {
                Cursor::Offset(o) => *o as i64,
                _ => 0,
            };
            let order = match &self.id_column {
                Some(c) => format!("ORDER BY {} ASC", safe_ident(c)?),
                None => String::new(),
            };
            let sql = format!(
                "SELECT * FROM ({}) AS _src {order} LIMIT {limit} OFFSET {offset}",
                self.query
            );
            let raw = self.fetch_all(&sql, None).await?;
            let count = raw.len() as u64;
            Ok(Batch {
                rows: raw,
                next: Cursor::Offset(offset as u64 + count),
            })
        }
    }
}
