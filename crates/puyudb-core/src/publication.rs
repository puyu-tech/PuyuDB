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

use crate::canon::{self, Item};
use crate::identity::{ArtifactId, BatchId, BranchId, ChunkId, SpaceId, WriterEpoch};
use crate::profile::{self, BatchBody, Record, chunk_id};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chunks (
  id BLOB PRIMARY KEY,
  bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS batches (
  batch_id BLOB PRIMARY KEY,
  writer_epoch BLOB NOT NULL,
  sequence INTEGER NOT NULL,
  lamport INTEGER NOT NULL,
  body BLOB NOT NULL,
  quarantined INTEGER NOT NULL,
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
    /// The batch failed an admission check.
    #[error("admission rejected: {0}")]
    Rejected(String),
    /// A parent or previous batch is not published. Nothing was applied.
    #[error("missing dependency")]
    MissingDependency,
    /// Two batches claim one stream position. Neither stays published.
    #[error("writer equivocation")]
    Equivocation,
    /// Native scope is not checked by this ledger yet.
    #[error("native scope is not checked")]
    NativeUnchecked,
}

/// One signed batch proposed for publication.
///
/// Admission runs here. A successful [`Receipt`] means the batch was stored,
/// not that its native artifacts were hydrated.
#[derive(Clone, Debug)]
pub struct Publication<'a> {
    /// Canonical body.
    pub body: &'a BatchBody,
    /// Key that signed the batch. Must equal `body.public_key`.
    pub signature_key: &'a [u8; 32],
    /// Ed25519 signature over the batch preimage.
    pub signature: &'a [u8; 64],
    /// Staged chunks this publication requires, in order.
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
        Self::open_vfs(dir, None)
    }

    /// Open the ledger through a named SQLite VFS.
    ///
    /// `vfs` is a fault-injection hook. Production callers use [`Store::open`].
    ///
    /// # Errors
    /// Same as [`Store::open`].
    pub fn open_vfs(dir: impl AsRef<Path>, vfs: Option<&str>) -> Result<Self, Error> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("owner.lock"))?;
        owner.try_lock().map_err(|_| Error::Busy)?;
        let db_path = dir.join("space.sqlite");
        let conn = if let Some(vfs) = vfs {
            Connection::open_with_flags_and_vfs(db_path, rusqlite::OpenFlags::default(), vfs)?
        } else {
            Connection::open(db_path)?
        };
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

    /// Admit and publish a signed batch, its chunk references, one event, and one outbox row.
    ///
    /// # Errors
    /// A failed admission or a missing chunk leaves no new visible batch. A
    /// missing parent stays pending. `Ok` means the commit returned under WAL/FULL.
    pub fn publish(&self, publication: &Publication<'_>) -> Result<Receipt, Error> {
        if !publication.body.native_artifacts.is_empty() {
            return Err(Error::NativeUnchecked);
        }
        let encoded = publication.body.encode()?;
        profile::validate_record(Record::BatchBody, &encoded)?;
        let batch_id = profile::verify_batch(
            publication.body,
            publication.signature_key,
            publication.signature,
        )?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            if let Some(cursor) = self.visible_cursor(&batch_id)? {
                self.conn.execute_batch("ROLLBACK")?;
                return Ok(Receipt {
                    batch_id,
                    event_cursor: cursor,
                });
            }
            self.admit_stream(publication.body, &batch_id)?;
            self.admit_parents(publication.body)?;
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
            self.conn.execute(
                "INSERT INTO batches(batch_id, writer_epoch, sequence, lamport, body, quarantined)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                params![
                    batch_id.as_bytes().as_slice(),
                    publication.body.writer_epoch.as_bytes().as_slice(),
                    publication.body.sequence as i64,
                    publication.body.lamport as i64,
                    encoded,
                ],
            )?;
            for (ordinal, chunk) in publication.chunks.iter().enumerate() {
                self.conn.execute(
                    "INSERT INTO batch_chunks(batch_id, ordinal, chunk_id) VALUES (?1, ?2, ?3)",
                    params![
                        batch_id.as_bytes().as_slice(),
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
                params![cursor, batch_id.as_bytes().as_slice()],
            )?;
            self.conn.execute(
                "INSERT INTO outbox(batch_id, peer) VALUES (?1, ?2)",
                params![batch_id.as_bytes().as_slice(), publication.peer as i64],
            )?;
            self.conn.execute_batch("COMMIT")?;
            Ok(Receipt {
                batch_id,
                event_cursor: cursor as u64,
            })
        })();
        if result.is_err() {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
        result
    }

    fn visible_cursor(&self, id: &BatchId) -> Result<Option<u64>, Error> {
        let row: Option<i64> = self
            .conn
            .query_row(
                "SELECT e.cursor FROM events e
                 JOIN batches b ON b.batch_id = e.batch_id
                 WHERE b.batch_id = ?1 AND b.quarantined = 0",
                params![id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .ok();
        Ok(row.map(|cursor| cursor as u64))
    }

    fn admit_stream(&self, body: &BatchBody, batch_id: &BatchId) -> Result<(), Error> {
        let existing: Option<(Vec<u8>, i64)> = self
            .conn
            .query_row(
                "SELECT batch_id, quarantined FROM batches WHERE writer_epoch = ?1 AND sequence = ?2",
                params![
                    body.writer_epoch.as_bytes().as_slice(),
                    body.sequence as i64,
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((stored_id, _)) = existing {
            if stored_id.as_slice() != batch_id.as_bytes().as_slice() {
                self.conn.execute(
                    "UPDATE batches SET quarantined = 1 WHERE writer_epoch = ?1 AND sequence = ?2",
                    params![
                        body.writer_epoch.as_bytes().as_slice(),
                        body.sequence as i64,
                    ],
                )?;
                self.conn.execute_batch("COMMIT")?;
                return Err(Error::Equivocation);
            }
            return Err(Error::Rejected("batch is quarantined".into()));
        }
        if body.sequence == 1 {
            return Ok(());
        }
        let previous_body: Vec<u8> = self
            .conn
            .query_row(
                "SELECT body FROM batches
                 WHERE writer_epoch = ?1 AND sequence = ?2 AND quarantined = 0",
                params![
                    body.writer_epoch.as_bytes().as_slice(),
                    (body.sequence - 1) as i64,
                ],
                |row| row.get(0),
            )
            .map_err(|_| Error::MissingDependency)?;
        let previous = decode_body(&previous_body)?;
        profile::validate_stream_step(&previous, body)?;
        Ok(())
    }

    fn admit_parents(&self, body: &BatchBody) -> Result<(), Error> {
        if body.parents.is_empty() {
            if body.lamport != 1 {
                return Err(Error::Rejected(
                    "a batch without parents has lamport 1".into(),
                ));
            }
            return Ok(());
        }
        let mut max_parent = 0u64;
        for parent in &body.parents {
            let lamport: i64 = self
                .conn
                .query_row(
                    "SELECT lamport FROM batches WHERE batch_id = ?1 AND quarantined = 0",
                    params![parent.as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .map_err(|_| Error::MissingDependency)?;
            max_parent = max_parent.max(lamport as u64);
        }
        if body.lamport < max_parent.saturating_add(1) {
            return Err(Error::Rejected(
                "lamport is below 1 + max(parent lamport)".into(),
            ));
        }
        Ok(())
    }

    /// Whether this batch id is a visible publication.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn is_published(&self, id: &BatchId) -> Result<bool, Error> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM batches WHERE batch_id = ?1 AND quarantined = 0",
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

impl From<canon::Error> for Error {
    fn from(err: canon::Error) -> Self {
        Self::Rejected(err.to_string())
    }
}

impl From<profile::Error> for Error {
    fn from(err: profile::Error) -> Self {
        Self::Rejected(err.to_string())
    }
}

impl From<profile::StreamError> for Error {
    fn from(err: profile::StreamError) -> Self {
        Self::Rejected(err.to_string())
    }
}

fn decode_body(bytes: &[u8]) -> Result<BatchBody, Error> {
    let Item::Array(record) = canon::decode(bytes)? else {
        return Err(Error::Rejected("stored batch is not a record".into()));
    };
    if record.len() != 11 {
        return Err(Error::Rejected("stored batch has the wrong width".into()));
    }
    let space = SpaceId::from_bytes(&bytes_at(&record[1])?)
        .map_err(|err| Error::Rejected(err.to_string()))?;
    let branch = BranchId::from_bytes(&bytes_at(&record[2])?)
        .map_err(|err| Error::Rejected(err.to_string()))?;
    let mut public_key = [0u8; 32];
    public_key.copy_from_slice(&bytes_at(&record[3])?);
    let writer_epoch = WriterEpoch::from_bytes(&bytes_at(&record[4])?)
        .map_err(|err| Error::Rejected(err.to_string()))?;
    let Item::Uint(sequence) = record[5] else {
        return Err(Error::Rejected("stored sequence is not an integer".into()));
    };
    let previous = match &record[6] {
        Item::Null => None,
        Item::Bytes(bytes) => {
            Some(BatchId::from_bytes(bytes).map_err(|err| Error::Rejected(err.to_string()))?)
        }
        _ => return Err(Error::Rejected("stored previous is invalid".into())),
    };
    let Item::Array(parent_items) = &record[7] else {
        return Err(Error::Rejected("stored parents are not a list".into()));
    };
    let mut parents = Vec::with_capacity(parent_items.len());
    for parent in parent_items {
        parents.push(
            BatchId::from_bytes(&bytes_at(parent)?)
                .map_err(|err| Error::Rejected(err.to_string()))?,
        );
    }
    let Item::Uint(lamport) = record[8] else {
        return Err(Error::Rejected("stored lamport is not an integer".into()));
    };
    let Item::Array(native_items) = &record[9] else {
        return Err(Error::Rejected("stored artifacts are not a list".into()));
    };
    let mut native_artifacts = Vec::with_capacity(native_items.len());
    for artifact in native_items {
        native_artifacts.push(
            ArtifactId::from_bytes(&bytes_at(artifact)?)
                .map_err(|err| Error::Rejected(err.to_string()))?,
        );
    }
    let metadata = ArtifactId::from_bytes(&bytes_at(&record[10])?)
        .map_err(|err| Error::Rejected(err.to_string()))?;
    Ok(BatchBody {
        space,
        branch,
        public_key,
        writer_epoch,
        sequence,
        previous,
        parents,
        lamport,
        native_artifacts,
        metadata,
    })
}

fn bytes_at(item: &Item) -> Result<Vec<u8>, Error> {
    match item {
        Item::Bytes(bytes) => Ok(bytes.clone()),
        _ => Err(Error::Rejected("stored field is not bytes".into())),
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

    fn genesis(metadata: u8) -> BatchBody {
        BatchBody {
            space: SpaceId::from_array([1; 16]),
            branch: BranchId::from_array([2; 16]),
            public_key: profile::public_key(&[3; 32]),
            writer_epoch: WriterEpoch::from_array([4; 16]),
            sequence: 1,
            previous: None,
            parents: Vec::new(),
            lamport: 1,
            native_artifacts: Vec::new(),
            metadata: ArtifactId::from_array([metadata; 32]),
        }
    }

    fn signed<'a>(
        body: &'a BatchBody,
        chunks: &'a [ChunkId],
        sig: &'a [u8; 64],
    ) -> Publication<'a> {
        Publication {
            body,
            signature_key: &body.public_key,
            signature: sig,
            chunks,
            peer: 1,
        }
    }

    fn sign(body: &BatchBody) -> [u8; 64] {
        profile::sign_batch(&[3; 32], &body.space, &body.id().expect("id"))
    }

    #[test]
    fn publish_is_visible_after_reopen_and_staged_chunks_are_not() {
        let dir = temp();
        let chunk = b"chunk-bytes";
        let id = chunk_id(chunk);
        let body = genesis(6);
        let signature = sign(&body);
        let store = Store::open(&dir).expect("open");
        assert!(
            store
                .sqlite_version()
                .expect("version")
                .starts_with("3.54.")
        );
        store.stage_chunk(id, chunk).expect("stage");
        assert!(!store.is_published(&body.id().expect("id")).expect("read"));
        assert_eq!(store.staged_without_publication().expect("staged"), 1);
        let receipt = store
            .publish(&signed(&body, &[id], &signature))
            .expect("publish");
        assert_eq!(receipt.event_cursor, 1);
        let again = store
            .publish(&signed(&body, &[id], &signature))
            .expect("idempotent");
        assert_eq!(again.event_cursor, 1);
        drop(store);
        let reopened = Store::open(&dir).expect("reopen");
        assert!(
            reopened
                .is_published(&body.id().expect("id"))
                .expect("read")
        );
        assert_eq!(reopened.staged_without_publication().expect("staged"), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsigned_batch_is_not_published() {
        let dir = temp();
        let body = genesis(6);
        let store = Store::open(&dir).expect("open");
        let err = store
            .publish(&signed(&body, &[], &[0; 64]))
            .expect_err("unsigned");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(!store.is_published(&body.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_parent_stays_pending_and_equivocation_unpublishes_both() {
        let dir = temp();
        let store = Store::open(&dir).expect("open");
        let first = genesis(6);
        let first_sig = sign(&first);
        store
            .publish(&signed(&first, &[], &first_sig))
            .expect("first");

        let mut child = first.clone();
        child.sequence = 2;
        child.lamport = 2;
        child.previous = Some(first.id().expect("id"));
        child.parents = vec![first.id().expect("id")];
        child.metadata = ArtifactId::from_array([8; 32]);
        let child_sig = sign(&child);
        store
            .publish(&signed(&child, &[], &child_sig))
            .expect("child");

        let mut orphan = child.clone();
        orphan.sequence = 3;
        orphan.lamport = 3;
        orphan.previous = Some(child.id().expect("id"));
        orphan.parents = vec![BatchId::from_array([9; 32])];
        orphan.metadata = ArtifactId::from_array([10; 32]);
        let orphan_sig = sign(&orphan);
        let err = store
            .publish(&signed(&orphan, &[], &orphan_sig))
            .expect_err("parent");
        assert!(matches!(err, Error::MissingDependency));

        let mut other = first.clone();
        other.metadata = ArtifactId::from_array([11; 32]);
        let other_sig = sign(&other);
        let err = store
            .publish(&signed(&other, &[], &other_sig))
            .expect_err("equivocation");
        assert!(matches!(err, Error::Equivocation));
        assert!(
            !store
                .is_published(&first.id().expect("id"))
                .expect("first hidden")
        );
        assert!(
            !store
                .is_published(&other.id().expect("id"))
                .expect("other hidden")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_chunk_leaves_no_publication() {
        let dir = temp();
        let body = genesis(6);
        let signature = sign(&body);
        let missing = ChunkId::from_array([9; 32]);
        let store = Store::open(&dir).expect("open");
        let err = store
            .publish(&signed(&body, &[missing], &signature))
            .expect_err("missing");
        assert!(matches!(err, Error::MissingChunk));
        assert!(!store.is_published(&body.id().expect("id")).expect("read"));
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
