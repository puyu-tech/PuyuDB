use std::path::Path;

use adr02_protocol::{Engine, RootRow, SCHEMA, Snapshot};
use turso::{Builder, Connection, Database, Value};

struct Store {
    rt: tokio::runtime::Runtime,
    _db: Database,
    conn: Connection,
}

impl Store {
    fn open_path(path: &Path) -> Result<Self, String> {
        let text = path.to_str().ok_or("path is not utf-8")?.to_owned();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let (db, conn) = rt.block_on(async {
            let db = Builder::new_local(&text)
                .build()
                .await
                .map_err(|e| e.to_string())?;
            let conn = db.connect().map_err(|e| e.to_string())?;
            conn.execute_batch("PRAGMA journal_mode=WAL")
                .await
                .map_err(|e| e.to_string())?;
            conn.execute_batch("PRAGMA synchronous=FULL")
                .await
                .map_err(|e| e.to_string())?;
            if std::env::var_os("ADR02_FULLFSYNC").is_some() {
                conn.execute_batch("PRAGMA fullfsync=ON")
                    .await
                    .map_err(|e| e.to_string())?;
            }
            conn.execute_batch("PRAGMA foreign_keys=ON")
                .await
                .map_err(|e| e.to_string())?;
            conn.execute_batch(SCHEMA)
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>((db, conn))
        })?;
        Ok(Self { rt, _db: db, conn })
    }

    fn exec(&self, sql: &str, params: Vec<Value>) -> Result<(), String> {
        self.rt
            .block_on(self.conn.execute(sql, turso::params_from_iter(params)))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn version(&self) -> String {
        self.rt
            .block_on(async {
                let mut rows = self.conn.query("SELECT sqlite_version()", ()).await?;
                let Some(row) = rows.next().await? else {
                    return Ok("none".into());
                };
                Ok(format!("{:?}", row.get_value(0)?))
            })
            .unwrap_or_else(|err: turso::Error| err.to_string())
    }

    fn pragma(&self, name: &str) -> String {
        self.rt
            .block_on(async {
                let mut rows = self.conn.query(&format!("PRAGMA {name}"), ()).await?;
                let Some(row) = rows.next().await? else {
                    return Ok("none".into());
                };
                Ok(format!("{:?}", row.get_value(0)?))
            })
            .unwrap_or_else(|err: turso::Error| err.to_string())
    }
}

impl Engine for Store {
    fn open(dir: &Path) -> Result<Self, String> {
        Self::open_path(&dir.join("turso.db"))
    }

    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String> {
        self.exec(
            "INSERT OR IGNORE INTO chunks(key, bytes) VALUES (?1, ?2)",
            vec![Value::Blob(key.to_vec()), Value::Blob(bytes.to_vec())],
        )
    }

    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String> {
        self.rt
            .block_on(self.conn.execute_batch("BEGIN IMMEDIATE"))
            .map_err(|e| e.to_string())?;
        let result = (|| {
            self.exec(
                "INSERT INTO roots(root_id, stream, seq, chunk_key) VALUES (?1, ?2, ?3, ?4)",
                vec![
                    Value::Integer(root as i64),
                    Value::Integer(stream as i64),
                    Value::Integer(seq as i64),
                    Value::Blob(key.to_vec()),
                ],
            )?;
            self.exec(
                "INSERT INTO events(cursor, root_id) VALUES (?1, ?1)",
                vec![Value::Integer(root as i64)],
            )?;
            self.exec(
                "INSERT INTO outbox(root_id, peer) VALUES (?1, 1)",
                vec![Value::Integer(root as i64)],
            )?;
            self.rt
                .block_on(self.conn.execute_batch("COMMIT"))
                .map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = self.rt.block_on(self.conn.execute_batch("ROLLBACK"));
        }
        result
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let mut snap = Snapshot::default();
        query(self, "SELECT key FROM chunks", |row| {
            snap.chunks.push(blob_key(row, 0)?);
            Ok(())
        })?;
        query(
            self,
            "SELECT root_id, stream, seq, chunk_key FROM roots",
            |row| {
                snap.roots.push(RootRow {
                    root_id: integer(row, 0)? as u64,
                    stream: integer(row, 1)? as u64,
                    seq: integer(row, 2)? as u64,
                    chunk_key: blob_key(row, 3)?,
                });
                Ok(())
            },
        )?;
        query(self, "SELECT cursor, root_id FROM events", |row| {
            snap.events
                .push((integer(row, 0)? as u64, integer(row, 1)? as u64));
            Ok(())
        })?;
        query(self, "SELECT root_id, peer FROM outbox", |row| {
            snap.outbox
                .push((integer(row, 0)? as u64, integer(row, 1)? as u64));
            Ok(())
        })?;
        Ok(snap)
    }

    fn probe_native(dir: &Path) -> Result<String, String> {
        let path = dir.join("turso.db");
        let first = Self::open_path(&path)?;
        first
            .rt
            .block_on(first.conn.execute_batch("BEGIN IMMEDIATE"))
            .map_err(|e| e.to_string())?;
        let second = match Self::open_path(&path) {
            Ok(store) => store,
            Err(err) => return Ok(format!("second_open=rejected:{err}")),
        };
        let _ = second
            .rt
            .block_on(second.conn.execute_batch("PRAGMA busy_timeout=0"));
        let write = second
            .rt
            .block_on(second.conn.execute_batch("BEGIN IMMEDIATE"));
        let _ = first.rt.block_on(first.conn.execute_batch("ROLLBACK"));
        match write {
            Ok(()) => Ok("second_begin=accepted".into()),
            Err(err) => Ok(format!("second_begin=rejected:{err}")),
        }
    }

    fn settings(&self) -> String {
        format!(
            "journal_mode={} synchronous={} fullfsync={} foreign_keys={}",
            self.pragma("journal_mode"),
            self.pragma("synchronous"),
            self.pragma("fullfsync"),
            self.pragma("foreign_keys")
        ) + &format!(" sqlite_version={}", self.version())
    }

    fn identify() -> String {
        let flush = if std::env::var_os("ADR02_FULLFSYNC").is_some() {
            " fullfsync=on"
        } else {
            " fullfsync=default"
        };
        format!("turso 0.9.0-pre.2{flush}")
    }
}

fn query(
    store: &Store,
    sql: &str,
    mut on_row: impl FnMut(&turso::Row) -> Result<(), String>,
) -> Result<(), String> {
    store.rt.block_on(async {
        let mut rows = store.conn.query(sql, ()).await.map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
            on_row(&row)?;
        }
        Ok(())
    })
}

fn integer(row: &turso::Row, index: usize) -> Result<i64, String> {
    match row.get_value(index).map_err(|e| e.to_string())? {
        Value::Integer(value) => Ok(value),
        other => Err(format!("expected integer, got {other:?}")),
    }
}

fn blob_key(row: &turso::Row, index: usize) -> Result<[u8; 32], String> {
    match row.get_value(index).map_err(|e| e.to_string())? {
        Value::Blob(bytes) => bytes
            .try_into()
            .map_err(|_| "chunk key is not 32 bytes".to_string()),
        other => Err(format!("expected blob, got {other:?}")),
    }
}

fn main() {
    if let Err(err) = adr02_protocol::main::<Store>() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
