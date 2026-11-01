use std::future::Future;
use std::path::Path;
use std::task::{Context, Poll, Waker};

use adr02_protocol::{Engine, RootRow, SCHEMA, Snapshot};
use libsql::{Builder, Connection, Value};

struct Store {
    conn: Connection,
}

impl Store {
    fn open_path(path: &Path) -> Result<Self, String> {
        let text = path.to_str().ok_or("path is not utf-8")?;
        let db = block_on(Builder::new_local(text).build()).map_err(|e| e.to_string())?;
        let conn = db.connect().map_err(|e| e.to_string())?;
        block_on(conn.execute_batch("PRAGMA journal_mode=WAL")).map_err(|e| e.to_string())?;
        block_on(conn.execute_batch("PRAGMA synchronous=FULL")).map_err(|e| e.to_string())?;
        if std::env::var_os("ADR02_FULLFSYNC").is_some() {
            block_on(conn.execute_batch("PRAGMA fullfsync=ON")).map_err(|e| e.to_string())?;
        }
        block_on(conn.execute_batch("PRAGMA foreign_keys=ON")).map_err(|e| e.to_string())?;
        block_on(conn.execute_batch(SCHEMA)).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    fn exec(&self, sql: &str, params: Vec<Value>) -> Result<(), String> {
        block_on(self.conn.execute(sql, libsql::params_from_iter(params)))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

impl Engine for Store {
    fn open(dir: &Path) -> Result<Self, String> {
        Self::open_path(&dir.join("libsql.db"))
    }

    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String> {
        self.exec(
            "INSERT OR IGNORE INTO chunks(key, bytes) VALUES (?1, ?2)",
            vec![Value::Blob(key.to_vec()), Value::Blob(bytes.to_vec())],
        )
    }

    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String> {
        block_on(self.conn.execute_batch("BEGIN IMMEDIATE")).map_err(|e| e.to_string())?;
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
            block_on(self.conn.execute_batch("COMMIT"))
                .map(|_| ())
                .map_err(|e| e.to_string())
        })();
        if result.is_err() {
            let _ = block_on(self.conn.execute_batch("ROLLBACK"));
        }
        result
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let mut snap = Snapshot::default();
        query(&self.conn, "SELECT key FROM chunks", |row| {
            snap.chunks.push(blob_key(row, 0)?);
            Ok(())
        })?;
        query(
            &self.conn,
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
        query(&self.conn, "SELECT cursor, root_id FROM events", |row| {
            snap.events
                .push((integer(row, 0)? as u64, integer(row, 1)? as u64));
            Ok(())
        })?;
        query(&self.conn, "SELECT root_id, peer FROM outbox", |row| {
            snap.outbox
                .push((integer(row, 0)? as u64, integer(row, 1)? as u64));
            Ok(())
        })?;
        Ok(snap)
    }

    fn probe_native(dir: &Path) -> Result<String, String> {
        let path = dir.join("libsql.db");
        let first = Self::open_path(&path)?;
        block_on(first.conn.execute_batch("BEGIN IMMEDIATE")).map_err(|e| e.to_string())?;
        let second = Self::open_path(&path)?;
        let _ = block_on(second.conn.execute_batch("PRAGMA busy_timeout=0"));
        let write = block_on(second.conn.execute_batch("BEGIN IMMEDIATE"));
        let _ = block_on(first.conn.execute_batch("ROLLBACK"));
        match write {
            Ok(_) => Ok("second_begin=accepted".into()),
            Err(err) => Ok(format!("second_begin=rejected:{err}")),
        }
    }

    fn identify() -> String {
        match (|| {
            let db = block_on(Builder::new_local(":memory:").build()).map_err(|e| e.to_string())?;
            let conn = db.connect().map_err(|e| e.to_string())?;
            let mut rows =
                block_on(conn.query("SELECT sqlite_version()", ())).map_err(|e| e.to_string())?;
            let row = block_on(rows.next())
                .map_err(|e| e.to_string())?
                .ok_or("no sqlite_version row")?;
            let version: String = row.get(0).map_err(|e| e.to_string())?;
            Ok::<_, String>(version)
        })() {
            Ok(version) => {
                let flush = if std::env::var_os("ADR02_FULLFSYNC").is_some() {
                    " fullfsync=on"
                } else {
                    " fullfsync=default-off"
                };
                format!("libsql 0.10.0 sqlite {version}{flush}")
            }
            Err(err) => format!("libsql 0.10.0 ({err})"),
        }
    }
}

fn query(
    conn: &Connection,
    sql: &str,
    mut on_row: impl FnMut(&libsql::Row) -> Result<(), String>,
) -> Result<(), String> {
    let mut rows = block_on(conn.query(sql, ())).map_err(|e| e.to_string())?;
    while let Some(row) = block_on(rows.next()).map_err(|e| e.to_string())? {
        on_row(&row)?;
    }
    Ok(())
}

fn integer(row: &libsql::Row, index: i32) -> Result<i64, String> {
    row.get(index).map_err(|e| e.to_string())
}

fn blob_key(row: &libsql::Row, index: i32) -> Result<[u8; 32], String> {
    let bytes: Vec<u8> = row.get(index).map_err(|e| e.to_string())?;
    bytes
        .try_into()
        .map_err(|_| "chunk key is not 32 bytes".to_string())
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => {
            panic!("libsql local future returned Pending; the core API was not ready")
        }
    }
}

fn main() {
    if let Err(err) = adr02_protocol::main::<Store>() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
