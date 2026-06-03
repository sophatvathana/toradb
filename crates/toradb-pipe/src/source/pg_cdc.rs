use async_trait::async_trait;
use tokio_postgres::NoTls;

use super::{Batch, Cursor, FieldInfo, RawRow, Source};

pub struct PgCdcSource {
    client: tokio_postgres::Client,
    slot: String,
}

impl PgCdcSource {
    pub async fn open(url: &str, slot: &str) -> Result<Self, String> {
        let (client, connection) = tokio_postgres::connect(url, NoTls)
            .await
            .map_err(|e| e.to_string())?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let create = format!(
            "SELECT 1 FROM pg_create_logical_replication_slot('{}', 'test_decoding') \
             WHERE NOT EXISTS (SELECT 1 FROM pg_replication_slots WHERE slot_name = '{}')",
            escape_ident(slot)?,
            escape_ident(slot)?
        );
        client.batch_execute(&create).await.map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            slot: slot.to_string(),
        })
    }
}

fn escape_ident(s: &str) -> Result<&str, String> {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(s)
    } else {
        Err(format!("unsafe slot name: {s:?}"))
    }
}

#[async_trait]
impl Source for PgCdcSource {
    async fn test(&self) -> Result<(), String> {
        self.client
            .simple_query("SELECT 1")
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn introspect(&self) -> Result<Vec<FieldInfo>, String> {
        Ok(vec![
            FieldInfo { name: "lsn".into(), data_type: "text".into() },
            FieldInfo { name: "data".into(), data_type: "text".into() },
        ])
    }

    async fn fetch_batch(&mut self, _cursor: &Cursor, n: usize) -> Result<Batch, String> {
        let rows = self
            .client
            .query(
                "SELECT lsn::text AS lsn, data FROM pg_logical_slot_get_changes($1, NULL, $2)",
                &[&self.slot, &(n as i32)],
            )
            .await
            .map_err(|e| e.to_string())?;

        let mut out = Vec::with_capacity(rows.len());
        let mut last_lsn: Option<String> = None;
        for row in &rows {
            let lsn: String = row.get("lsn");
            let data: String = row.get("data");
            last_lsn = Some(lsn.clone());
            let mut values = std::collections::HashMap::new();
            values.insert("lsn".to_string(), lsn);
            values.insert("data".to_string(), data);
            out.push(RawRow { values });
        }
        Ok(Batch {
            rows: out,
            next: last_lsn.map(Cursor::Value).unwrap_or_else(|| _cursor.clone()),
        })
    }
}
