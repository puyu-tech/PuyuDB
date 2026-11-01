// SPDX-License-Identifier: Apache-2.0

//! Local publication ledger.
//!
//! This is not admission. The caller still has to validate the batch before
//! calling [`Store::publish`]. The store only makes that decision durable:
//! required chunks are already staged, the batch and its event and outbox row
//! commit together, and the ack returns only after that commit.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params};
use thiserror::Error;

use crate::canon::{self, Item};
use crate::identity::{ArtifactId, BatchId, BranchId, ChunkId, SpaceId, WriterEpoch};
use crate::profile::{self, ArtifactManifest, BatchBody, Record, chunk_id};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS format_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chunks (
  id BLOB PRIMARY KEY,
  bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS artifacts (
  id BLOB PRIMARY KEY,
  kind TEXT NOT NULL,
  raw_length INTEGER NOT NULL,
  raw_digest BLOB NOT NULL,
  manifest BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS artifact_chunks (
  artifact_id BLOB NOT NULL,
  ordinal INTEGER NOT NULL,
  chunk_id BLOB NOT NULL,
  PRIMARY KEY(artifact_id, ordinal),
  FOREIGN KEY(artifact_id) REFERENCES artifacts(id),
  FOREIGN KEY(chunk_id) REFERENCES chunks(id)
);
CREATE INDEX IF NOT EXISTS artifact_chunks_chunk ON artifact_chunks(chunk_id);
CREATE TABLE IF NOT EXISTS batches (
  batch_id BLOB PRIMARY KEY,
  public_key BLOB NOT NULL,
  branch BLOB NOT NULL,
  writer_epoch BLOB NOT NULL,
  sequence INTEGER NOT NULL,
  lamport INTEGER NOT NULL,
  body BLOB NOT NULL,
  quarantined INTEGER NOT NULL,
  UNIQUE(public_key, branch, writer_epoch, sequence)
);
CREATE TABLE IF NOT EXISTS batch_artifacts (
  batch_id BLOB NOT NULL,
  ordinal INTEGER NOT NULL,
  artifact_id BLOB NOT NULL,
  PRIMARY KEY(batch_id, ordinal),
  FOREIGN KEY(batch_id) REFERENCES batches(batch_id),
  FOREIGN KEY(artifact_id) REFERENCES artifacts(id)
);
CREATE TABLE IF NOT EXISTS batch_parents (
  child BLOB NOT NULL,
  parent BLOB NOT NULL,
  PRIMARY KEY(child, parent),
  FOREIGN KEY(child) REFERENCES batches(batch_id),
  FOREIGN KEY(parent) REFERENCES batches(batch_id)
);
CREATE INDEX IF NOT EXISTS batch_parents_parent ON batch_parents(parent);
CREATE TABLE IF NOT EXISTS content_availability (
  artifact_id BLOB PRIMARY KEY,
  state TEXT NOT NULL CHECK(state IN ('resident', 'intentional-missing', 'lost')),
  FOREIGN KEY(artifact_id) REFERENCES artifacts(id)
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
CREATE TABLE IF NOT EXISTS requests (
  caller BLOB NOT NULL,
  key BLOB NOT NULL,
  request_hash BLOB NOT NULL,
  batch_id BLOB,
  event_cursor INTEGER,
  PRIMARY KEY(caller, key),
  FOREIGN KEY(batch_id) REFERENCES batches(batch_id)
);
CREATE TABLE IF NOT EXISTS staged_batches (
  peer INTEGER NOT NULL,
  batch_id BLOB NOT NULL,
  body BLOB NOT NULL,
  received_unix INTEGER NOT NULL,
  expires_unix INTEGER NOT NULL CHECK(expires_unix >= received_unix),
  PRIMARY KEY(peer, batch_id)
);
CREATE INDEX IF NOT EXISTS staged_batches_expiry ON staged_batches(expires_unix);
CREATE TABLE IF NOT EXISTS staged_artifacts (
  peer INTEGER NOT NULL,
  artifact_id BLOB NOT NULL,
  verified_bytes INTEGER NOT NULL CHECK(verified_bytes >= 0),
  received_unix INTEGER NOT NULL,
  expires_unix INTEGER NOT NULL CHECK(expires_unix >= received_unix),
  PRIMARY KEY(peer, artifact_id)
);
CREATE INDEX IF NOT EXISTS staged_artifacts_expiry ON staged_artifacts(expires_unix);
CREATE TABLE IF NOT EXISTS peer_receipts (
  peer INTEGER NOT NULL PRIMARY KEY,
  stored_cursor INTEGER NOT NULL CHECK(stored_cursor >= 0),
  applied_cursor INTEGER NOT NULL CHECK(applied_cursor >= 0 AND applied_cursor <= stored_cursor)
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
    /// A staged chunk exceeds the frozen 256 KiB raw chunk cap.
    #[error("chunk exceeds the 256 KiB raw cap")]
    ChunkTooLarge,
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
    /// The space manifest is unreadable or not a v1 space manifest.
    #[error("space manifest: {0}")]
    Manifest(String),
    /// The manifest or database names an unsupported format or profile.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    /// The manifest and the database name different spaces.
    #[error("manifest and database name different spaces")]
    SpaceMismatch,
    /// SQLite's integrity check failed at open.
    #[error("database failed integrity check: {0}")]
    Corrupt(String),
}

/// Schema major written to `format_meta`. Readers reject any other major.
const SCHEMA_MAJOR: i64 = 1;
/// Schema minor written to `format_meta`. Readers reject a newer minor.
const SCHEMA_MINOR: i64 = 0;
/// Frozen native codec recorded in `format_meta`.
const NATIVE_CODEC: &str = "dt-text-v1";
/// `format` value of the per-space manifest.
const MANIFEST_FORMAT: &str = "puyudb-space-manifest/v1";
/// `storage` profile of the manifest.
const STORAGE_PROFILE: &str = "sqlite-wal-full-v1";
/// `lock` profile of the manifest.
const LOCK_PROFILE: &str = "owner-file-v1";
/// Per-space manifest file name inside the space directory.
const MANIFEST_NAME: &str = "manifest.toml";

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
    /// Artifact manifests this publication binds, in body reference order
    /// (native artifacts in edit order, then the metadata artifact).
    pub artifacts: &'a [ArtifactManifest],
    /// Staged chunks this publication requires: exactly the concatenation of
    /// the artifacts' chunk lists, in order.
    pub chunks: &'a [ChunkId],
    /// Outbox peer for this local publication.
    pub peer: u64,
}

/// Maximum byte length of an idempotency caller or key.
const MAX_IDEMPOTENCY_PART_BYTES: usize = 1024;

/// One row of the `requests` idempotency table.
struct RequestRow {
    hash: Vec<u8>,
    batch: Option<Vec<u8>>,
    cursor: Option<i64>,
}

/// One staged incoming batch from a peer: received but not yet applied
/// (replication apply is a later issue; only the staging schema exists here).
#[derive(Clone, Debug)]
pub struct StagedBatch {
    /// Canonical batch body bytes as received.
    pub body: Vec<u8>,
    /// When the batch was staged.
    pub received_unix: u64,
    /// When this staging entry expires.
    pub expires_unix: u64,
}

/// Hydration state of one artifact, distinct from publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    /// All chunks are resident and verified against the manifest.
    Resident,
    /// Authenticated manifest, payload deliberately not fetched (replication).
    IntentionalMissing,
    /// Expected bytes are known to be gone; distinct from corruption.
    Lost,
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
    space: SpaceId,
    _owner: File,
}

impl Store {
    /// Open or create the ledger for `space` and take the owner lock.
    ///
    /// A missing manifest is created for `space`; an existing manifest must
    /// name the same space, or the open fails with [`Error::SpaceMismatch`].
    ///
    /// # Errors
    /// Returns [`Error::Busy`] when another process holds the lock,
    /// [`Error::UnpatchedSqlite`] when the linked SQLite predates 3.51.3,
    /// [`Error::UnsupportedFormat`]/[`Error::Manifest`] for an unreadable or
    /// unsupported on-disk format, and [`Error::Corrupt`] when the integrity
    /// check fails.
    pub fn open(dir: impl AsRef<Path>, space: &SpaceId) -> Result<Self, Error> {
        Self::open_vfs(dir, None, space)
    }

    /// Open the ledger through a named SQLite VFS.
    ///
    /// `vfs` is a fault-injection hook. Production callers use [`Store::open`].
    ///
    /// # Errors
    /// Same as [`Store::open`].
    pub fn open_vfs(
        dir: impl AsRef<Path>,
        vfs: Option<&str>,
        space: &SpaceId,
    ) -> Result<Self, Error> {
        let dir = dir.as_ref();
        fs::create_dir_all(dir)?;
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("owner.lock"))?;
        // A file scanner can hold a byte-range lock on a file it has just seen
        // created, so a first failure is not proof that another process owns the
        // database. Retry briefly before reporting it as taken.
        let mut tries = 0;
        while owner.try_lock().is_err() {
            tries += 1;
            if tries == 4 {
                return Err(Error::Busy);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let manifest_path = dir.join(MANIFEST_NAME);
        if manifest_path.exists() {
            let existing = read_manifest(&manifest_path)?;
            if existing != *space {
                return Err(Error::SpaceMismatch);
            }
        }
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
        let db_existed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'batches'",
            [],
            |row| row.get(0),
        )?;
        // Schema and the initial format_meta commit together: a kill between
        // two autocommit statements must not leave a fresh database that fails
        // open as corrupt forever.
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let schema_result = (|| {
            conn.execute_batch(SCHEMA)?;
            check_format_meta(&conn, space, db_existed == 1)
        })();
        match schema_result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(err);
            }
        }
        let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if check != "ok" {
            return Err(Error::Corrupt(check));
        }
        // Only the lock owner writes the manifest, and only after the database
        // is validated, so a failed open never leaves a manifest naming a space
        // the database does not.
        if !manifest_path.exists() {
            write_manifest(&manifest_path, space)?;
        }
        Ok(Self {
            conn,
            space: *space,
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

    /// Run `PRAGMA wal_checkpoint(TRUNCATE)`, the WAL-reset path fixed in
    /// SQLite >= 3.51.3. The T04 fault corpus drives this between publications.
    ///
    /// # Errors
    /// Returns a SQLite error if the pragma fails, and [`Error::Rejected`]
    /// when the checkpoint could not run because the WAL was busy.
    pub fn wal_checkpoint_truncate(&self) -> Result<(), Error> {
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(Error::Rejected("wal checkpoint was busy".into()));
        }
        Ok(())
    }

    /// Durably stage one chunk. Idempotent when the same bytes are staged again.
    ///
    /// # Errors
    /// Returns [`Error::ChunkTooLarge`] above the frozen 256 KiB raw cap and
    /// [`Error::ChunkMismatch`] when `id` is not the digest of `bytes`.
    pub fn stage_chunk(&self, id: ChunkId, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > profile::CHUNK_RAW_BYTES {
            return Err(Error::ChunkTooLarge);
        }
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
        self.publish_inner(publication, None)
    }

    /// Publish with an idempotency key.
    ///
    /// The same `(caller, key)` with the same request content returns the
    /// stored receipt, also after a reopen (the unknown-after-commit case). A
    /// different request under the same key is rejected. A request that never
    /// committed leaves no row, so the key stays usable (cancel-before-publish).
    ///
    /// # Errors
    /// Same as [`Store::publish`], plus [`Error::Rejected`] on key reuse.
    pub fn publish_keyed(
        &self,
        caller: &[u8],
        key: &[u8],
        publication: &Publication<'_>,
    ) -> Result<Receipt, Error> {
        if caller.len() > MAX_IDEMPOTENCY_PART_BYTES || key.len() > MAX_IDEMPOTENCY_PART_BYTES {
            return Err(Error::Rejected(
                "idempotency caller or key exceeds the size cap".into(),
            ));
        }
        self.publish_inner(publication, Some((caller, key)))
    }

    fn publish_inner(
        &self,
        publication: &Publication<'_>,
        idempotency: Option<(&[u8], &[u8])>,
    ) -> Result<Receipt, Error> {
        if publication.body.space != self.space {
            return Err(Error::Rejected(
                "batch names a different space than this store".into(),
            ));
        }
        if !publication.body.native_artifacts.is_empty() {
            return Err(Error::NativeUnchecked);
        }
        // SQLite stores INTEGER as a signed i64; reject values that would wrap
        // on the way into the database.
        if publication.body.sequence > i64::MAX as u64
            || publication.body.lamport > i64::MAX as u64
            || publication.peer > i64::MAX as u64
        {
            return Err(Error::Rejected(
                "integer exceeds the storable i64 range".into(),
            ));
        }
        let encoded = publication.body.encode()?;
        profile::validate_record(Record::BatchBody, &encoded)?;
        let batch_id = profile::verify_batch(
            publication.body,
            publication.signature_key,
            publication.signature,
        )?;
        // Artifact admission: every manifest is canonical and in-profile, its
        // ID is exactly the one the signed body commits, and the chunk list is
        // the concatenation of the manifests' chunk lists. Counts are bounded
        // transitively: body cap on artifact references, manifest cap on chunks.
        let mut referenced = publication.body.native_artifacts.clone();
        referenced.push(publication.body.metadata);
        if referenced.len() != publication.artifacts.len() {
            return Err(Error::Rejected(
                "artifact list does not match the batch body".into(),
            ));
        }
        let mut encoded_artifacts = Vec::with_capacity(publication.artifacts.len());
        for (manifest, want) in publication.artifacts.iter().zip(&referenced) {
            let bytes = manifest.encode()?;
            profile::validate_record(Record::ArtifactManifest, &bytes)?;
            let id = manifest.id()?;
            if &id != want {
                return Err(Error::Rejected(
                    "artifact id does not match the batch body".into(),
                ));
            }
            encoded_artifacts.push((id, bytes));
        }
        let expected_chunks: Vec<&ChunkId> = publication
            .artifacts
            .iter()
            .flat_map(|manifest| manifest.chunks.iter())
            .collect();
        if expected_chunks.len() != publication.chunks.len()
            || expected_chunks
                .iter()
                .zip(publication.chunks)
                .any(|(expected, given)| *expected != given)
        {
            return Err(Error::Rejected(
                "chunk list does not match the artifact manifests".into(),
            ));
        }
        let request_hash = idempotency.map(|_| {
            profile::request_hash(
                &encoded,
                publication.signature,
                publication.chunks,
                publication.peer,
            )
        });
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            // The keyed path checks the requests row first: key reuse must be
            // rejected even when the new request's batch is already published,
            // and a stored receipt is only returned while its batch is still
            // visible (a quarantined batch must not be acked again).
            if let (Some((caller, key)), Some(hash)) = (idempotency, &request_hash) {
                let existing: Option<RequestRow> = self
                    .conn
                    .query_row(
                        "SELECT request_hash, batch_id, event_cursor FROM requests
                         WHERE caller = ?1 AND key = ?2",
                        params![caller, key],
                        |row| {
                            Ok(RequestRow {
                                hash: row.get(0)?,
                                batch: row.get(1)?,
                                cursor: row.get(2)?,
                            })
                        },
                    )
                    .ok();
                match existing {
                    Some(row) if row.hash.as_slice() != &hash[..] => {
                        return Err(Error::Rejected(
                            "idempotency key already used for a different request".into(),
                        ));
                    }
                    Some(RequestRow {
                        batch: Some(batch),
                        cursor: Some(cursor),
                        ..
                    }) => {
                        let visible: i64 = self.conn.query_row(
                            "SELECT COUNT(*) FROM batches WHERE batch_id = ?1 AND quarantined = 0",
                            params![batch.as_slice()],
                            |row| row.get(0),
                        )?;
                        if visible != 1 {
                            return Err(Error::Rejected(
                                "idempotent request points to a quarantined batch".into(),
                            ));
                        }
                        self.conn.execute_batch("ROLLBACK")?;
                        return Ok(Receipt {
                            batch_id: BatchId::from_bytes(&batch)
                                .map_err(|err| Error::Corrupt(err.to_string()))?,
                            event_cursor: cursor as u64,
                        });
                    }
                    // Same hash without a committed batch: cancelled before
                    // publish, so this retry may proceed.
                    _ => {}
                }
            }
            if let Some(cursor) = self.visible_cursor(&batch_id)? {
                self.conn.execute_batch("ROLLBACK")?;
                return Ok(Receipt {
                    batch_id,
                    event_cursor: cursor,
                });
            }
            self.admit_stream(publication.body, &batch_id)?;
            self.admit_parents(publication.body)?;
            for (manifest, (artifact_id, manifest_bytes)) in
                publication.artifacts.iter().zip(&encoded_artifacts)
            {
                // Hydration of a local publication: every listed chunk must be
                // staged, hash to its id, and stream to the manifest's raw
                // digest and length. Applied implies resident here.
                let mut total: u64 = 0;
                let mut payload: Vec<Vec<u8>> = Vec::with_capacity(manifest.chunks.len());
                for chunk in &manifest.chunks {
                    let bytes: Vec<u8> = self
                        .conn
                        .query_row(
                            "SELECT bytes FROM chunks WHERE id = ?1",
                            params![chunk.as_bytes().as_slice()],
                            |row| row.get(0),
                        )
                        .map_err(|_| Error::MissingChunk)?;
                    if chunk_id(&bytes) != *chunk {
                        return Err(Error::Rejected(
                            "stored chunk does not hash to its id".into(),
                        ));
                    }
                    total += bytes.len() as u64;
                    payload.push(bytes);
                }
                if total != manifest.raw_length
                    || profile::raw_digest_chunks(&manifest.kind, payload.iter().map(Vec::as_slice))
                        != manifest.raw_digest
                {
                    return Err(Error::Rejected(
                        "artifact payload does not match its manifest".into(),
                    ));
                }
                // Content-addressed rows are shared: a second batch carrying a
                // byte-identical artifact must not hit the PK. Hydration was
                // just re-verified inside this transaction, so ignoring the
                // duplicate is safe.
                self.conn.execute(
                    "INSERT OR IGNORE INTO artifacts(id, kind, raw_length, raw_digest, manifest)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        artifact_id.as_bytes().as_slice(),
                        manifest.kind.as_str(),
                        manifest.raw_length as i64,
                        manifest.raw_digest.as_slice(),
                        manifest_bytes.as_slice(),
                    ],
                )?;
                for (ordinal, chunk) in manifest.chunks.iter().enumerate() {
                    self.conn.execute(
                        "INSERT OR IGNORE INTO artifact_chunks(artifact_id, ordinal, chunk_id) VALUES (?1, ?2, ?3)",
                        params![
                            artifact_id.as_bytes().as_slice(),
                            ordinal as i64,
                            chunk.as_bytes().as_slice(),
                        ],
                    )?;
                }
                self.conn.execute(
                    "INSERT OR IGNORE INTO content_availability(artifact_id, state) VALUES (?1, 'resident')",
                    params![artifact_id.as_bytes().as_slice()],
                )?;
            }
            self.conn.execute(
                "INSERT INTO batches(batch_id, public_key, branch, writer_epoch, sequence, lamport, body, quarantined)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
                params![
                    batch_id.as_bytes().as_slice(),
                    publication.body.public_key.as_slice(),
                    publication.body.branch.as_bytes().as_slice(),
                    publication.body.writer_epoch.as_bytes().as_slice(),
                    publication.body.sequence as i64,
                    publication.body.lamport as i64,
                    encoded,
                ],
            )?;
            for (ordinal, (artifact_id, _)) in encoded_artifacts.iter().enumerate() {
                self.conn.execute(
                    "INSERT INTO batch_artifacts(batch_id, ordinal, artifact_id) VALUES (?1, ?2, ?3)",
                    params![
                        batch_id.as_bytes().as_slice(),
                        ordinal as i64,
                        artifact_id.as_bytes().as_slice(),
                    ],
                )?;
            }
            for parent in &publication.body.parents {
                self.conn.execute(
                    "INSERT INTO batch_parents(child, parent) VALUES (?1, ?2)",
                    params![batch_id.as_bytes().as_slice(), parent.as_bytes().as_slice(),],
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
            if let (Some((caller, key)), Some(hash)) = (idempotency, &request_hash) {
                self.conn.execute(
                    "INSERT INTO requests(caller, key, request_hash, batch_id, event_cursor)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(caller, key) DO UPDATE SET
                       request_hash = excluded.request_hash,
                       batch_id = excluded.batch_id,
                       event_cursor = excluded.event_cursor",
                    params![
                        caller,
                        key,
                        hash.as_slice(),
                        batch_id.as_bytes().as_slice(),
                        cursor,
                    ],
                )?;
            }
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
                "SELECT batch_id, quarantined FROM batches
                 WHERE public_key = ?1 AND branch = ?2 AND writer_epoch = ?3 AND sequence = ?4",
                params![
                    body.public_key.as_slice(),
                    body.branch.as_bytes().as_slice(),
                    body.writer_epoch.as_bytes().as_slice(),
                    body.sequence as i64,
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        if let Some((stored_id, _)) = existing {
            if stored_id.as_slice() != batch_id.as_bytes().as_slice() {
                self.conn.execute(
                    "UPDATE batches SET quarantined = 1
                     WHERE public_key = ?1 AND branch = ?2 AND writer_epoch = ?3 AND sequence = ?4",
                    params![
                        body.public_key.as_slice(),
                        body.branch.as_bytes().as_slice(),
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
                 WHERE public_key = ?1 AND branch = ?2 AND writer_epoch = ?3 AND sequence = ?4 AND quarantined = 0",
                params![
                    body.public_key.as_slice(),
                    body.branch.as_bytes().as_slice(),
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
            if body.previous.is_some() {
                return Err(Error::Rejected(
                    "previous must lie in the parent closure, which is empty".into(),
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
        let expected = max_parent
            .checked_add(1)
            .ok_or_else(|| Error::Rejected("lamport overflow".into()))?;
        if body.lamport != expected {
            return Err(Error::Rejected(
                "lamport must equal 1 + max(parent lamport)".into(),
            ));
        }
        // §13: a declared `previous` must already be in the intent-base
        // closure, i.e. reachable from the parents through the DAG.
        if let Some(previous) = &body.previous
            && !self.closure_contains(&body.parents, previous)?
        {
            return Err(Error::Rejected(
                "previous is not in the parent closure".into(),
            ));
        }
        // Parents must denote a minimal frontier: no parent may be an
        // ancestor of another parent.
        for (index, parent) in body.parents.iter().enumerate() {
            let seeds: Vec<BatchId> = body
                .parents
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != index)
                .map(|(_, p)| *p)
                .collect();
            if self.closure_contains(&seeds, parent)? {
                return Err(Error::Rejected("parents are not a minimal frontier".into()));
            }
        }
        Ok(())
    }

    /// Whether `target` is a member of the ancestor closure of `seeds`
    /// (seeds included), traversed through `batch_parents`.
    fn closure_contains(&self, seeds: &[BatchId], target: &BatchId) -> Result<bool, Error> {
        if seeds.is_empty() {
            return Ok(false);
        }
        let values = (1..=seeds.len())
            .map(|i| format!("(?{i})"))
            .collect::<Vec<_>>()
            .join(",");
        let target_param = seeds.len() + 1;
        let sql = format!(
            "WITH RECURSIVE anc(x) AS (
               SELECT column1 FROM (VALUES {values})
               UNION
               SELECT bp.parent FROM batch_parents bp JOIN anc ON bp.child = anc.x
             )
             SELECT EXISTS(SELECT 1 FROM anc WHERE x = ?{target_param})"
        );
        let mut all: Vec<&[u8]> = seeds
            .iter()
            .map(|seed| seed.as_bytes().as_slice())
            .collect();
        all.push(target.as_bytes().as_slice());
        let found: i64 = self
            .conn
            .query_row(&sql, rusqlite::params_from_iter(all), |row| row.get(0))?;
        Ok(found == 1)
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

    /// Staged chunks that no admitted artifact references.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn staged_without_publication(&self) -> Result<u64, Error> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM chunks c
             WHERE NOT EXISTS (
               SELECT 1 FROM artifact_chunks a WHERE a.chunk_id = c.id
             )",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Hydration state recorded for one artifact.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn artifact_availability(&self, id: &ArtifactId) -> Result<Option<Availability>, Error> {
        let state: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM content_availability WHERE artifact_id = ?1",
                params![id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .ok();
        Ok(match state.as_deref() {
            None => None,
            Some("resident") => Some(Availability::Resident),
            Some("intentional-missing") => Some(Availability::IntentionalMissing),
            Some("lost") => Some(Availability::Lost),
            Some(other) => return Err(Error::Corrupt(format!("unknown availability {other}"))),
        })
    }

    /// The committed receipt for an idempotency key, if one exists.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn receipt_for(&self, caller: &[u8], key: &[u8]) -> Result<Option<Receipt>, Error> {
        // A quarantined batch is not visible, so its stored receipt is not
        // returned either.
        let row: Option<(Vec<u8>, i64)> = self
            .conn
            .query_row(
                "SELECT r.batch_id, r.event_cursor FROM requests r
                 JOIN batches b ON b.batch_id = r.batch_id
                 WHERE r.caller = ?1 AND r.key = ?2
                   AND r.batch_id IS NOT NULL AND b.quarantined = 0",
                params![caller, key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        match row {
            None => Ok(None),
            Some((batch, cursor)) => Ok(Some(Receipt {
                batch_id: BatchId::from_bytes(&batch)
                    .map_err(|err| Error::Corrupt(err.to_string()))?,
                event_cursor: cursor as u64,
            })),
        }
    }

    /// Stage one incoming batch from a peer and return its batch id (the
    /// hash of the body). A duplicate `(peer, batch)` is rejected; the entry
    /// expires `ttl_secs` from now.
    ///
    /// # Errors
    /// Returns [`Error::Rejected`] on a duplicate staging row or an integer
    /// outside the storable i64 range.
    pub fn stage_remote_batch(
        &self,
        peer: u64,
        body: &[u8],
        ttl_secs: u64,
    ) -> Result<BatchId, Error> {
        if peer > i64::MAX as u64 || ttl_secs > i64::MAX as u64 {
            return Err(Error::Rejected(
                "integer exceeds the storable i64 range".into(),
            ));
        }
        if body.len() > profile::MAX_BATCH_BODY_BYTES {
            return Err(Error::Rejected(
                "staged batch body exceeds the profile cap".into(),
            ));
        }
        let id = profile::batch_id(body);
        let now = now_unix()?;
        let expires = now
            .checked_add(ttl_secs)
            .ok_or_else(|| Error::Rejected("expiry overflow".into()))?;
        if expires > i64::MAX as u64 {
            return Err(Error::Rejected(
                "expiry exceeds the storable i64 range".into(),
            ));
        }
        self.conn
            .execute(
                "INSERT INTO staged_batches(peer, batch_id, body, received_unix, expires_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    peer as i64,
                    id.as_bytes().as_slice(),
                    body,
                    now as i64,
                    expires as i64,
                ],
            )
            .map_err(|err| constraint_rejected(err, "staged batch already exists for this peer"))?;
        Ok(id)
    }

    /// The staged row of one incoming batch, if present.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn staged_batch(&self, peer: u64, batch: &BatchId) -> Result<Option<StagedBatch>, Error> {
        let row: Option<(Vec<u8>, i64, i64)> = self
            .conn
            .query_row(
                "SELECT body, received_unix, expires_unix FROM staged_batches
                 WHERE peer = ?1 AND batch_id = ?2",
                params![peer as i64, batch.as_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        Ok(row.map(|(body, received, expires)| StagedBatch {
            body,
            received_unix: received as u64,
            expires_unix: expires as u64,
        }))
    }

    /// Stage verified bytes of one incoming artifact from a peer. A duplicate
    /// `(peer, artifact)` is rejected; the entry expires `ttl_secs` from now.
    ///
    /// # Errors
    /// Returns [`Error::Rejected`] on a duplicate staging row or an integer
    /// outside the storable i64 range.
    pub fn stage_remote_artifact(
        &self,
        peer: u64,
        artifact: &ArtifactId,
        verified_bytes: u64,
        ttl_secs: u64,
    ) -> Result<(), Error> {
        if peer > i64::MAX as u64 || verified_bytes > i64::MAX as u64 || ttl_secs > i64::MAX as u64
        {
            return Err(Error::Rejected(
                "integer exceeds the storable i64 range".into(),
            ));
        }
        let now = now_unix()?;
        let expires = now
            .checked_add(ttl_secs)
            .ok_or_else(|| Error::Rejected("expiry overflow".into()))?;
        if expires > i64::MAX as u64 {
            return Err(Error::Rejected(
                "expiry exceeds the storable i64 range".into(),
            ));
        }
        self.conn
            .execute(
                "INSERT INTO staged_artifacts(peer, artifact_id, verified_bytes, received_unix, expires_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    peer as i64,
                    artifact.as_bytes().as_slice(),
                    verified_bytes as i64,
                    now as i64,
                    expires as i64,
                ],
            )
            .map_err(|err| constraint_rejected(err, "staged artifact already exists for this peer"))?;
        Ok(())
    }

    /// Verified bytes staged for one incoming artifact, if present.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn staged_artifact_bytes(
        &self,
        peer: u64,
        artifact: &ArtifactId,
    ) -> Result<Option<u64>, Error> {
        let bytes: Option<i64> = self
            .conn
            .query_row(
                "SELECT verified_bytes FROM staged_artifacts WHERE peer = ?1 AND artifact_id = ?2",
                params![peer as i64, artifact.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .ok();
        Ok(bytes.map(|bytes| bytes as u64))
    }

    /// Record a peer's stored/applied receipt cursor. Cursors are monotonic
    /// per peer: a backwards movement is rejected, an equal one is an
    /// idempotent receipt.
    ///
    /// # Errors
    /// Returns [`Error::Rejected`] when a cursor moves backwards, violates
    /// `applied <= stored`, or is outside the storable i64 range.
    pub fn record_peer_receipt(
        &self,
        peer: u64,
        stored_cursor: u64,
        applied_cursor: u64,
    ) -> Result<(), Error> {
        if peer > i64::MAX as u64
            || stored_cursor > i64::MAX as u64
            || applied_cursor > i64::MAX as u64
        {
            return Err(Error::Rejected(
                "integer exceeds the storable i64 range".into(),
            ));
        }
        let changed = self
            .conn
            .execute(
                "INSERT INTO peer_receipts(peer, stored_cursor, applied_cursor) VALUES (?1, ?2, ?3)
                 ON CONFLICT(peer) DO UPDATE SET
                   stored_cursor = excluded.stored_cursor,
                   applied_cursor = excluded.applied_cursor
                 WHERE excluded.stored_cursor >= peer_receipts.stored_cursor
                   AND excluded.applied_cursor >= peer_receipts.applied_cursor",
                params![peer as i64, stored_cursor as i64, applied_cursor as i64,],
            )
            .map_err(|err| {
                constraint_rejected(err, "peer receipt violates the cursor constraints")
            })?;
        if changed == 0 {
            return Err(Error::Rejected(
                "peer receipt cursor moved backwards".into(),
            ));
        }
        Ok(())
    }

    /// The stored/applied receipt cursor recorded for one peer.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn peer_receipt(&self, peer: u64) -> Result<Option<(u64, u64)>, Error> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT stored_cursor, applied_cursor FROM peer_receipts WHERE peer = ?1",
                params![peer as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        Ok(row.map(|(stored, applied)| (stored as u64, applied as u64)))
    }

    /// Direct parents recorded for one batch.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn parents_of(&self, batch: &BatchId) -> Result<Vec<BatchId>, Error> {
        let mut rows = self
            .conn
            .prepare("SELECT parent FROM batch_parents WHERE child = ?1")?;
        let ids = rows
            .query_map(params![batch.as_bytes().as_slice()], |row| {
                row.get::<_, Vec<u8>>(0)
            })?
            .collect::<Result<Vec<Vec<u8>>, rusqlite::Error>>()?;
        ids.iter()
            .map(|bytes| BatchId::from_bytes(bytes).map_err(|err| Error::Corrupt(err.to_string())))
            .collect()
    }

    /// Artifact ids one published batch references, in body order.
    ///
    /// # Errors
    /// Returns a SQLite error if the read fails.
    pub fn batch_artifacts(&self, batch: &BatchId) -> Result<Vec<ArtifactId>, Error> {
        let mut rows = self.conn.prepare(
            "SELECT artifact_id FROM batch_artifacts WHERE batch_id = ?1 ORDER BY ordinal",
        )?;
        let ids = rows
            .query_map(params![batch.as_bytes().as_slice()], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                Ok(bytes)
            })?
            .collect::<Result<Vec<Vec<u8>>, rusqlite::Error>>()?;
        ids.iter()
            .map(|bytes| {
                ArtifactId::from_bytes(bytes).map_err(|err| Error::Corrupt(err.to_string()))
            })
            .collect()
    }
}

fn now_unix() -> Result<u64, Error> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| Error::Rejected(format!("system clock before the unix epoch: {err}")))?
        .as_secs())
}

/// Map a constraint violation to a domain rejection; keep other SQLite errors.
fn constraint_rejected(err: rusqlite::Error, message: &'static str) -> Error {
    match err {
        rusqlite::Error::SqliteFailure(ffi, _)
            if ffi.code == rusqlite::ffi::ErrorCode::ConstraintViolation =>
        {
            Error::Rejected(message.into())
        }
        other => Error::Sqlite(other),
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

/// Write the flat v1 space manifest atomically: temp file, fsync, rename.
fn write_manifest(path: &Path, space: &SpaceId) -> Result<(), Error> {
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| Error::Manifest(err.to_string()))?
        .as_secs();
    let text = format!(
        "format = \"{MANIFEST_FORMAT}\"\n\
         space = \"{space}\"\n\
         storage = \"{STORAGE_PROFILE}\"\n\
         lock = \"{LOCK_PROFILE}\"\n\
         created_unix = \"{created}\"\n"
    );
    let tmp = path.with_extension("toml.tmp");
    let mut file = File::create(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    if let Ok(dir) = File::open(path.parent().unwrap_or(Path::new("."))) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Parse the strict flat manifest: exactly the five v1 keys, each once,
/// values double-quoted, no comments or blank lines.
fn read_manifest(path: &Path) -> Result<SpaceId, Error> {
    let text = fs::read_to_string(path).map_err(|err| Error::Manifest(err.to_string()))?;
    if text.contains('\r') {
        return Err(Error::Manifest("CR is not allowed".into()));
    }
    let mut format = None;
    let mut space = None;
    let mut storage = None;
    let mut lock = None;
    let mut created = false;
    for line in text.lines() {
        let (key, value) = line
            .split_once(" = ")
            .and_then(|(key, value)| {
                value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .map(|v| (key, v))
            })
            .ok_or_else(|| Error::Manifest(format!("malformed line: {line:?}")))?;
        let slot = match key {
            "format" => &mut format,
            "space" => &mut space,
            "storage" => &mut storage,
            "lock" => &mut lock,
            "created_unix" => {
                if created {
                    return Err(Error::Manifest("duplicate created_unix".into()));
                }
                created = true;
                value
                    .parse::<u64>()
                    .map_err(|_| Error::Manifest("created_unix is not a number".into()))?;
                continue;
            }
            other => return Err(Error::Manifest(format!("unknown key {other:?}"))),
        };
        if slot.is_some() {
            return Err(Error::Manifest(format!("duplicate key {key:?}")));
        }
        *slot = Some(value.to_string());
    }
    let format = format.ok_or_else(|| Error::Manifest("missing format".into()))?;
    if format != MANIFEST_FORMAT {
        return Err(Error::UnsupportedFormat(format));
    }
    let storage = storage.ok_or_else(|| Error::Manifest("missing storage".into()))?;
    if storage != STORAGE_PROFILE {
        return Err(Error::UnsupportedFormat(storage));
    }
    let lock = lock.ok_or_else(|| Error::Manifest("missing lock".into()))?;
    if lock != LOCK_PROFILE {
        return Err(Error::UnsupportedFormat(lock));
    }
    if !created {
        return Err(Error::Manifest("missing created_unix".into()));
    }
    let space = space.ok_or_else(|| Error::Manifest("missing space".into()))?;
    SpaceId::from_str(&space).map_err(|err| Error::Manifest(err.to_string()))
}

/// Create `format_meta` on a fresh database, or verify it on an existing one.
/// `db_existed` is true when the database file already had a `batches` table;
/// an existing database with an emptied `format_meta` is corruption, not a
/// rebind to the caller's space.
fn check_format_meta(conn: &Connection, space: &SpaceId, db_existed: bool) -> Result<(), Error> {
    let get = |key: &str| -> Result<Option<String>, Error> {
        Ok(conn
            .query_row(
                "SELECT value FROM format_meta WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .ok())
    };
    match get("schema_major")? {
        None => {
            if db_existed {
                return Err(Error::Corrupt("format_meta is missing".into()));
            }
            conn.execute(
                "INSERT INTO format_meta(key, value) VALUES
                 ('schema_major', ?1), ('schema_minor', ?2), ('native_codec', ?3), ('space', ?4)",
                params![
                    SCHEMA_MAJOR.to_string(),
                    SCHEMA_MINOR.to_string(),
                    NATIVE_CODEC,
                    space.to_string(),
                ],
            )?;
        }
        Some(major) => {
            let major: i64 = major
                .parse()
                .map_err(|_| Error::UnsupportedFormat(major.clone()))?;
            if major != SCHEMA_MAJOR {
                return Err(Error::UnsupportedFormat(format!("schema major {major}")));
            }
            let minor = get("schema_minor")?
                .ok_or_else(|| Error::Corrupt("format_meta is missing schema_minor".into()))?;
            let minor: i64 = minor
                .parse()
                .map_err(|_| Error::UnsupportedFormat(minor.clone()))?;
            if minor > SCHEMA_MINOR {
                return Err(Error::UnsupportedFormat(format!("schema minor {minor}")));
            }
            let codec = get("native_codec")?.unwrap_or_default();
            if codec != NATIVE_CODEC {
                return Err(Error::UnsupportedFormat(format!("native codec {codec}")));
            }
            let db_space = get("space")?.ok_or_else(|| Error::Manifest("missing space".into()))?;
            if SpaceId::from_str(&db_space).map_err(|err| Error::Manifest(err.to_string()))?
                != *space
            {
                return Err(Error::SpaceMismatch);
            }
        }
    }
    Ok(())
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

    const SPACE: SpaceId = SpaceId::from_array([1; 16]);

    fn temp() -> PathBuf {
        // The clock alone is not unique across parallel tests on every
        // platform, so a counter keeps each directory distinct.
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "puyu-pub-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("temp");
        path
    }

    /// Frame a payload as an artifact and split it into chunk bytes.
    fn frame(kind: &str, payload: &[u8]) -> (ArtifactManifest, Vec<(ChunkId, Vec<u8>)>) {
        let manifest = ArtifactManifest::for_payload(kind, payload).expect("manifest");
        let chunks = payload
            .chunks(profile::CHUNK_RAW_BYTES)
            .map(|c| (chunk_id(c), c.to_vec()))
            .collect();
        (manifest, chunks)
    }

    /// Stage every chunk and return the ids in manifest order.
    fn stage(store: &Store, chunks: &[(ChunkId, Vec<u8>)]) -> Vec<ChunkId> {
        let mut ids = Vec::with_capacity(chunks.len());
        for (id, bytes) in chunks {
            store.stage_chunk(*id, bytes).expect("stage");
            ids.push(*id);
        }
        ids
    }

    struct Fixture {
        body: BatchBody,
        signature: [u8; 64],
        manifest: ArtifactManifest,
        chunks: Vec<(ChunkId, Vec<u8>)>,
    }

    fn fixture(metadata_byte: u8) -> Fixture {
        let (manifest, chunks) = frame("metadata-v1", &[metadata_byte]);
        let mut body = BatchBody {
            space: SPACE,
            branch: BranchId::from_array([2; 16]),
            public_key: profile::public_key(&[3; 32]),
            writer_epoch: WriterEpoch::from_array([4; 16]),
            sequence: 1,
            previous: None,
            parents: Vec::new(),
            lamport: 1,
            native_artifacts: Vec::new(),
            metadata: ArtifactId::from_array([0; 32]),
        };
        body.metadata = manifest.id().expect("artifact id");
        let signature = sign(&body);
        Fixture {
            body,
            signature,
            manifest,
            chunks,
        }
    }

    /// A child batch of `parent` carrying a fresh one-byte metadata artifact.
    fn child(
        parent: &Fixture,
        sequence: u64,
        lamport: u64,
        metadata_byte: u8,
        parents: Vec<BatchId>,
    ) -> Fixture {
        let (manifest, chunks) = frame("metadata-v1", &[metadata_byte]);
        let mut body = parent.body.clone();
        body.sequence = sequence;
        body.lamport = lamport;
        body.previous = Some(parent.body.id().expect("id"));
        body.parents = parents;
        body.metadata = manifest.id().expect("artifact id");
        let signature = sign(&body);
        Fixture {
            body,
            signature,
            manifest,
            chunks,
        }
    }

    fn publication<'a>(f: &'a Fixture, ids: &'a [ChunkId]) -> Publication<'a> {
        Publication {
            body: &f.body,
            signature_key: &f.body.public_key,
            signature: &f.signature,
            artifacts: std::slice::from_ref(&f.manifest),
            chunks: ids,
            peer: 1,
        }
    }

    fn sign(body: &BatchBody) -> [u8; 64] {
        profile::sign_batch(&[3; 32], &body.space, &body.id().expect("id"))
    }

    #[test]
    fn publish_is_visible_after_reopen_and_staged_chunks_are_not() {
        let dir = temp();
        let f = fixture(6);
        let store = Store::open(&dir, &SPACE).expect("open");
        assert!(
            store
                .sqlite_version()
                .expect("version")
                .starts_with("3.54.")
        );
        let ids = stage(&store, &f.chunks);
        let batch = f.body.id().expect("id");
        let artifact = f.manifest.id().expect("artifact id");
        assert!(!store.is_published(&batch).expect("read"));
        assert_eq!(store.staged_without_publication().expect("staged"), 1);
        let receipt = store.publish(&publication(&f, &ids)).expect("publish");
        assert_eq!(receipt.event_cursor, 1);
        let again = store.publish(&publication(&f, &ids)).expect("idempotent");
        assert_eq!(again.event_cursor, 1);
        assert_eq!(
            store
                .artifact_availability(&artifact)
                .expect("availability"),
            Some(Availability::Resident)
        );
        assert_eq!(
            store.batch_artifacts(&batch).expect("artifacts"),
            vec![artifact]
        );
        drop(store);
        let reopened = Store::open(&dir, &SPACE).expect("reopen");
        assert!(reopened.is_published(&batch).expect("read"));
        assert_eq!(reopened.staged_without_publication().expect("staged"), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_chunk_artifact_publishes_and_verifies() {
        let dir = temp();
        let payload = vec![7u8; profile::CHUNK_RAW_BYTES * 2 + 100];
        let (manifest, chunks) = frame("opaque-baseline-v1", &payload);
        assert_eq!(manifest.chunks.len(), 3);
        let mut body = fixture(6).body;
        body.metadata = manifest.id().expect("artifact id");
        let signature = sign(&body);
        let store = Store::open(&dir, &SPACE).expect("open");
        let ids = stage(&store, &chunks);
        let f = Fixture {
            body,
            signature,
            manifest: manifest.clone(),
            chunks,
        };
        store.publish(&publication(&f, &ids)).expect("publish");
        assert_eq!(
            store
                .artifact_availability(&manifest.id().expect("id"))
                .expect("availability"),
            Some(Availability::Resident)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn oversized_chunk_is_rejected() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let bytes = vec![0u8; profile::CHUNK_RAW_BYTES + 1];
        let err = store
            .stage_chunk(chunk_id(&bytes), &bytes)
            .expect_err("too large");
        assert!(matches!(err, Error::ChunkTooLarge));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsigned_batch_is_not_published() {
        let dir = temp();
        let f = fixture(6);
        let store = Store::open(&dir, &SPACE).expect("open");
        let ids = stage(&store, &f.chunks);
        let err = store
            .publish(&Publication {
                body: &f.body,
                signature_key: &f.body.public_key,
                signature: &[0; 64],
                artifacts: std::slice::from_ref(&f.manifest),
                chunks: &ids,
                peer: 1,
            })
            .expect_err("unsigned");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(!store.is_published(&f.body.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_parent_stays_pending_and_equivocation_unpublishes_both() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let first = fixture(6);
        let first_ids = stage(&store, &first.chunks);
        store
            .publish(&publication(&first, &first_ids))
            .expect("first");

        let second = child(&first, 2, 2, 8, vec![first.body.id().expect("id")]);
        let second_ids = stage(&store, &second.chunks);
        store
            .publish(&publication(&second, &second_ids))
            .expect("child");

        let orphan = child(&second, 3, 3, 10, vec![BatchId::from_array([9; 32])]);
        let orphan_ids = stage(&store, &orphan.chunks);
        let err = store
            .publish(&publication(&orphan, &orphan_ids))
            .expect_err("parent");
        assert!(matches!(err, Error::MissingDependency));

        // Same stream position as `first`, different body: equivocation.
        let (rival_manifest, rival_chunks) = frame("metadata-v1", &[11]);
        let mut rival = first.body.clone();
        rival.metadata = rival_manifest.id().expect("artifact id");
        let rival_sig = sign(&rival);
        let rival_ids = stage(&store, &rival_chunks);
        let err = store
            .publish(&Publication {
                body: &rival,
                signature_key: &rival.public_key,
                signature: &rival_sig,
                artifacts: std::slice::from_ref(&rival_manifest),
                chunks: &rival_ids,
                peer: 1,
            })
            .expect_err("equivocation");
        assert!(matches!(err, Error::Equivocation));
        assert!(
            !store
                .is_published(&first.body.id().expect("id"))
                .expect("first hidden")
        );
        assert!(
            !store
                .is_published(&rival.id().expect("id"))
                .expect("other hidden")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_artifact_chunk_leaves_no_publication() {
        let dir = temp();
        let f = fixture(6);
        let store = Store::open(&dir, &SPACE).expect("open");
        // The chunk ids match the manifest list, but nothing was staged.
        let ids: Vec<ChunkId> = f.manifest.chunks.clone();
        let err = store.publish(&publication(&f, &ids)).expect_err("missing");
        assert!(matches!(err, Error::MissingChunk));
        assert!(!store.is_published(&f.body.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn chunk_list_must_match_the_manifests() {
        let dir = temp();
        let f = fixture(6);
        let store = Store::open(&dir, &SPACE).expect("open");
        let _ = stage(&store, &f.chunks);
        let err = store.publish(&publication(&f, &[])).expect_err("chunks");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(!store.is_published(&f.body.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_id_must_match_the_signed_body() {
        let dir = temp();
        let f = fixture(6);
        let store = Store::open(&dir, &SPACE).expect("open");
        let ids = stage(&store, &f.chunks);
        // Rebind the body to a different artifact id and re-sign.
        let mut body = f.body.clone();
        body.metadata = ArtifactId::from_array([9; 32]);
        let sig = sign(&body);
        let err = store
            .publish(&Publication {
                body: &body,
                signature_key: &body.public_key,
                signature: &sig,
                artifacts: std::slice::from_ref(&f.manifest),
                chunks: &ids,
                peer: 1,
            })
            .expect_err("id mismatch");
        assert!(matches!(err, Error::Rejected(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn payload_that_does_not_match_the_manifest_is_rejected() {
        let dir = temp();
        let (mut manifest, chunks) = frame("metadata-v1", &[6]);
        manifest.raw_digest = [9; 32]; // lie about the content digest
        let mut body = fixture(6).body;
        body.metadata = manifest.id().expect("artifact id");
        let signature = sign(&body);
        let store = Store::open(&dir, &SPACE).expect("open");
        let ids = stage(&store, &chunks);
        let err = store
            .publish(&Publication {
                body: &body,
                signature_key: &body.public_key,
                signature: &signature,
                artifacts: std::slice::from_ref(&manifest),
                chunks: &ids,
                peer: 1,
            })
            .expect_err("digest mismatch");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(!store.is_published(&body.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reserved_artifact_kind_is_rejected() {
        let dir = temp();
        let (mut manifest, chunks) = frame("metadata-v1", &[6]);
        manifest.kind = "archive-coverage-v1".into();
        let mut body = fixture(6).body;
        body.metadata = manifest.id().expect("artifact id");
        let signature = sign(&body);
        let store = Store::open(&dir, &SPACE).expect("open");
        let ids = stage(&store, &chunks);
        let err = store
            .publish(&Publication {
                body: &body,
                signature_key: &body.public_key,
                signature: &signature,
                artifacts: std::slice::from_ref(&manifest),
                chunks: &ids,
                peer: 1,
            })
            .expect_err("reserved kind");
        assert!(matches!(err, Error::Rejected(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_open_is_busy() {
        let dir = temp();
        let _owner = Store::open(&dir, &SPACE).expect("open");
        match Store::open(&dir, &SPACE) {
            Err(Error::Busy) => {}
            Err(err) => panic!("expected busy, got {err}"),
            Ok(_) => panic!("second open succeeded"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_is_written_and_binds_the_space() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let manifest = fs::read_to_string(dir.join("manifest.toml")).expect("manifest");
        assert!(manifest.contains("format = \"puyudb-space-manifest/v1\""));
        assert!(manifest.contains(&format!("space = \"{SPACE}\"")));
        drop(store);
        Store::open(&dir, &SPACE).expect("reopen same space");
        match Store::open(&dir, &SpaceId::from_array([9; 16])) {
            Err(Error::SpaceMismatch) => {}
            Err(err) => panic!("expected space mismatch, got {err}"),
            Ok(_) => panic!("a different space opened the same directory"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_rejects_unknown_content() {
        let dir = temp();
        fs::write(dir.join("manifest.toml"), "not a manifest\n").expect("write");
        assert!(matches!(Store::open(&dir, &SPACE), Err(Error::Manifest(_))));
        fs::write(
            dir.join("manifest.toml"),
            "format = \"puyudb-space-manifest/v1\"\nspace = \"space:00\"\nstorage = \"sqlite-wal-full-v1\"\nlock = \"owner-file-v1\"\ncreated_unix = \"1\"\nextra = \"x\"\n",
        )
        .expect("write");
        assert!(matches!(Store::open(&dir, &SPACE), Err(Error::Manifest(_))));
        fs::write(
            dir.join("manifest.toml"),
            "format = \"other/v9\"\nspace = \"space:0101010101010101010101010101010101\"\nstorage = \"sqlite-wal-full-v1\"\nlock = \"owner-file-v1\"\ncreated_unix = \"1\"\n",
        )
        .expect("write");
        assert!(matches!(
            Store::open(&dir, &SPACE),
            Err(Error::UnsupportedFormat(_))
        ));
        // missing created_unix
        fs::write(
            dir.join("manifest.toml"),
            "format = \"puyudb-space-manifest/v1\"\nspace = \"space:0101010101010101010101010101010101\"\nstorage = \"sqlite-wal-full-v1\"\nlock = \"owner-file-v1\"\n",
        )
        .expect("write");
        assert!(matches!(Store::open(&dir, &SPACE), Err(Error::Manifest(_))));
        // CRLF line endings are not accepted
        fs::write(
            dir.join("manifest.toml"),
            "format = \"puyudb-space-manifest/v1\"\r\nspace = \"space:0101010101010101010101010101010101\"\r\nstorage = \"sqlite-wal-full-v1\"\r\nlock = \"owner-file-v1\"\r\ncreated_unix = \"1\"\r\n",
        )
        .expect("write");
        assert!(matches!(Store::open(&dir, &SPACE), Err(Error::Manifest(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn emptied_format_meta_is_corruption_not_rebind() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        drop(store);
        let conn = Connection::open(dir.join("space.sqlite")).expect("direct open");
        conn.execute("DELETE FROM format_meta", []).expect("wipe");
        drop(conn);
        assert!(matches!(Store::open(&dir, &SPACE), Err(Error::Corrupt(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_schema_major_is_rejected() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        drop(store);
        let conn = Connection::open(dir.join("space.sqlite")).expect("direct open");
        conn.execute(
            "UPDATE format_meta SET value = '99' WHERE key = 'schema_major'",
            [],
        )
        .expect("bump major");
        drop(conn);
        assert!(matches!(
            Store::open(&dir, &SPACE),
            Err(Error::UnsupportedFormat(_))
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn foreign_space_body_is_rejected() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let mut foreign = fixture(6).body;
        foreign.space = SpaceId::from_array([9; 16]);
        let foreign_sig = sign(&foreign);
        let err = store
            .publish(&Publication {
                body: &foreign,
                signature_key: &foreign.public_key,
                signature: &foreign_sig,
                artifacts: &[],
                chunks: &[],
                peer: 1,
            })
            .expect_err("foreign space");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(
            !store
                .is_published(&foreign.id().expect("id"))
                .expect("read")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_writer_keys_are_distinct_streams() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let first = fixture(6);
        let first_ids = stage(&store, &first.chunks);
        store
            .publish(&publication(&first, &first_ids))
            .expect("first");
        // Same writer_epoch bytes and sequence, but a different public key and
        // branch context: a different stream, not an equivocation.
        let (manifest, chunks) = frame("metadata-v1", &[7]);
        let mut other = first.body.clone();
        other.public_key = profile::public_key(&[4; 32]);
        other.metadata = manifest.id().expect("artifact id");
        let other_sig = profile::sign_batch(&[4; 32], &other.space, &other.id().expect("id"));
        let ids = stage(&store, &chunks);
        store
            .publish(&Publication {
                body: &other,
                signature_key: &other.public_key,
                signature: &other_sig,
                artifacts: std::slice::from_ref(&manifest),
                chunks: &ids,
                peer: 1,
            })
            .expect("second writer");
        assert!(
            store
                .is_published(&first.body.id().expect("id"))
                .expect("read")
        );
        assert!(store.is_published(&other.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_key_on_another_branch_is_a_distinct_stream() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let first = fixture(6);
        let first_ids = stage(&store, &first.chunks);
        store
            .publish(&publication(&first, &first_ids))
            .expect("first branch");
        // Same public key, writer epoch and sequence, but a different branch:
        // a distinct stream per spec §13, not an equivocation.
        let (manifest, chunks) = frame("metadata-v1", &[7]);
        let mut other = first.body.clone();
        other.branch = BranchId::from_array([5; 16]);
        other.metadata = manifest.id().expect("artifact id");
        let other_sig = sign(&other);
        let ids = stage(&store, &chunks);
        store
            .publish(&Publication {
                body: &other,
                signature_key: &other.public_key,
                signature: &other_sig,
                artifacts: std::slice::from_ref(&manifest),
                chunks: &ids,
                peer: 1,
            })
            .expect("second branch");
        assert!(
            store
                .is_published(&first.body.id().expect("id"))
                .expect("read")
        );
        assert!(store.is_published(&other.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lamport_must_equal_one_plus_max_parent() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let first = fixture(6);
        let first_ids = stage(&store, &first.chunks);
        store
            .publish(&publication(&first, &first_ids))
            .expect("first");
        let wrong = child(&first, 2, 1000, 8, vec![first.body.id().expect("id")]);
        let wrong_ids = stage(&store, &wrong.chunks);
        let err = store
            .publish(&publication(&wrong, &wrong_ids))
            .expect_err("lamport");
        assert!(matches!(err, Error::Rejected(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Publish `f` and return its batch id.
    fn publish_fixture(store: &Store, f: &Fixture) -> BatchId {
        let ids = stage(store, &f.chunks);
        store
            .publish(&publication(f, &ids))
            .expect("publish fixture");
        f.body.id().expect("id")
    }

    /// A root batch on another branch, so it shares no history with `fixture`.
    fn other_branch_root(byte: u8, branch_byte: u8) -> Fixture {
        let mut f = fixture(byte);
        f.body.branch = BranchId::from_array([branch_byte; 16]);
        f.body.metadata = f.manifest.id().expect("artifact id");
        f.signature = sign(&f.body);
        f
    }

    #[test]
    fn parents_are_recorded_and_previous_must_be_in_the_closure() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let first = fixture(6);
        let first_id = publish_fixture(&store, &first);
        let second = child(&first, 2, 2, 8, vec![first_id]);
        let second_id = publish_fixture(&store, &second);
        assert_eq!(
            store.parents_of(&second_id).expect("parents"),
            vec![first_id]
        );
        // An unrelated root on another branch gives a batch outside every
        // closure that only contains the main-stream history.
        let root_b = other_branch_root(13, 5);
        let root_b_id = publish_fixture(&store, &root_b);
        // previous outside the parent closure is rejected.
        let (manifest, chunks) = frame("metadata-v1", &[15]);
        let mut bad = root_b.body.clone();
        bad.sequence = 2;
        bad.lamport = 2;
        bad.previous = Some(second_id);
        bad.parents = vec![root_b_id];
        bad.metadata = manifest.id().expect("artifact id");
        let bad_sig = sign(&bad);
        let ids = stage(&store, &chunks);
        let err = store
            .publish(&Publication {
                body: &bad,
                signature_key: &bad.public_key,
                signature: &bad_sig,
                artifacts: std::slice::from_ref(&manifest),
                chunks: &ids,
                peer: 1,
            })
            .expect_err("closure");
        assert!(matches!(err, Error::Rejected(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parents_must_form_a_minimal_frontier() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let first = fixture(6);
        let first_id = publish_fixture(&store, &first);
        let second = child(&first, 2, 2, 8, vec![first_id]);
        let second_id = publish_fixture(&store, &second);
        let root_b = other_branch_root(13, 5);
        let root_b_id = publish_fixture(&store, &root_b);
        // A minimal merge frontier: two unrelated heads, previous = one head.
        let root_c = other_branch_root(14, 6);
        let root_c_id = publish_fixture(&store, &root_c);
        let mid = child(&root_c, 2, 2, 15, vec![root_c_id]);
        let mid_id = publish_fixture(&store, &mid);
        let (mm, mc) = frame("metadata-v1", &[16]);
        let mut merge = mid.body.clone();
        merge.sequence = 3;
        merge.lamport = 3; // 1 + max(mid 2, root_b 1)
        merge.previous = Some(mid_id);
        let mut parents = vec![root_b_id, mid_id];
        parents.sort();
        merge.parents = parents;
        merge.metadata = mm.id().expect("artifact id");
        let merge_sig = sign(&merge);
        let merge_ids = stage(&store, &mc);
        store
            .publish(&Publication {
                body: &merge,
                signature_key: &merge.public_key,
                signature: &merge_sig,
                artifacts: std::slice::from_ref(&mm),
                chunks: &merge_ids,
                peer: 1,
            })
            .expect("minimal merge");
        assert_eq!(
            store
                .parents_of(&merge.id().expect("id"))
                .expect("parents")
                .len(),
            2
        );
        // Non-minimal: first is an ancestor of second, so [first, second] is
        // not a frontier. Same stream as first/second keeps the lamport and
        // stream-step checks consistent (both expect 3).
        let (nm, nc) = frame("metadata-v1", &[17]);
        let mut bad = first.body.clone();
        bad.sequence = 3;
        bad.lamport = 3;
        bad.previous = Some(second_id);
        let mut bad_parents = vec![first_id, second_id];
        bad_parents.sort();
        bad.parents = bad_parents;
        bad.metadata = nm.id().expect("artifact id");
        let bad_sig = sign(&bad);
        let bad_ids = stage(&store, &nc);
        let err = store
            .publish(&Publication {
                body: &bad,
                signature_key: &bad.public_key,
                signature: &bad_sig,
                artifacts: std::slice::from_ref(&nm),
                chunks: &bad_ids,
                peer: 1,
            })
            .expect_err("minimal frontier");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(!store.is_published(&bad.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn idempotency_key_returns_the_stored_receipt_and_rejects_reuse() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let f = fixture(6);
        let ids = stage(&store, &f.chunks);
        let caller = b"caller-a";
        let key = b"req-1";
        let first = store
            .publish_keyed(caller, key, &publication(&f, &ids))
            .expect("first");
        let retry = store
            .publish_keyed(caller, key, &publication(&f, &ids))
            .expect("retry");
        assert_eq!(first, retry);
        assert_eq!(
            store.receipt_for(caller, key).expect("receipt"),
            Some(first)
        );
        // A different request under the same key is rejected.
        let second = child(&f, 2, 2, 8, vec![f.body.id().expect("id")]);
        let second_ids = stage(&store, &second.chunks);
        let err = store
            .publish_keyed(caller, key, &publication(&second, &second_ids))
            .expect_err("key reuse");
        assert!(matches!(err, Error::Rejected(_)));
        // A request that failed before commit leaves no row: the key stays
        // usable and the retry publishes normally.
        let third = child(&f, 2, 2, 9, vec![f.body.id().expect("id")]);
        let unstaged: Vec<ChunkId> = third.manifest.chunks.clone();
        let key2 = b"req-2";
        let err = store
            .publish_keyed(caller, key2, &publication(&third, &unstaged))
            .expect_err("missing chunk");
        assert!(matches!(err, Error::MissingChunk));
        assert_eq!(store.receipt_for(caller, key2).expect("none"), None);
        let third_ids = stage(&store, &third.chunks);
        let committed = store
            .publish_keyed(caller, key2, &publication(&third, &third_ids))
            .expect("after cancel");
        assert_eq!(committed.event_cursor, 2);
        // Unknown-after-commit: a reopen still resolves the stored receipt.
        drop(store);
        let store = Store::open(&dir, &SPACE).expect("reopen");
        let again = store
            .publish_keyed(caller, key, &publication(&f, &ids))
            .expect("retry after reopen");
        assert_eq!(again, first);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keyed_reuse_with_a_published_batch_is_rejected_and_quarantine_hides_the_receipt() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let caller = b"caller-a";
        let first = fixture(6);
        let first_ids = stage(&store, &first.chunks);
        let key_a = b"key-a";
        let r1 = store
            .publish_keyed(caller, key_a, &publication(&first, &first_ids))
            .expect("first");
        // A second batch published under a different key must not be accepted
        // under key_a, even though its batch is already published.
        let second = child(&first, 2, 2, 8, vec![first.body.id().expect("id")]);
        let second_ids = stage(&store, &second.chunks);
        store
            .publish_keyed(caller, b"key-b", &publication(&second, &second_ids))
            .expect("second");
        let err = store
            .publish_keyed(caller, key_a, &publication(&second, &second_ids))
            .expect_err("key reuse");
        assert!(matches!(err, Error::Rejected(_)));
        assert_eq!(store.receipt_for(caller, key_a).expect("receipt"), Some(r1));
        // Equivocation quarantines `first`; the stored receipt must stop
        // resolving on both the keyed path and receipt_for.
        let (rm, rc) = frame("metadata-v1", &[11]);
        let mut rival = first.body.clone();
        rival.metadata = rm.id().expect("artifact id");
        let rsig = sign(&rival);
        let rids = stage(&store, &rc);
        let err = store
            .publish(&Publication {
                body: &rival,
                signature_key: &rival.public_key,
                signature: &rsig,
                artifacts: std::slice::from_ref(&rm),
                chunks: &rids,
                peer: 1,
            })
            .expect_err("equivocation");
        assert!(matches!(err, Error::Equivocation));
        let err = store
            .publish_keyed(caller, key_a, &publication(&first, &first_ids))
            .expect_err("quarantined receipt");
        assert!(matches!(err, Error::Rejected(_)));
        assert_eq!(store.receipt_for(caller, key_a).expect("gone"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_batches_may_share_one_content_addressed_artifact() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let first = fixture(6);
        publish_fixture(&store, &first);
        // Same metadata payload on another branch: identical ArtifactId.
        let twin = other_branch_root(6, 5);
        assert_eq!(
            first.manifest.id().expect("id"),
            twin.manifest.id().expect("id")
        );
        let twin_id = publish_fixture(&store, &twin);
        assert!(store.is_published(&twin_id).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wal_reset_version_gate() {
        assert!(sqlite_has_wal_reset_fix("3.51.3"));
        assert!(sqlite_has_wal_reset_fix("3.54.1"));
        assert!(!sqlite_has_wal_reset_fix("3.51.2"));
        assert!(!sqlite_has_wal_reset_fix("3.47.0"));
    }

    #[test]
    fn integers_beyond_i64_are_rejected() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let f = fixture(6);
        let ids = stage(&store, &f.chunks);
        let mut overflow = publication(&f, &ids);
        overflow.peer = u64::MAX;
        let err = store.publish(&overflow).expect_err("peer overflow");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(!store.is_published(&f.body.id().expect("id")).expect("read"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupted_database_fails_open_as_corrupt() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        drop(store);
        // Flip bytes in the middle of the file: the 100-byte header stays
        // valid, so open reaches PRAGMA quick_check and reports corruption
        // instead of failing earlier with "not a database".
        let db = dir.join("space.sqlite");
        let mut file = OpenOptions::new().write(true).open(&db).expect("open db");
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(8192)).expect("seek");
        file.write_all(&[0xFFu8; 64]).expect("corrupt");
        drop(file);
        assert!(matches!(Store::open(&dir, &SPACE), Err(Error::Corrupt(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_tables_exist_and_reject_duplicate_staged_batches() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        let tables: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('staged_batches', 'staged_artifacts', 'peer_receipts')",
                [],
                |row| row.get(0),
            )
            .expect("schema");
        assert_eq!(tables, 3);

        let f = fixture(6);
        let body = f.body.encode().expect("encode");
        let id = store.stage_remote_batch(7, &body, 3600).expect("stage");
        assert_eq!(id, f.body.id().expect("id"));
        // A duplicate (peer, batch) staging row is rejected.
        let err = store
            .stage_remote_batch(7, &body, 3600)
            .expect_err("duplicate");
        assert!(matches!(err, Error::Rejected(_)));
        // The same batch from another peer is a distinct staging row.
        store
            .stage_remote_batch(8, &body, 3600)
            .expect("other peer");
        let staged = store.staged_batch(7, &id).expect("query");
        let staged = staged.expect("present");
        assert_eq!(staged.body, body);
        assert!(staged.expires_unix >= staged.received_unix);
        assert!(
            store
                .staged_batch(7, &BatchId::from_array([9; 32]))
                .expect("none")
                .is_none()
        );

        // Artifact staging records verified bytes and expiry.
        let artifact = f.manifest.id().expect("artifact id");
        store
            .stage_remote_artifact(7, &artifact, 128, 60)
            .expect("artifact");
        assert_eq!(
            store.staged_artifact_bytes(7, &artifact).expect("bytes"),
            Some(128)
        );
        let err = store
            .stage_remote_artifact(7, &artifact, 128, 60)
            .expect_err("duplicate artifact");
        assert!(matches!(err, Error::Rejected(_)));
        assert!(
            store
                .staged_artifact_bytes(7, &ArtifactId::from_array([9; 32]))
                .expect("none")
                .is_none()
        );

        // Staging rows are durable across a reopen.
        drop(store);
        let store = Store::open(&dir, &SPACE).expect("reopen");
        assert!(store.staged_batch(7, &id).expect("query").is_some());
        assert_eq!(
            store.staged_artifact_bytes(7, &artifact).expect("bytes"),
            Some(128)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn peer_receipt_cursors_are_monotonic_per_peer() {
        let dir = temp();
        let store = Store::open(&dir, &SPACE).expect("open");
        store.record_peer_receipt(1, 5, 3).expect("first");
        assert_eq!(store.peer_receipt(1).expect("read"), Some((5, 3)));
        // An equal receipt is idempotent.
        store.record_peer_receipt(1, 5, 3).expect("idempotent");
        // A backwards stored cursor is rejected.
        let err = store
            .record_peer_receipt(1, 4, 3)
            .expect_err("backwards stored");
        assert!(matches!(err, Error::Rejected(_)));
        // A backwards applied cursor is rejected.
        let err = store
            .record_peer_receipt(1, 5, 2)
            .expect_err("backwards applied");
        assert!(matches!(err, Error::Rejected(_)));
        // applied beyond stored violates the CHECK constraint.
        let err = store
            .record_peer_receipt(1, 5, 6)
            .expect_err("applied beyond stored");
        assert!(matches!(err, Error::Rejected(_)));
        // Forward movement is accepted, and peers are independent.
        store.record_peer_receipt(1, 7, 6).expect("forward");
        store.record_peer_receipt(2, 1, 1).expect("other peer");
        assert_eq!(store.peer_receipt(1).expect("read"), Some((7, 6)));
        assert_eq!(store.peer_receipt(2).expect("read"), Some((1, 1)));
        assert_eq!(store.peer_receipt(3).expect("none"), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
