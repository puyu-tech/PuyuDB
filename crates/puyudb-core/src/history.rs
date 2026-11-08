// SPDX-License-Identifier: Apache-2.0

//! History queries and the §6.3 frontier/branch/snapshot tables (I08 P1).
//!
//! Every version read resolves a VersionId through `closure(H)` over the
//! batch DAG (D1), never through arrival order or "current minus replay".
//! The `frontiers`/`branches`/`snapshots` tables are derived state maintained
//! inside the publication commit (§5.7 step: update branch frontier), not a
//! new write surface; canonical history alone survives their loss (D3).

use std::collections::HashSet;
use std::time::Instant;

use rusqlite::Connection;

use crate::identity::{ArtifactId, BatchId, BranchId, EpochId, SpaceId, VersionId};
use crate::projection::{self, Error as ProjectionError};
use crate::publication::Error;
use crate::version;

/// History tables added on top of the I04/I05 schema (§6.3).
///
/// Snapshot names use the global namespace (D4 default; plan Q3 pending a
/// possible maintainer veto — swapping to per-branch names touches only this
/// schema line and the read resolution).
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS frontiers (
  version_id BLOB PRIMARY KEY,
  space BLOB NOT NULL,
  body BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS branches (
  branch BLOB PRIMARY KEY,
  initial_version BLOB NOT NULL,
  current_version BLOB NOT NULL,
  FOREIGN KEY(initial_version) REFERENCES frontiers(version_id),
  FOREIGN KEY(current_version) REFERENCES frontiers(version_id)
);
CREATE TABLE IF NOT EXISTS snapshots (
  name TEXT PRIMARY KEY,
  version_id BLOB NOT NULL,
  created_unix INTEGER NOT NULL,
  FOREIGN KEY(version_id) REFERENCES frontiers(version_id)
);
";

fn encoding(detail: impl Into<String>) -> Error {
    Error::Projection(ProjectionError::Encoding(detail.into()))
}

fn corrupt(detail: impl Into<String>) -> Error {
    Error::Corrupt(detail.into())
}

fn version_id_from_bytes(bytes: &[u8]) -> Result<VersionId, Error> {
    VersionId::from_bytes(bytes).map_err(|err| corrupt(err.to_string()))
}

fn native_head_row(
    epoch: Vec<u8>,
    artifact: Vec<u8>,
    producer: Option<Vec<u8>>,
) -> Result<NativeHead, Error> {
    Ok(NativeHead {
        epoch: EpochId::from_bytes(&epoch).map_err(|err| corrupt(err.to_string()))?,
        artifact: ArtifactId::from_bytes(&artifact).map_err(|err| corrupt(err.to_string()))?,
        producer: producer
            .map(|bytes| BatchId::from_bytes(&bytes).map_err(|err| corrupt(err.to_string())))
            .transpose()?,
    })
}

/// One `native_changes` mapping entry visible in a version's closure
/// (§6.3: epoch/native hash → artifact; a NULL producer is the epoch-owned
/// seed).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NativeHead {
    /// Text epoch the change belongs to.
    pub epoch: EpochId,
    /// Native-change artifact.
    pub artifact: ArtifactId,
    /// Producer batch, or `None` for the epoch-owned seed.
    pub producer: Option<BatchId>,
}

/// Per-read budget for historical reconstruction (D5, blueprint §5.8): step,
/// output-byte, and wall-clock limits. A read that exceeds any limit returns
/// the typed cancellation error, never a partial "latest" fallback.
#[derive(Debug, Clone, Copy)]
pub struct ReadBudget {
    /// Maximum reconstruction steps (applied native changes).
    pub max_steps: u64,
    /// Maximum output bytes of one read result.
    pub max_output_bytes: u64,
    /// Wall-clock deadline, checked on every replay step.
    pub deadline: Option<Instant>,
}

impl ReadBudget {
    /// Fail if the wall-clock deadline has already passed.
    pub(crate) fn check_deadline(&self) -> Result<(), Error> {
        if let Some(deadline) = self.deadline
            && Instant::now() >= deadline
        {
            return Err(Error::ReadCancelled("read deadline exceeded".into()));
        }
        Ok(())
    }

    /// Account one reconstruction step. The deadline is checked on every
    /// step: `Instant::now` is a vDSO call, negligible against the merge it
    /// guards, and a sampled deadline can overshoot by up to 63 steps.
    pub(crate) fn consume_step(&self, steps: &mut u64) -> Result<(), Error> {
        *steps = steps
            .checked_add(1)
            .ok_or_else(|| Error::ReadCancelled("step counter overflow".into()))?;
        if *steps > self.max_steps {
            return Err(Error::ReadCancelled("step budget exceeded".into()));
        }
        if let Some(deadline) = self.deadline
            && Instant::now() >= deadline
        {
            return Err(Error::ReadCancelled("read deadline exceeded".into()));
        }
        Ok(())
    }

    /// Account one output byte of a read result.
    pub(crate) fn consume_output(&self, bytes: u64) -> Result<(), Error> {
        if bytes > self.max_output_bytes {
            return Err(Error::ReadCancelled("output byte budget exceeded".into()));
        }
        Ok(())
    }
}

/// The closed ancestor set of `heads`, seeds included, sorted (spec §5.3
/// `closure(H)`; computed over the DAG, quarantined batches excluded).
///
/// # Errors
/// Returns `invalid_encoding` above the frontier cap, for a non-visible head,
/// and SQLite errors.
pub fn closure(conn: &Connection, heads: &[BatchId]) -> Result<Vec<BatchId>, Error> {
    // Gate visibility first (spec §5: a missing head is never silently
    // dropped into a smaller closure).
    projection::check_cut_pub(conn, heads)?;
    let mut out: Vec<BatchId> = projection::closure_set_pub(conn, heads)?
        .into_iter()
        .collect();
    out.sort_unstable();
    Ok(out)
}

/// Whether `ancestor` lies in the ancestor closure of `descendant` (DAG
/// dominance; arrival order and lamport values never decide). Both ids must
/// be visible publications; a quarantined or unknown id is
/// [`Error::MissingVersion`], never a silently smaller answer.
///
/// # Errors
/// Returns [`Error::MissingVersion`] for a non-visible id and SQLite errors.
pub fn is_ancestor(
    conn: &Connection,
    ancestor: &BatchId,
    descendant: &BatchId,
) -> Result<bool, Error> {
    for id in [ancestor, descendant] {
        let visible: i64 = conn.query_row(
            "SELECT COUNT(*) FROM batches WHERE batch_id = ?1 AND quarantined = 0",
            [id.as_bytes().as_slice()],
            |row| row.get(0),
        )?;
        if visible != 1 {
            return Err(Error::MissingVersion);
        }
    }
    let set = projection::closure_set_pub(conn, std::slice::from_ref(descendant))?;
    Ok(set.contains(ancestor))
}

/// Split a set into owned chunks of at most `SEED_CHUNK` for VALUES lists.
fn chunked<T: Copy>(set: &HashSet<T>) -> Vec<Vec<T>> {
    let mut chunks: Vec<Vec<T>> = Vec::new();
    let mut current: Vec<T> = Vec::new();
    for item in set {
        current.push(*item);
        if current.len() == projection::SEED_CHUNK {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Epoch ids whose declaring `Field::Epoch` write lies in `closure`. A seed
/// row (producer NULL) belongs to an old view only when its epoch is declared
/// in that view's closure — a seed registered later for a post-cut epoch must
/// not leak into the old version's native-head listing (D1).
pub(crate) fn declared_epochs(
    conn: &Connection,
    closure: &HashSet<BatchId>,
) -> Result<HashSet<EpochId>, Error> {
    let mut out = HashSet::new();
    let epoch_tag = projection::Field::Epoch.tag();
    for chunk in chunked(closure) {
        let values = projection::values_list(chunk.len());
        let sql = format!(
            "SELECT fw.field, fw.value FROM field_writes fw
             JOIN batches b ON b.batch_id = fw.batch_id AND b.quarantined = 0
             WHERE fw.batch_id IN (SELECT column1 FROM (VALUES {values}))"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(projection::seed_params(&chunk)),
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;
        for row in rows {
            let (field, value) = row?;
            if field as u64 != epoch_tag {
                continue;
            }
            let descriptor =
                projection::decode_epoch_value_pub(&value).map_err(Error::Projection)?;
            let id = descriptor
                .id()
                .map_err(|err| Error::Projection(projection::Error::Profile(err)))?;
            out.insert(id);
        }
    }
    Ok(out)
}

/// The native-head mapping of a version's closure: every `native_changes`
/// row whose producer lies in `closure`, plus the seed row (producer NULL)
/// of each epoch declared in `closure`, sorted by (epoch, artifact). Post-cut
/// producers and post-cut epochs never appear, so no future native change or
/// seed can leak into an old view (D1).
///
/// # Errors
/// Returns SQLite errors and profile errors for undecodable rows.
pub fn native_heads(
    conn: &Connection,
    closure: &HashSet<BatchId>,
) -> Result<Vec<NativeHead>, Error> {
    let mut seen: HashSet<NativeHead> = HashSet::new();
    let chunks = chunked(closure);
    // Non-seed rows: producer in the closure (chunked VALUES respect the
    // bind-variable budget).
    for chunk in &chunks {
        let values = projection::values_list(chunk.len());
        let sql = format!(
            "SELECT epoch, artifact_id, producer FROM native_changes
             WHERE producer IN (SELECT column1 FROM (VALUES {values}))"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(projection::seed_params(chunk)),
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )?;
        for row in rows {
            let (epoch, artifact, producer) = row?;
            seen.insert(native_head_row(epoch, artifact, producer)?);
        }
    }
    // Seed rows: only for epochs declared inside this closure.
    let epochs = declared_epochs(conn, closure)?;
    for chunk in chunked(&epochs) {
        let values = projection::values_list(chunk.len());
        let sql = format!(
            "SELECT epoch, artifact_id, producer FROM native_changes
             WHERE producer IS NULL AND epoch IN (SELECT column1 FROM (VALUES {values}))"
        );
        let params: Vec<&[u8]> = chunk.iter().map(|e| e.as_bytes().as_slice()).collect();
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(params), |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
            ))
        })?;
        for row in rows {
            let (epoch, artifact, producer) = row?;
            seen.insert(native_head_row(epoch, artifact, producer)?);
        }
    }
    let mut out: Vec<NativeHead> = seen.into_iter().collect();
    out.sort_unstable();
    Ok(out)
}

/// The branch's current frontier VersionId (§6.3 `branches`). Reads the
/// derived table; for a pre-I08 space with no row yet, the frontier is
/// computed from the visible heads on read and the row appears at the next
/// publication on the branch. `None` when the branch has no visible batch.
///
/// # Errors
/// Returns SQLite errors and `invalid_encoding` above the frontier cap.
pub fn branch_current_version(
    conn: &Connection,
    space: &SpaceId,
    branch: &BranchId,
) -> Result<Option<VersionId>, Error> {
    let row: Option<Vec<u8>> = conn
        .query_row(
            "SELECT current_version FROM branches WHERE branch = ?1",
            [branch.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .ok();
    if let Some(bytes) = row {
        return Ok(Some(version_id_from_bytes(&bytes)?));
    }
    let heads = projection::branch_heads(conn, branch)?;
    if heads.is_empty() {
        return Ok(None);
    }
    let frontier = version::normalize(conn, space, &heads)?;
    Ok(Some(
        frontier.id().map_err(|err| encoding(err.to_string()))?,
    ))
}

/// The branch's initial VersionId (§8.3 `initial_version`, immutable once
/// set): the captured base of the branch's genesis batch, stored at first
/// publication. `None` for an unknown branch.
///
/// # Errors
/// Returns SQLite errors.
pub fn branch_initial_version(
    conn: &Connection,
    branch: &BranchId,
) -> Result<Option<VersionId>, Error> {
    let row: Option<Vec<u8>> = conn
        .query_row(
            "SELECT initial_version FROM branches WHERE branch = ?1",
            [branch.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .ok();
    row.map(|bytes| version_id_from_bytes(&bytes)).transpose()
}

/// Resolve a snapshot pin to its VersionId (D4; global name namespace).
/// `None` when the name is not pinned.
///
/// # Errors
/// Returns SQLite errors.
pub fn snapshot_version(conn: &Connection, name: &str) -> Result<Option<VersionId>, Error> {
    let row: Option<Vec<u8>> = conn
        .query_row(
            "SELECT version_id FROM snapshots WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .ok();
    row.map(|bytes| version_id_from_bytes(&bytes)).transpose()
}

/// One named snapshot pin (D4: name → VersionId, immutable entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPin {
    /// Global snapshot name.
    pub name: String,
    /// Pinned version.
    pub version: VersionId,
    /// When the pin was created (unix seconds).
    pub created_unix: u64,
}

/// Pin `name` to `version` (D4). Idempotent when the name already pins the
/// SAME version; a different version under the same name is rejected with
/// `precondition_failed` (CAS on name take-over, §14) — never silently
/// re-pointed. The version must resolve (visible heads), else
/// [`Error::MissingVersion`]. Runs inside the caller's transaction.
///
/// Returns `Ok(true)` when a new pin was written, `Ok(false)` when the
/// identical pin already existed.
///
/// # Errors
/// Returns `invalid_encoding` for an empty or over-cap name,
/// [`Error::Precondition`] on name take-over with a different version, and
/// [`Error::MissingVersion`] for an unresolvable version.
pub fn create_snapshot(
    conn: &Connection,
    space: &SpaceId,
    name: &str,
    version: &VersionId,
    now_unix: u64,
) -> Result<bool, Error> {
    if name.is_empty() || name.len() > crate::checkpoint::MAX_NAME_BYTES {
        return Err(encoding("snapshot name is empty or exceeds the cap"));
    }
    // The pinned version must resolve: visible heads, stored frontier object.
    version::resolve(conn, space, version)?;
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO snapshots(name, version_id, created_unix) VALUES (?1, ?2, ?3)",
        rusqlite::params![name, version.as_bytes().as_slice(), now_unix as i64],
    )?;
    if inserted == 1 {
        return Ok(true);
    }
    let existing: Option<Vec<u8>> = conn
        .query_row(
            "SELECT version_id FROM snapshots WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .ok();
    match existing {
        Some(bytes) if version_id_from_bytes(&bytes)? == *version => Ok(false),
        // §14 CAS: a name take-over with a different version fails.
        _ => Err(Error::Precondition(format!(
            "snapshot name {name:?} already pins a different version"
        ))),
    }
}

/// All snapshot pins, sorted by name. Read-only.
///
/// # Errors
/// Returns SQLite errors.
pub fn list_snapshots(conn: &Connection) -> Result<Vec<SnapshotPin>, Error> {
    let mut statement =
        conn.prepare("SELECT name, version_id, created_unix FROM snapshots ORDER BY name")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (name, version, created) = row?;
        out.push(SnapshotPin {
            name,
            version: version_id_from_bytes(&version)?,
            created_unix: u64::try_from(created).map_err(|_| corrupt("negative created_unix"))?,
        });
    }
    Ok(out)
}

/// Maintain the derived `frontiers`/`branches` rows inside the publication
/// transaction, after the batch row exists. The new branch frontier is
/// normalized (producer side) and recorded immutably; on the branch's first
/// row the initial version is the genesis batch's parent cut — the captured
/// base per §5.3/§13 (empty frontier for the very first batch of a space).
pub(crate) fn record_branch_frontier(
    conn: &Connection,
    space: &SpaceId,
    branch: &BranchId,
    parents: &[BatchId],
) -> Result<(), Error> {
    let heads = projection::branch_heads(conn, branch)?;
    let current_frontier = version::normalize(conn, space, &heads)?;
    let current = version::record_frontier(conn, &current_frontier)?;
    let existing: Option<Vec<u8>> = conn
        .query_row(
            "SELECT current_version FROM branches WHERE branch = ?1",
            [branch.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .ok();
    if existing.is_none() {
        let initial_frontier = version::normalize(conn, space, parents)?;
        let initial = version::record_frontier(conn, &initial_frontier)?;
        conn.execute(
            "INSERT INTO branches(branch, initial_version, current_version) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                branch.as_bytes().as_slice(),
                initial.as_bytes().as_slice(),
                current.as_bytes().as_slice(),
            ],
        )?;
    } else {
        conn.execute(
            "UPDATE branches SET current_version = ?1 WHERE branch = ?2",
            rusqlite::params![current.as_bytes().as_slice(), branch.as_bytes().as_slice()],
        )?;
    }
    Ok(())
}
