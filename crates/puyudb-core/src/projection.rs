// SPDX-License-Identifier: Apache-2.0

//! Metadata MV-register storage and evaluation (I05 P1–P2).
//!
//! Normative source: `PUYUDB-BLUEPRINT.md` §5.6, §6.3 and
//! `docs/spec/canonical-profile-v1.md` §7. A metadata artifact's operations are
//! causal multi-value register writes: each field write has identity
//! `(batch_id, operation_index)`, a write supersedes the writes to that field in
//! its causal past (including earlier operations in its own batch), concurrent
//! maximal writes remain alternatives, and alternatives are reported in stable
//! bytewise identity order with the smallest identity as display representative.
//!
//! Storage is the `field_writes` table ("metadata truth extracted from immutable
//! batches", §6.3); evaluation at a cut recomputes maxima from history, so the
//! result never depends on arrival order. Register evaluation is O(closure x
//! writes) per read initially; §5.8 defers indexed causal dominance, so that
//! indexing is deliberately not built yet.
//!
//! All closure walks happen inside SQL recursive CTEs keyed by the seed list
//! only: the bundled SQLite accepts at most 32,766 bind variables per
//! statement, so seed lists are chunked well below that and the (unbounded)
//! ancestor closure is never turned into bind parameters.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;
use thiserror::Error;

use crate::canon::{self, Item};
use crate::identity::{ArtifactId, BatchId, FileId};
use crate::profile::{
    self, EpochDescriptor, EpochKind, LiveTag, MetadataArtifact, ModeTag, Operation, Path,
    WriteIdent,
};

/// Projection tables added on top of the I04 ledger schema.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS field_writes (
  file BLOB NOT NULL,
  field INTEGER NOT NULL,
  op_tag INTEGER NOT NULL,
  batch_id BLOB NOT NULL,
  op_index INTEGER NOT NULL,
  value BLOB NOT NULL,
  PRIMARY KEY(file, field, batch_id, op_index),
  FOREIGN KEY(batch_id) REFERENCES batches(batch_id)
);
CREATE INDEX IF NOT EXISTS field_writes_batch ON field_writes(batch_id);
";

/// Projection failure carrying the stable public code where one is assigned.
#[derive(Debug, Error)]
pub enum Error {
    /// SQLite rejected the operation.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The canonical profile rejected the record.
    #[error("{0}")]
    Profile(#[from] profile::Error),
    /// Non-canonical or out-of-cap projection input.
    #[error("invalid_encoding: {0}")]
    Encoding(String),
    /// A metadata operation is invalid against the parent-cut history.
    #[error("invalid_operation: {0}")]
    InvalidOperation(String),
}

impl From<canon::Error> for Error {
    fn from(error: canon::Error) -> Self {
        Self::Profile(profile::Error::from(error))
    }
}

/// Register fields stored in `field_writes.field`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// Path assignment.
    Path,
    /// Liveness assignment.
    Live,
    /// Selected content epoch.
    Epoch,
    /// Opaque content value.
    Content,
    /// Mode assignment.
    Mode,
}

impl Field {
    /// Stored field tag.
    #[must_use]
    pub const fn tag(self) -> u64 {
        match self {
            Self::Path => 0,
            Self::Live => 1,
            Self::Epoch => 2,
            Self::Content => 3,
            Self::Mode => 4,
        }
    }

    /// Map a stored field tag back to its field; `None` for unknown tags.
    #[must_use]
    pub const fn from_tag(tag: u64) -> Option<Self> {
        match tag {
            0 => Some(Self::Path),
            1 => Some(Self::Live),
            2 => Some(Self::Epoch),
            3 => Some(Self::Content),
            4 => Some(Self::Mode),
            _ => None,
        }
    }
}

/// Typed register value as stored canonically in `field_writes.value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// Path text.
    Path(Path),
    /// Liveness tag.
    Live(LiveTag),
    /// Full epoch descriptor.
    Epoch(EpochDescriptor),
    /// Opaque content artifact.
    Content(ArtifactId),
    /// Mode tag.
    Mode(ModeTag),
}

impl FieldValue {
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        match self {
            Self::Path(path) => canon::write_text(&mut out, path.as_str())?,
            Self::Live(tag) => canon::write_uint(&mut out, tag.tag())?,
            Self::Epoch(descriptor) => out.extend_from_slice(&descriptor.encode()?),
            Self::Content(artifact) => canon::write_bytes(&mut out, artifact.as_bytes())?,
            Self::Mode(tag) => canon::write_uint(&mut out, tag.tag())?,
        }
        Ok(out)
    }

    /// Decode the canonical value bytes of one field.
    ///
    /// # Errors
    /// Returns [`Error::Profile`] for non-canonical or wrong-field bytes.
    pub fn decode(field: Field, bytes: &[u8]) -> Result<Self, Error> {
        let item = canon::decode(bytes)?;
        Ok(match field {
            Field::Path => match &item {
                Item::Text(text) => Self::Path(Path::new(text)?),
                _ => return Err(Error::Encoding("path value is not text".into())),
            },
            Field::Live => match &item {
                Item::Uint(tag) => Self::Live(
                    LiveTag::from_tag(*tag)
                        .ok_or_else(|| Error::Encoding("unknown live tag".into()))?,
                ),
                _ => return Err(Error::Encoding("live value is not a uint".into())),
            },
            Field::Epoch => Self::Epoch(EpochDescriptor::decode(&item)?),
            Field::Content => match &item {
                Item::Bytes(raw) if raw.len() == ArtifactId::WIDTH => Self::Content(
                    ArtifactId::from_bytes(raw).map_err(|err| Error::Encoding(err.to_string()))?,
                ),
                _ => return Err(Error::Encoding("content value is not bytes32".into())),
            },
            Field::Mode => match &item {
                Item::Uint(tag) => Self::Mode(
                    ModeTag::from_tag(*tag)
                        .ok_or_else(|| Error::Encoding("unknown mode tag".into()))?,
                ),
                _ => return Err(Error::Encoding("mode value is not a uint".into())),
            },
        })
    }
}

/// One maximal register write at a cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterWrite {
    /// File the write concerns.
    pub file: FileId,
    /// Register field.
    pub field: Field,
    /// Write identity `(batch_id, operation_index)`.
    pub write: WriteIdent,
    /// Assigned value.
    pub value: FieldValue,
}

/// Store one published batch's field writes in `field_writes`.
///
/// Must run inside the publication transaction, after the batch row exists.
/// A create operation also writes an initial `Alive` liveness assignment at the
/// same write identity: liveness is a register (§5.6) and a created file is
/// alive, while later content edits never implicitly resurrect a deleted file.
pub(crate) fn record_batch(
    conn: &Connection,
    batch: &BatchId,
    metadata: &MetadataArtifact,
) -> Result<(), Error> {
    for (index, operation) in metadata.operations.iter().enumerate() {
        let op_index = index as u64;
        if op_index > profile::MAX_OPERATION_INDEX || op_index > i64::MAX as u64 {
            return Err(Error::Encoding(
                "operation index exceeds the storable range".into(),
            ));
        }
        let op_tag = operation.tag();
        let rows: &[(Field, FieldValue)] = match operation {
            Operation::Create {
                path, mode, epoch, ..
            } => &[
                (Field::Path, FieldValue::Path(path.clone())),
                (Field::Mode, FieldValue::Mode(*mode)),
                (Field::Epoch, FieldValue::Epoch(epoch.clone())),
                (Field::Live, FieldValue::Live(LiveTag::Alive)),
            ],
            Operation::Path { path, .. } => &[(Field::Path, FieldValue::Path(path.clone()))],
            Operation::Live { live, .. } => &[(Field::Live, FieldValue::Live(*live))],
            Operation::Epoch { epoch, .. } => &[(Field::Epoch, FieldValue::Epoch(epoch.clone()))],
            Operation::Opaque { artifact, .. } => {
                &[(Field::Content, FieldValue::Content(*artifact))]
            }
            Operation::Mode { mode, .. } => &[(Field::Mode, FieldValue::Mode(*mode))],
            // Context events, Git associations, resolutions and provenance
            // assertions write no file register field.
            Operation::Context { .. }
            | Operation::Git { .. }
            | Operation::Resolution { .. }
            | Operation::Provenance { .. } => &[],
        };
        let Some(file) = operation_file(operation) else {
            continue;
        };
        for (field, value) in rows {
            let encoded = value.encode()?;
            conn.execute(
                "INSERT INTO field_writes(file, field, op_tag, batch_id, op_index, value)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    file.as_bytes().as_slice(),
                    field.tag() as i64,
                    op_tag as i64,
                    batch.as_bytes().as_slice(),
                    op_index as i64,
                    encoded,
                ],
            )?;
        }
    }
    Ok(())
}

fn operation_file(operation: &Operation) -> Option<FileId> {
    match operation {
        Operation::Create { file, .. }
        | Operation::Path { file, .. }
        | Operation::Live { file, .. }
        | Operation::Epoch { file, .. }
        | Operation::Opaque { file, .. }
        | Operation::Mode { file, .. } => Some(*file),
        Operation::Context { .. }
        | Operation::Git { .. }
        | Operation::Resolution { .. }
        | Operation::Provenance { .. } => None,
    }
}

/// Admission validation of a metadata artifact against the parent-cut history.
///
/// Runs inside the publication transaction after the parents are admitted.
/// Enforces §7.1: a create mints a fresh `FileId` (duplicate creates are
/// `invalid_operation`), every other file operation names a file whose create
/// is in the parent closure or earlier in the same batch, create/epoch
/// descriptors name their own file, and an opaque value assignment names an
/// epoch declared `opaque` for that file.
pub(crate) fn validate_against_history(
    conn: &Connection,
    parents: &[BatchId],
    metadata: &MetadataArtifact,
) -> Result<(), Error> {
    if parents.len() > profile::MAX_BATCH_PARENTS {
        return Err(Error::Encoding("parents exceed the batch cap".into()));
    }
    let history = load_writes(conn, parents)?;
    let mut created: HashSet<FileId> = HashSet::new();
    let mut epochs: HashMap<(FileId, crate::identity::EpochId), EpochKind> = HashMap::new();
    for write in &history {
        if write.op_tag == 0 {
            created.insert(write.file);
        }
        if write.field == Field::Epoch.tag() {
            let descriptor = decode_epoch_value(&write.value)?;
            epochs.insert((write.file, descriptor.id()?), descriptor.kind);
        }
    }
    for (index, operation) in metadata.operations.iter().enumerate() {
        if index as u64 > profile::MAX_OPERATION_INDEX {
            return Err(Error::InvalidOperation(
                "operation index exceeds the profile cap".into(),
            ));
        }
        match operation {
            Operation::Create { file, epoch, .. } => {
                if epoch.file != *file {
                    return Err(Error::InvalidOperation(
                        "create epoch descriptor names another file".into(),
                    ));
                }
                if !created.insert(*file) {
                    return Err(Error::InvalidOperation(format!(
                        "file id {file} is already created"
                    )));
                }
                epochs.insert((*file, epoch.id()?), epoch.kind);
            }
            Operation::Path { file, .. }
            | Operation::Live { file, .. }
            | Operation::Mode { file, .. } => {
                if !created.contains(file) {
                    return Err(Error::InvalidOperation(format!(
                        "operation references file {file} not created in the parent cut"
                    )));
                }
            }
            Operation::Epoch { file, epoch } => {
                if !created.contains(file) {
                    return Err(Error::InvalidOperation(format!(
                        "epoch selection references file {file} not created in the parent cut"
                    )));
                }
                if epoch.file != *file {
                    return Err(Error::InvalidOperation(
                        "epoch descriptor names another file".into(),
                    ));
                }
                epochs.insert((*file, epoch.id()?), epoch.kind);
            }
            Operation::Opaque { file, epoch, .. } => {
                if !created.contains(file) {
                    return Err(Error::InvalidOperation(format!(
                        "opaque assignment references file {file} not created in the parent cut"
                    )));
                }
                if !matches!(epochs.get(&(*file, *epoch)), Some(EpochKind::Opaque)) {
                    return Err(Error::InvalidOperation(
                        "opaque value assigned to an epoch not declared opaque for this file"
                            .into(),
                    ));
                }
            }
            Operation::Context { .. }
            | Operation::Git { .. }
            | Operation::Resolution { .. }
            | Operation::Provenance { .. } => {}
        }
    }
    Ok(())
}

/// Maximal MV-register writes visible at a cut, one entry per surviving
/// alternative.
///
/// `cut` is a frontier (heads included). The result is grouped by
/// `(file, field)` and, within each group, ordered by stable bytewise write
/// identity `(batch32, operation_index)`; the first entry of a group is the
/// smallest-identity display representative (§5.6). Arrival order never
/// affects the result.
///
/// # Errors
/// Returns [`Error::Encoding`] when the cut exceeds the frontier cap, and
/// SQLite/profile errors when history cannot be read.
pub fn register_at(conn: &Connection, cut: &[BatchId]) -> Result<Vec<RegisterWrite>, Error> {
    if cut.is_empty() {
        return Ok(Vec::new());
    }
    if cut.len() > profile::MAX_FRONTIER_HEADS {
        return Err(Error::Encoding("cut exceeds the frontier cap".into()));
    }
    // A cut must name visible publications: unknown or quarantined members are
    // an explicit failure, never an approximate view (§5.2).
    for chunk in cut.chunks(SEED_CHUNK) {
        let values = values_list(chunk.len());
        let sql = format!(
            "SELECT COUNT(*) FROM batches b
             WHERE b.quarantined = 0 AND b.batch_id IN (SELECT column1 FROM (VALUES {values}))"
        );
        let params: Vec<&[u8]> = chunk
            .iter()
            .map(|seed| seed.as_bytes().as_slice())
            .collect();
        let visible: i64 =
            conn.query_row(&sql, rusqlite::params_from_iter(params), |row| row.get(0))?;
        if visible != chunk.len() as i64 {
            return Err(Error::Encoding(
                "cut names a batch that is not a visible publication".into(),
            ));
        }
    }
    let ancestors = ancestor_map(conn, cut)?;
    let writes = load_writes(conn, cut)?;
    let mut groups: HashMap<(FileId, u64), Vec<(WriteIdent, FieldValue)>> = HashMap::new();
    for write in writes {
        let field = Field::from_tag(write.field)
            .ok_or_else(|| Error::Encoding("unknown field tag in field_writes".into()))?;
        let value = FieldValue::decode(field, &write.value)?;
        groups.entry((write.file, write.field)).or_default().push((
            WriteIdent {
                batch: write.batch,
                operation_index: write.op_index,
            },
            value,
        ));
    }
    let mut out = Vec::new();
    for ((file, field_tag), mut entries) in groups {
        let Some(field) = Field::from_tag(field_tag) else {
            continue;
        };
        entries.sort_by_key(|(write, _)| *write);
        for (write, value) in &entries {
            let dominated = entries
                .iter()
                .any(|(other, _)| supersedes(other, write, &ancestors));
            if !dominated {
                out.push(RegisterWrite {
                    file,
                    field,
                    write: *write,
                    value: value.clone(),
                });
            }
        }
    }
    out.sort_by(|a, b| (&a.file, a.field, &a.write).cmp(&(&b.file, b.field, &b.write)));
    Ok(out)
}

/// Whether `winner` causally supersedes `loser`: the loser's batch is in the
/// winner's ancestor closure, or both are the same batch and the loser's
/// operation index is earlier (§5.6, §7 write identity).
fn supersedes(
    winner: &WriteIdent,
    loser: &WriteIdent,
    ancestors: &HashMap<BatchId, HashSet<BatchId>>,
) -> bool {
    if winner.batch == loser.batch {
        loser.operation_index < winner.operation_index
    } else {
        ancestors
            .get(&winner.batch)
            .is_some_and(|set| set.contains(&loser.batch))
    }
}

struct RawWrite {
    file: FileId,
    field: u64,
    op_tag: u64,
    batch: BatchId,
    op_index: u64,
    value: Vec<u8>,
}

fn decode_epoch_value(bytes: &[u8]) -> Result<EpochDescriptor, Error> {
    let item = canon::decode(bytes)?;
    Ok(EpochDescriptor::decode(&item)?)
}

/// Bind-variable budget per statement. The bundled SQLite accepts at most
/// 32,766 variables per statement, so seed lists are chunked well below that;
/// the ancestor closure itself is always walked inside the SQL CTE, never
/// turned into bind parameters.
const SEED_CHUNK: usize = 16_384;

fn values_list(count: usize) -> String {
    (1..=count)
        .map(|i| format!("(?{i})"))
        .collect::<Vec<_>>()
        .join(",")
}

fn seed_params(chunk: &[BatchId]) -> Vec<&[u8]> {
    chunk
        .iter()
        .map(|seed| seed.as_bytes().as_slice())
        .collect()
}

/// Map every batch in the closure of `seeds` to its own ancestor closure
/// (itself included), so supersession can be checked for any writing batch,
/// not only the seeds. Quarantined batches are not visible history and are
/// excluded at the seed and at every recursive step.
fn ancestor_map(
    conn: &Connection,
    seeds: &[BatchId],
) -> Result<HashMap<BatchId, HashSet<BatchId>>, Error> {
    if seeds.is_empty() {
        return Ok(HashMap::new());
    }
    if seeds.len() > profile::MAX_FRONTIER_HEADS {
        return Err(Error::Encoding("seed set exceeds the frontier cap".into()));
    }
    let mut map: HashMap<BatchId, HashSet<BatchId>> = HashMap::new();
    for chunk in seeds.chunks(SEED_CHUNK) {
        let values = values_list(chunk.len());
        let sql = format!(
            "WITH RECURSIVE h(x) AS (
               SELECT column1 FROM (VALUES {values}) v
               JOIN batches b ON b.batch_id = v.column1 AND b.quarantined = 0
             ),
             mem(x) AS (
               SELECT x FROM h
               UNION
               SELECT bp.parent FROM mem
               JOIN batch_parents bp ON bp.child = mem.x
               JOIN batches b ON b.batch_id = bp.parent AND b.quarantined = 0
             ),
             cl(root, node) AS (
               SELECT x, x FROM mem
               UNION
               SELECT cl.root, bp.parent FROM cl
               JOIN batch_parents bp ON bp.child = cl.node
               JOIN batches b ON b.batch_id = bp.parent AND b.quarantined = 0
             )
             SELECT root, node FROM cl"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(seed_params(chunk)), |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (root, node) = row?;
            let root =
                BatchId::from_bytes(&root).map_err(|err| Error::Encoding(err.to_string()))?;
            let node =
                BatchId::from_bytes(&node).map_err(|err| Error::Encoding(err.to_string()))?;
            map.entry(root).or_default().insert(node);
        }
    }
    Ok(map)
}

/// Load every visible field write whose batch lies in the closure of `seeds`.
/// The closure is computed by the SQL CTE, so only the seeds are bound.
fn load_writes(conn: &Connection, seeds: &[BatchId]) -> Result<Vec<RawWrite>, Error> {
    if seeds.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut seen: HashSet<(FileId, u64, BatchId, u64)> = HashSet::new();
    for chunk in seeds.chunks(SEED_CHUNK) {
        let values = values_list(chunk.len());
        let sql = format!(
            "WITH RECURSIVE cl(node) AS (
               SELECT column1 FROM (VALUES {values}) v
               JOIN batches b ON b.batch_id = v.column1 AND b.quarantined = 0
               UNION
               SELECT bp.parent FROM cl
               JOIN batch_parents bp ON bp.child = cl.node
               JOIN batches b ON b.batch_id = bp.parent AND b.quarantined = 0
             )
             SELECT fw.file, fw.field, fw.op_tag, fw.batch_id, fw.op_index, fw.value
             FROM field_writes fw
             JOIN cl ON cl.node = fw.batch_id
             JOIN batches b ON b.batch_id = fw.batch_id AND b.quarantined = 0"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(seed_params(chunk)), |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Vec<u8>>(5)?,
            ))
        })?;
        for row in rows {
            let (file, field, op_tag, batch, op_index, value) = row?;
            if field < 0 || op_tag < 0 || op_index < 0 {
                return Err(Error::Encoding("negative integer in field_writes".into()));
            }
            let file = FileId::from_bytes(&file).map_err(|err| Error::Encoding(err.to_string()))?;
            let batch =
                BatchId::from_bytes(&batch).map_err(|err| Error::Encoding(err.to_string()))?;
            // Overlapping closures of different seed chunks can yield the same
            // write twice; each write identity is stored once.
            if !seen.insert((file, field as u64, batch, op_index as u64)) {
                continue;
            }
            out.push(RawWrite {
                file,
                field: field as u64,
                op_tag: op_tag as u64,
                batch,
                op_index: op_index as u64,
                value,
            });
        }
    }
    Ok(out)
}
