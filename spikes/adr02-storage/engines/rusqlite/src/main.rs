use std::path::Path;
use std::time::Duration;

use adr02_protocol::{Engine, RootRow, SCHEMA, Snapshot};
use rusqlite::{Connection, params};

struct Store {
    conn: Connection,
}

impl Store {
    fn open_path(path: &Path) -> Result<Connection, String> {
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(|e| e.to_string())?;
        if std::env::var_os("ADR02_FULLFSYNC").is_some() {
            conn.pragma_update(None, "fullfsync", "ON")
                .map_err(|e| e.to_string())?;
        }
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| e.to_string())?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        Ok(conn)
    }

    fn rollback_on_err(&self, result: Result<(), String>) -> Result<(), String> {
        if result.is_err() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        result
    }
}

impl Engine for Store {
    fn open(dir: &Path) -> Result<Self, String> {
        Ok(Self {
            conn: Self::open_path(&dir.join("sqlite.db"))?,
        })
    }

    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO chunks(key, bytes) VALUES (?1, ?2)",
                params![key.as_slice(), bytes],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        let result = (|| {
            self.conn.execute(
                "INSERT INTO roots(root_id, stream, seq, chunk_key) VALUES (?1, ?2, ?3, ?4)",
                params![root as i64, stream as i64, seq as i64, key.as_slice()],
            )?;
            self.conn.execute(
                "INSERT INTO events(cursor, root_id) VALUES (?1, ?1)",
                params![root as i64],
            )?;
            self.conn.execute(
                "INSERT INTO outbox(root_id, peer) VALUES (?1, 1)",
                params![root as i64],
            )?;
            self.conn.execute_batch("COMMIT")?;
            Ok(())
        })();
        self.rollback_on_err(result.map_err(|e: rusqlite::Error| e.to_string()))
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let mut snap = Snapshot::default();
        let mut chunks = self
            .conn
            .prepare("SELECT key FROM chunks")
            .map_err(|e| e.to_string())?;
        let rows = chunks
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|e| e.to_string())?;
        for row in rows {
            snap.chunks.push(key_of(row.map_err(|e| e.to_string())?)?);
        }
        let mut roots = self
            .conn
            .prepare("SELECT root_id, stream, seq, chunk_key FROM roots")
            .map_err(|e| e.to_string())?;
        let rows = roots
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, stream, seq, key) = row.map_err(|e| e.to_string())?;
            snap.roots.push(RootRow {
                root_id: id as u64,
                stream: stream as u64,
                seq: seq as u64,
                chunk_key: key_of(key)?,
            });
        }
        let mut events = self
            .conn
            .prepare("SELECT cursor, root_id FROM events")
            .map_err(|e| e.to_string())?;
        let rows = events
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (cursor, id) = row.map_err(|e| e.to_string())?;
            snap.events.push((cursor as u64, id as u64));
        }
        let mut outbox = self
            .conn
            .prepare("SELECT root_id, peer FROM outbox")
            .map_err(|e| e.to_string())?;
        let rows = outbox
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (id, peer) = row.map_err(|e| e.to_string())?;
            snap.outbox.push((id as u64, peer as u64));
        }
        Ok(snap)
    }

    fn probe_native(dir: &Path) -> Result<String, String> {
        let path = dir.join("sqlite.db");
        let first = Self::open_path(&path)?;
        first
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        let second = Self::open_path(&path)?;
        second
            .busy_timeout(Duration::from_millis(0))
            .map_err(|e| e.to_string())?;
        let write = second.execute_batch("BEGIN IMMEDIATE");
        let _ = first.execute_batch("ROLLBACK");
        match write {
            Ok(()) => Ok("second_begin=accepted".into()),
            Err(err) => Ok(format!("second_begin=rejected:{err}")),
        }
    }

    fn identify() -> String {
        let flush = if std::env::var_os("ADR02_FULLFSYNC").is_some() {
            " fullfsync=on"
        } else {
            " fullfsync=default-off"
        };
        format!(
            "rusqlite 0.41.1 bundled sqlite {}{flush}",
            rusqlite::version()
        )
    }
}

fn key_of(bytes: Vec<u8>) -> Result<[u8; 32], String> {
    bytes
        .try_into()
        .map_err(|_| "chunk key is not 32 bytes".to_string())
}

fn main() {
    if let Err(err) = adr02_protocol::main::<Store>() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
