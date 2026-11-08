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
    /// Wall-clock deadline, sampled every 64 steps.
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

    /// Account one reconstruction step. The clock is sampled every 64 steps
    /// so a tight replay loop does not pay a clock read per step.
    pub(crate) fn consume_step(&self, steps: &mut u64) -> Result<(), Error> {
        *steps = steps
            .checked_add(1)
            .ok_or_else(|| Error::ReadCancelled("step counter overflow".into()))?;
        if *steps > self.max_steps {
            return Err(Error::ReadCancelled("step budget exceeded".into()));
        }
        if steps.is_multiple_of(64)
            && let Some(deadline) = self.deadline
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
/// Returns `invalid_encoding` above the frontier cap and SQLite errors.
pub fn closure(conn: &Connection, heads: &[BatchId]) -> Result<Vec<BatchId>, Error> {
    let mut out: Vec<BatchId> = projection::closure_set_pub(conn, heads)?
        .into_iter()
        .collect();
    out.sort_unstable();
    Ok(out)
}

/// Whether `ancestor` lies in the ancestor closure of `descendant` (DAG
/// dominance; arrival order and lamport values never decide).
///
/// # Errors
/// Returns SQLite errors.
pub fn is_ancestor(
    conn: &Connection,
    ancestor: &BatchId,
    descendant: &BatchId,
) -> Result<bool, Error> {
    let set = projection::closure_set_pub(conn, std::slice::from_ref(descendant))?;
    Ok(set.contains(ancestor))
}

/// The native-head mapping of a version's closure: every `native_changes`
/// row whose producer is the epoch seed (NULL) or lies in `closure`, sorted
/// by (epoch, artifact). Post-cut producers never appear, so no future
/// native change can leak into an old view (D1).
///
/// # Errors
/// Returns SQLite errors.
pub fn native_heads(
    conn: &Connection,
    closure: &HashSet<BatchId>,
) -> Result<Vec<NativeHead>, Error> {
    let mut seen: HashSet<NativeHead> = HashSet::new();
    // Chunked VALUES respect the bind-variable budget; producer-NULL seed
    // rows are closure-independent and repeat per chunk, so the set dedups.
    let mut chunks: Vec<Vec<BatchId>> = Vec::new();
    let mut current: Vec<BatchId> = Vec::new();
    for batch in closure {
        current.push(*batch);
        if current.len() == projection::SEED_CHUNK {
            chunks.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(Vec::new());
    }
    for chunk in &chunks {
        // An empty VALUES list is not valid SQL; the seed-only query covers
        // the empty-closure case (e.g. the empty frontier).
        let sql = if chunk.is_empty() {
            "SELECT epoch, artifact_id, producer FROM native_changes WHERE producer IS NULL"
                .to_owned()
        } else {
            let values = projection::values_list(chunk.len());
            format!(
                "SELECT epoch, artifact_id, producer FROM native_changes
                 WHERE producer IS NULL
                    OR producer IN (SELECT column1 FROM (VALUES {values}))"
            )
        };
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

/// Resolve a snapshot pin to its VersionId (D4; the `snapshots` table is
/// populated by P3 — Phase 1 exposes only this read resolution). Global name
/// namespace. `None` when the name is not pinned.
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
