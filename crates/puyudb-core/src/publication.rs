// SPDX-License-Identifier: Apache-2.0

//! Local publication ledger.
//!
//! This is not admission. The caller still has to validate the batch before
//! calling [`Store::publish`]. The store only makes that decision durable:
//! required chunks are already staged, the batch and its event and outbox row
//! commit together, and the ack returns only after that commit.

use std::fs::{self, File, OpenOptions};
use std::path::Path;

use rusqlite::{Connection, params};
use thiserror::Error;

use crate::identity::{BatchId, ChunkId, WriterEpoch};
use crate::profile::chunk_id;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chunks (
  id BLOB PRIMARY KEY,
  bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS batches (
  batch_id BLOB PRIMARY KEY,
  writer_epoch BLOB NOT NULL,
  sequence INTEGER NOT NULL,
  body BLOB NOT NULL,
  UNIQUE(writer_epoch, sequence)
);
CREATE TABLE IF NOT EXISTS batch_chunks (
  batch_id BLOB NOT NULL,
  ordinal INTEGER NOT NULL,
  chunk_id BLOB NOT NULL,
  PRIMARY KEY(batch_id, ordinal),
  FOREIGN KEY(batch_id) REFERENCES batches(batch_id),
  FOREIGN KEY(chunk_id) REFERENCES chunks(id)
);
CREATE TABLE IF NOT EXISTS events (
  cursor INTEGER PRIMARY KEY,
  batch_id BLOB NOT NULL UNIQUE,
  FOREIGN KEY(batch_id) REFERENCES batches(batch_id)
);
CREATE TABLE IF NOT EXISTS outbox (
  batch_id BLOB PRIMARY KEY,
  peer INTEGER NOT NULL,
  FOREIGN KEY(batch_id) REFERENCES batches(batch_id)
);
";

/// Failure of the publication ledger.
#[derive(Debug, Error)]
pub enum Error {
    /// The directory or database could not be opened.
    #[error("store io: {0}")]
    Io(#[from] std::io::Error),
    /// SQLite rejected the operation.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Another process already owns this database.
    #[error("database is owned by another process")]
    Busy,
    /// The linked SQLite is older than the WAL-reset fix.
    #[error("sqlite {0} is older than 3.51.3")]
    UnpatchedSqlite(String),
    /// Staged bytes do not hash to the supplied chunk id.
    #[error("chunk bytes do not match the chunk id")]
    ChunkMismatch,
    /// A required chunk is not staged.
    #[error("chunk is not staged")]
    MissingChunk,
    /// This batch id or stream position is already published.
    #[error("batch or stream position is already published")]
    Duplicate,
}

/// One batch the caller has already decided to publish.
#[derive(Clone, Debug)]
pub struct Publication<'a> {
    /// Batch identity.
    pub batch_id: BatchId,
    /// Writer-stream incarnation. Unique with `sequence`.
    pub writer_epoch: WriterEpoch,
    /// Stream sequence, starting at 1.
    pub sequence: u64,
    /// Canonical body bytes. Not re-validated here.
    pub body: &'a [u8],
    /// Staged chunks this batch requires, in order.
    pub chunks: &'a [ChunkId],
    /// Outbox peer for this local publication.
    pub peer: u64,
}

/// Returned only after the publication commit is durable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// Published batch.
    pub batch_id: BatchId,
    /// Monotonic event cursor assigned in the same commit.
    pub event_cursor: u64,
}

/// One owning writer for a local publication database.
pub struct Store {
    conn: Connection,
    _owner: File,
}

impl Store {
    /// Open or create the ledger and take the owner lock.
    ///
    /// # Errors
    /// Returns [`Error::Busy`] when another process holds the lock, and
    /// [`Error::UnpatchedSqlite`] when the linked SQLite predates 3.51.3.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("owner.lock"))?;
        owner.try_lock().map_err(|_| Error::Busy)?;
        let conn = Connection::open(dir.join("space.sqlite"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        if cfg!(target_vendor = "apple") {
            conn.pragma_update(None, "fullfsync", "ON")?;
        }
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: String = conn.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
        if !sqlite_has_wal_reset_fix(&version) {
            return Err(Error::UnpatchedSqlite(version));
        }
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            _owner: owner,
        })
    }

    /// Linked SQLite version.
    ///
    /// # Errors
    /// Returns a SQLite error if the connection cannot answer.
    pub fn sqlite_version(&self) -> Result<String, Error> {
        Ok(self
            .conn
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))?)
    }

    /// Durably stage one chunk. Idempotent when the same bytes are staged again.
    ///
    /// # Errors
    /// Returns [`Error::ChunkMismatch`] when `id` is not the digest of `bytes`.
    pub fn stage_chunk(&self, id: ChunkId, bytes: &[u8]) -> Result<(), Error> {
        if chunk_id(bytes) != id {
            return Err(Error::ChunkMismatch);
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO chunks(id, bytes) VALUES (?1, ?2)",
            params![id.as_bytes().as_slice(), bytes],
        )?;
        Ok(())
    }

    /// Publish a batch, its chunk references, one event, and one outbox row.
    ///
    /// # Errors
    /// Returns [`Error::MissingChunk`] or [`Error::Duplicate`] without leaving
    /// a visible batch. `Ok` means the commit returned under WAL/FULL.
    pub fn publish(&self, publication: &Publication<'_>) -> Result<Receipt, Error> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            for chunk in publication.chunks {
                let staged: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM chunks WHERE id = ?1",
                    params![chunk.as_bytes().as_slice()],
                    |row| row.get(0),
                )?;
                if staged != 1 {
                    return Err(Error::MissingChunk);
                }
            }
            let inserted = self.conn.execute(
                "INSERT INTO batches(batch_id, writer_epoch, sequence, body) VALUES (?1, ?2, ?3, ?4)",
                params![
                    publication.batch_id.as_bytes().as_slice(),
                    publication.writer_epoch.as_bytes().as_slice(),
                    publication.sequence as i64,
                    publication.body,
                ],
            );
            if inserted.is_err() {
                return Err(Error::Duplicate);
            }
            for (ordinal, chunk) in publication.chunks.iter().enumerate() {
                self.conn.execute(
                    "INSERT INTO batch_chunks(batch_id, ordinal, chunk_id) VALUES (?1, ?2, ?3)",
                    params![
                        publication.batch_id.as_bytes().as_slice(),
                        ordinal as i64,
                        chunk.as_bytes().as_slice(),
                    ],
                )?;
            }
            let cursor: i64 = self.conn.query_row(
                "SELECT COALESCE(MAX(cursor), 0) + 1 FROM events",
                [],
                |row| row.get(0),
            )?;
            self.conn.execute(
                "INSERT INTO events(cursor, batch_id) VALUES (?1, ?2)",
                params![cursor, publication.batch_id.as_bytes().as_slice()],
            )?;
            self.conn.execute(
                "INSERT INTO outbox(batch_id, peer) VALUES (?1, ?2)",
                params![
                    publication.batch_id.as_bytes().as_slice(),
                    publication.peer as i64
                ],
            )?;
            self.conn.execute_batch("COMMIT")?;
            Ok(Receipt {
                batch_id: publication.batch_id,
                event_cursor: cursor as u64,
            })
        })();
        if result.is_err() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        result
    }

    /// Whether this batch id is a visible publication.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn is_published(&self, id: &BatchId) -> Result<bool, Error> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM batches WHERE batch_id = ?1",
            params![id.as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }

    /// Staged chunks that no publication references.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn staged_without_publication(&self) -> Result<u64, Error> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chunks c
             WHERE NOT EXISTS (
               SELECT 1 FROM batch_chunks b WHERE b.chunk_id = c.id
             )",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }
}

fn sqlite_has_wal_reset_fix(version: &str) -> bool {
    let mut parts = version.split(['.', ' ']);
    let major: u32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let minor: u32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let patch: u32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    (major, minor, patch) >= (3, 51, 3)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "puyu-pub-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("temp");
        path
    }

    fn publication<'a>(sequence: u64, chunks: &'a [ChunkId], body: &'a [u8]) -> Publication<'a> {
        Publication {
            batch_id: BatchId::from_array([sequence as u8; 32]),
            writer_epoch: WriterEpoch::from_array([7; 16]),
            sequence,
            body,
            chunks,
            peer: 1,
        }
    }

    #[test]
    fn publish_is_visible_after_reopen_and_staged_chunks_are_not() {
        let dir = temp();
        let chunk = b"chunk-bytes";
        let id = chunk_id(chunk);
        let store = Store::open(&dir).expect("open");
        assert!(
            store
                .sqlite_version()
                .expect("version")
                .starts_with("3.54.")
        );
        store.stage_chunk(id, chunk).expect("stage");
        assert!(
            !store
                .is_published(&publication(1, &[id], b"body").batch_id)
                .expect("read")
        );
        assert_eq!(store.staged_without_publication().expect("staged"), 1);
        let receipt = store
            .publish(&publication(1, &[id], b"body"))
            .expect("publish");
        assert_eq!(receipt.event_cursor, 1);
        drop(store);
        let reopened = Store::open(&dir).expect("reopen");
        assert!(
            reopened
                .is_published(&publication(1, &[id], b"body").batch_id)
                .expect("read")
        );
        assert_eq!(reopened.staged_without_publication().expect("staged"), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_sequence_and_missing_chunk_leave_no_publication() {
        let dir = temp();
        let chunk = b"one";
        let id = chunk_id(chunk);
        let store = Store::open(&dir).expect("open");
        store.stage_chunk(id, chunk).expect("stage");
        store.publish(&publication(1, &[id], b"a")).expect("first");
        let err = store
            .publish(&publication(1, &[id], b"b"))
            .expect_err("duplicate");
        assert!(matches!(err, Error::Duplicate));
        let missing = ChunkId::from_array([9; 32]);
        let err = store
            .publish(&publication(2, &[missing], b"c"))
            .expect_err("missing");
        assert!(matches!(err, Error::MissingChunk));
        assert!(
            !store
                .is_published(&publication(2, &[missing], b"c").batch_id)
                .expect("read")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_open_is_busy() {
        let dir = temp();
        let _owner = Store::open(&dir).expect("open");
        match Store::open(&dir) {
            Err(Error::Busy) => {}
            Err(err) => panic!("expected busy, got {err}"),
            Ok(_) => panic!("second open succeeded"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wal_reset_version_gate() {
        assert!(sqlite_has_wal_reset_fix("3.51.3"));
        assert!(sqlite_has_wal_reset_fix("3.54.1"));
        assert!(!sqlite_has_wal_reset_fix("3.51.2"));
        assert!(!sqlite_has_wal_reset_fix("3.47.0"));
    }
}
