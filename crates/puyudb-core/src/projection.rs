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
//!
//! Projections see visible history only: quarantined batches (writer
//! equivocation) are not history and their writes are excluded everywhere;
//! a cut naming one is an explicit error at the boundary. A quarantine inside
//! an otherwise visible closure cuts causality at that node: the
//! quarantined batch's exclusive ancestors become invisible below any
//! descendant cut (writes reachable through another visible head still
//! surface as concurrent alternatives). This semantic is deliberate
//! (quarantine = not visible history) and pinned by tests.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Connection;
use thiserror::Error;

use crate::canon::{self, Item};
use crate::identity::{ArtifactId, BatchId, BranchId, ConflictId, EpochId, FileId, SpaceId};
use crate::profile::{
    self, Conflict, ConflictKind, EpochDescriptor, EpochKind, FileEntry, LiveTag, MetadataArtifact,
    ModeTag, Operation, Path, Resolution, SemanticManifest, WriteIdent,
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
    /// Opaque content assignment: the declared epoch and the content artifact.
    Content {
        /// Declared opaque epoch the value belongs to.
        epoch: EpochId,
        /// Content artifact.
        artifact: ArtifactId,
    },
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
            Self::Content { epoch, artifact } => {
                canon::write_array_header(&mut out, 2)?;
                canon::write_bytes(&mut out, epoch.as_bytes())?;
                canon::write_bytes(&mut out, artifact.as_bytes())?;
            }
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
            Field::Content => {
                let pair = match &item {
                    Item::Array(items) if items.len() == 2 => items,
                    _ => return Err(Error::Encoding("content value is not a pair".into())),
                };
                let fixed32 = |item: &Item| -> Result<[u8; 32], Error> {
                    match item {
                        Item::Bytes(raw) if raw.len() == 32 => {
                            let mut out = [0u8; 32];
                            out.copy_from_slice(raw);
                            Ok(out)
                        }
                        _ => Err(Error::Encoding("content pair member is not bytes32".into())),
                    }
                };
                Self::Content {
                    epoch: EpochId::from_array(fixed32(&pair[0])?),
                    artifact: ArtifactId::from_array(fixed32(&pair[1])?),
                }
            }
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
    /// Metadata operation tag that produced the write (§7.1).
    pub op_tag: u64,
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
            Operation::Opaque {
                epoch, artifact, ..
            } => &[(
                Field::Content,
                FieldValue::Content {
                    epoch: *epoch,
                    artifact: *artifact,
                },
            )],
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
    let mut epochs: HashMap<(FileId, EpochId), EpochKind> = HashMap::new();
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
    // Tag-8 resolutions are validated against the conflicts visible at the
    // parent cut (P4); skipped entirely when the batch carries none, so the
    // common publish path pays no conflict-derivation cost.
    if metadata
        .operations
        .iter()
        .any(|op| matches!(op, Operation::Resolution { .. }))
    {
        validate_resolutions(conn, parents, metadata)?;
    }
    Ok(())
}

/// Validate tag-8 resolution operations against the parent cut (§9, §22.2):
/// the named conflict must be visible at the parent cut, the parents must
/// observe every named participant, and the same batch must publish the fresh
/// corresponding field assignment (acceptance-only records are insufficient).
fn validate_resolutions(
    conn: &Connection,
    parents: &[BatchId],
    metadata: &MetadataArtifact,
) -> Result<(), Error> {
    let conflicts = conflicts_impl(conn, parents)?;
    let register = register_impl(conn, parents)?;
    let ancestors = ancestor_map(conn, parents)?;
    let mut closure: HashSet<BatchId> = parents.iter().copied().collect();
    for set in ancestors.values() {
        closure.extend(set.iter().copied());
    }
    for operation in &metadata.operations {
        let Operation::Resolution {
            conflict,
            resolution,
        } = operation
        else {
            continue;
        };
        let found = conflicts
            .iter()
            .find(|c| matches!(c.id(), Ok(id) if id == *conflict))
            .ok_or_else(|| {
                Error::InvalidOperation(
                    "resolution names a conflict not visible at the parent cut".into(),
                )
            })?;
        for participant in &found.participants {
            if !closure.contains(&participant.batch) {
                return Err(Error::InvalidOperation(
                    "resolution parents do not observe a named participant".into(),
                ));
            }
        }
        let file = found.file;
        // §22.2: resolutions are tag-specific per conflict kind, never a
        // generic value choice for every kind.
        let kind_matches = matches!(
            (found.kind, resolution),
            (
                ConflictKind::DeleteEdit,
                Resolution::RetainDelete | Resolution::RestoreEdit
            ) | (
                ConflictKind::EpochEdit,
                Resolution::RetainEpoch(_) | Resolution::Incorporate { .. }
            ) | (ConflictKind::Register, Resolution::Choose { .. })
        );
        if !kind_matches {
            return Err(Error::InvalidOperation(
                "resolution tag does not match the named conflict's kind".into(),
            ));
        }
        match resolution {
            Resolution::RetainDelete => {
                if !has_live_op(metadata, file, LiveTag::Deleted) {
                    return Err(Error::InvalidOperation(
                        "retain-delete resolution lacks a fresh Deleted assignment in the same batch".into(),
                    ));
                }
            }
            Resolution::RestoreEdit => {
                if !has_live_op(metadata, file, LiveTag::Alive) {
                    return Err(Error::InvalidOperation(
                        "restore-edit resolution lacks a fresh Alive assignment in the same batch"
                            .into(),
                    ));
                }
            }
            Resolution::RetainEpoch(epoch) => {
                // §9 tag 2 retains the SELECTED epoch: the payload must be the
                // epoch of the conflict's epoch-selection participant, not any
                // declared epoch.
                let selected = found.participants.iter().find(|participant| {
                    register.iter().any(|w| {
                        &w.write == *participant
                            && w.field == Field::Epoch
                            && matches!(&w.value, FieldValue::Epoch(d)
                                if d.id().map(|id| id == *epoch).unwrap_or(false))
                    })
                });
                if selected.is_none() {
                    return Err(Error::InvalidOperation(
                        "retain-epoch payload is not the conflict's selected epoch".into(),
                    ));
                }
                if !has_epoch_op(metadata, file, epoch)? {
                    return Err(Error::InvalidOperation(
                        "retain-epoch resolution lacks a fresh selection of that epoch in the same batch".into(),
                    ));
                }
            }
            Resolution::Incorporate { epoch, provenance } => {
                // Incorporation requires an explicit NEW epoch, not a
                // relabeling of one already declared for the file anywhere in
                // the parent closure — including causally superseded (no
                // longer maximal) declarations (§22.2).
                let declared_anywhere = load_writes(conn, parents)?.iter().any(|w| {
                    w.file == file
                        && w.field == Field::Epoch.tag()
                        && decode_epoch_value(&w.value)
                            .map(|d| d.id().map(|id| id == *epoch).unwrap_or(false))
                            .unwrap_or(false)
                });
                if declared_anywhere {
                    return Err(Error::InvalidOperation(
                        "incorporation must name a new epoch not already declared for the file"
                            .into(),
                    ));
                }
                if !has_epoch_op(metadata, file, epoch)? {
                    return Err(Error::InvalidOperation(
                        "incorporation lacks the fresh explicit epoch write in the same batch"
                            .into(),
                    ));
                }
                if !metadata.operations.iter().any(|op| {
                    matches!(
                        op,
                        Operation::Provenance { detail: Some(d), .. } if d == provenance
                    )
                }) {
                    return Err(Error::InvalidOperation(
                        "incorporation lacks the provenance operation in the same batch".into(),
                    ));
                }
            }
            Resolution::Choose { field, write } => {
                if !found.participants.contains(write) {
                    return Err(Error::InvalidOperation(
                        "chosen alternative is not a participant of the named conflict".into(),
                    ));
                }
                let Some(chosen) = register.iter().find(|w| &w.write == write) else {
                    return Err(Error::InvalidOperation(
                        "chosen write identity is not visible at the parent cut".into(),
                    ));
                };
                // Resolution field tags: 0 path, 1 content value, 2 mode; the
                // chosen write's field must be the one named.
                let expected_field = match chosen.field {
                    Field::Path => 0,
                    Field::Content => 1,
                    Field::Mode => 2,
                    Field::Live | Field::Epoch => {
                        return Err(Error::InvalidOperation(
                            "chosen alternative is not a path/content/mode field".into(),
                        ));
                    }
                };
                if *field != expected_field {
                    return Err(Error::InvalidOperation(
                        "resolution field tag does not match the chosen write's field".into(),
                    ));
                }
                let matches_fresh_op = |op: &Operation| match (field, op) {
                    (0, Operation::Path { file: f, path }) => {
                        *f == file && matches!(&chosen.value, FieldValue::Path(p) if p == path)
                    }
                    (
                        1,
                        Operation::Opaque {
                            file: f,
                            epoch,
                            artifact,
                        },
                    ) => {
                        *f == file
                            && matches!(&chosen.value, FieldValue::Content { epoch: e, artifact: a } if e == epoch && a == artifact)
                    }
                    (2, Operation::Mode { file: f, mode }) => {
                        *f == file && matches!(&chosen.value, FieldValue::Mode(m) if m == mode)
                    }
                    _ => false,
                };
                if !metadata.operations.iter().any(matches_fresh_op) {
                    return Err(Error::InvalidOperation(
                        "choose-alternative resolution lacks the fresh write of the chosen value in the same batch".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn has_live_op(metadata: &MetadataArtifact, file: FileId, live: LiveTag) -> bool {
    metadata
        .operations
        .iter()
        .any(|op| matches!(op, Operation::Live { file: f, live: l } if *f == file && *l == live))
}

fn has_epoch_op(metadata: &MetadataArtifact, file: FileId, epoch: &EpochId) -> Result<bool, Error> {
    Ok(metadata.operations.iter().any(|op| {
        matches!(op, Operation::Epoch { file: f, epoch: d } if *f == file && matches!(d.id(), Ok(id) if id == *epoch))
    }))
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
    check_cut(conn, cut)?;
    register_impl(conn, cut)
}

/// Public cut gate: non-empty, within the frontier cap, duplicate-free, and
/// naming only visible publications. Unknown or quarantined members are an
/// explicit failure, never an approximate view (§5.2).
fn check_cut(conn: &Connection, cut: &[BatchId]) -> Result<(), Error> {
    if cut.is_empty() {
        return Ok(());
    }
    if cut.len() > profile::MAX_FRONTIER_HEADS {
        return Err(Error::Encoding("cut exceeds the frontier cap".into()));
    }
    // A frontier is sorted unique (§5.2); duplicates are a caller bug and must
    // not degrade into a misleading visibility error below.
    let unique: HashSet<&BatchId> = cut.iter().collect();
    if unique.len() != cut.len() {
        return Err(Error::Encoding("cut contains duplicate heads".into()));
    }
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
    Ok(())
}

/// Register evaluation without the public cut-visibility gate, for internal
/// derivations (conflict derivation at a batch's parent cut) where the seeds
/// are already known to be admitted.
pub(crate) fn register_impl(
    conn: &Connection,
    cut: &[BatchId],
) -> Result<Vec<RegisterWrite>, Error> {
    let ancestors = ancestor_map(conn, cut)?;
    let writes = load_writes(conn, cut)?;
    /// One candidate write in a group: identity, decoded value, operation tag.
    type Candidate = (WriteIdent, FieldValue, u64);
    let mut groups: HashMap<(FileId, u64), Vec<Candidate>> = HashMap::new();
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
            write.op_tag,
        ));
    }
    let mut out = Vec::new();
    for ((file, field_tag), mut entries) in groups {
        let Some(field) = Field::from_tag(field_tag) else {
            continue;
        };
        entries.sort_by_key(|(write, _, _)| *write);
        for (write, value, op_tag) in &entries {
            let dominated = entries
                .iter()
                .any(|(other, _, _)| supersedes(other, write, &ancestors));
            if !dominated {
                out.push(RegisterWrite {
                    file,
                    field,
                    write: *write,
                    value: value.clone(),
                    op_tag: *op_tag,
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

/// Map each writing batch in the closure of `seeds` to its own ancestor
/// closure (itself included), so supersession can be checked for any writing
/// batch, not only the seeds. The map is rooted only at batches that have
/// field writes, bounding it at (#write-batches x closure) — write-free
/// batches are still walked as ancestors, never as roots. Quarantined
/// batches are not visible history and are excluded at the seed and at every
/// recursive step.
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
               SELECT mem.x, mem.x FROM mem
               JOIN (SELECT DISTINCT batch_id FROM field_writes) w ON w.batch_id = mem.x
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

// --------------------------------------------------------------------------
// I05 P3/P4/P5: file projections, conflicts, semantic manifest
// --------------------------------------------------------------------------

/// One file's derived projection at a cut: the maximal alternatives of every
/// register field. Deleted files keep their entry and their content (§7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProjection {
    /// File identity.
    pub file: FileId,
    /// Maximal liveness alternatives (sorted by tag).
    pub live: Vec<LiveTag>,
    /// Maximal path alternatives (sorted by raw UTF-8 bytes).
    pub paths: Vec<Path>,
    /// Maximal epoch alternatives (sorted by identity).
    pub epochs: Vec<EpochId>,
    /// Maximal opaque content alternatives (sorted by artifact identity).
    pub contents: Vec<ArtifactId>,
    /// Maximal mode alternatives (sorted by tag).
    pub modes: Vec<ModeTag>,
}

/// Per-file register projections at a cut, sorted by raw `FileId` bytes.
///
/// # Errors
/// Same cut gate as [`register_at`], plus SQLite/profile errors.
pub fn files_at(conn: &Connection, cut: &[BatchId]) -> Result<Vec<FileProjection>, Error> {
    check_cut(conn, cut)?;
    files_impl(conn, cut)
}

pub(crate) fn files_impl(conn: &Connection, cut: &[BatchId]) -> Result<Vec<FileProjection>, Error> {
    let register = register_impl(conn, cut)?;
    let mut by_file: HashMap<FileId, FileProjection> = HashMap::new();
    for write in register {
        let entry = by_file.entry(write.file).or_insert_with(|| FileProjection {
            file: write.file,
            live: Vec::new(),
            paths: Vec::new(),
            epochs: Vec::new(),
            contents: Vec::new(),
            modes: Vec::new(),
        });
        match write.value {
            FieldValue::Live(tag) => entry.live.push(tag),
            FieldValue::Path(path) => entry.paths.push(path),
            FieldValue::Epoch(descriptor) => entry.epochs.push(descriptor.id()?),
            FieldValue::Content { artifact, .. } => entry.contents.push(artifact),
            FieldValue::Mode(tag) => entry.modes.push(tag),
        }
    }
    let mut out: Vec<FileProjection> = by_file.into_values().collect();
    for projection in &mut out {
        projection.live.sort_by_key(|tag| tag.tag());
        projection.live.dedup();
        projection.paths.sort();
        projection.paths.dedup();
        projection.epochs.sort();
        projection.epochs.dedup();
        projection.contents.sort();
        projection.contents.dedup();
        projection.modes.sort_by_key(|tag| tag.tag());
        projection.modes.dedup();
    }
    out.sort_by_key(|projection| projection.file);
    Ok(out)
}

/// The `paths_current` projection at a cut: each path of a currently alive file
/// mapped to its sorted collision set of files. Two files assigned the same
/// path form a collision set that is never auto-resolved (§5.6); a file whose
/// maximal liveness is only `Deleted` has no current path. Deleted files keep
/// their semantic-manifest entry but drop out of the current path view.
///
/// # Errors
/// Same cut gate as [`register_at`].
pub fn paths_at(conn: &Connection, cut: &[BatchId]) -> Result<Vec<(Path, Vec<FileId>)>, Error> {
    check_cut(conn, cut)?;
    let files = files_impl(conn, cut)?;
    let mut map: BTreeMap<String, Vec<FileId>> = BTreeMap::new();
    for projection in files {
        if !projection.live.contains(&LiveTag::Alive) {
            continue;
        }
        for path in &projection.paths {
            map.entry(path.as_str().to_owned())
                .or_default()
                .push(projection.file);
        }
    }
    map.into_iter()
        .map(|(text, mut files)| {
            files.sort();
            Ok::<_, Error>((Path::new(&text)?, files))
        })
        .collect()
}

/// File/directory prefix conflicts at a cut, e.g. `a` and `a/b` (§5.6): pairs
/// of current paths where one is a strict path-prefix of the other. These are
/// collision sets, reported, never auto-resolved or renamed.
///
/// # Errors
/// Same cut gate as [`register_at`].
pub fn prefix_conflicts_at(conn: &Connection, cut: &[BatchId]) -> Result<Vec<(Path, Path)>, Error> {
    let paths = paths_at(conn, cut)?;
    let keys: Vec<&str> = paths.iter().map(|(path, _)| path.as_str()).collect();
    let mut out = Vec::new();
    for (path, _) in &paths {
        let prefix = format!("{}/", path.as_str());
        let start = keys.partition_point(|key| *key < prefix.as_str());
        let matching = keys[start..]
            .iter()
            .take_while(|key| key.starts_with(&prefix))
            .count();
        for other in &paths[start..start + matching] {
            out.push((path.clone(), other.0.clone()));
        }
    }
    Ok(out)
}

/// Current heads of one branch: visible batches on the branch that are not an
/// ancestor of another visible batch on the same branch. Until a frontier
/// table lands (I08/I12), the `files_current`/`paths_current` branch views are
/// computed from these heads on read; they stay rebuildable accelerations
/// (§6.3), never authoritative state.
///
/// # Errors
/// Returns a SQLite error if the read fails.
pub fn branch_heads(conn: &Connection, branch: &BranchId) -> Result<Vec<BatchId>, Error> {
    let sql = "
        WITH RECURSIVE h(x) AS (
          SELECT batch_id FROM batches WHERE branch = ?1 AND quarantined = 0
        ),
        d(root, x) AS (
          SELECT h.x, bp.child FROM h
          JOIN batch_parents bp ON bp.parent = h.x
          JOIN batches cb ON cb.batch_id = bp.child AND cb.quarantined = 0
          UNION
          SELECT d.root, bp.child FROM d
          JOIN batch_parents bp ON bp.parent = d.x
          JOIN batches cb ON cb.batch_id = bp.child AND cb.quarantined = 0
        )
        SELECT h.x FROM h
        WHERE NOT EXISTS (
          SELECT 1 FROM d JOIN batches b ON b.batch_id = d.x
          WHERE d.root = h.x AND d.x != h.x AND b.branch = ?1 AND b.quarantined = 0
        )
        ORDER BY h.x";
    let mut statement = conn.prepare(sql)?;
    let rows = statement.query_map([branch.as_bytes().as_slice()], |row| {
        row.get::<_, Vec<u8>>(0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        let bytes = row?;
        out.push(BatchId::from_bytes(&bytes).map_err(|err| Error::Encoding(err.to_string()))?);
    }
    Ok(out)
}

/// Conflicts visible at a cut, recomputed from history (§9) — never persisted
/// as arrival-dependent flags. Kind 0 register alternatives, kind 1
/// `delete_edit`, kind 2 `epoch_edit`, each with a stable `ConflictId` that is
/// independent of delivery order because participants and epochs are sorted.
///
/// # Errors
/// Same cut gate as [`register_at`].
pub fn conflicts_at(conn: &Connection, cut: &[BatchId]) -> Result<Vec<Conflict>, Error> {
    check_cut(conn, cut)?;
    conflicts_impl(conn, cut)
}

pub(crate) fn conflicts_impl(conn: &Connection, cut: &[BatchId]) -> Result<Vec<Conflict>, Error> {
    let register = register_impl(conn, cut)?;
    let ancestors = ancestor_map(conn, cut)?;
    let mut by_file: HashMap<FileId, Vec<&RegisterWrite>> = HashMap::new();
    for write in &register {
        by_file.entry(write.file).or_default().push(write);
    }
    let concurrent = |a: &BatchId, b: &BatchId| -> bool {
        let a_before_b = ancestors.get(a).is_some_and(|set| set.contains(b));
        let b_before_a = ancestors.get(b).is_some_and(|set| set.contains(a));
        !a_before_b && !b_before_a
    };
    let all_writes = load_writes(conn, cut)?;
    let mut out: Vec<Conflict> = Vec::new();
    for (file, writes) in &by_file {
        let group = |field: Field| -> Vec<&RegisterWrite> {
            writes
                .iter()
                .copied()
                .filter(|w| w.field == field)
                .collect()
        };
        // Kind 0: concurrent maximal register alternatives.
        for field in [
            Field::Path,
            Field::Live,
            Field::Epoch,
            Field::Content,
            Field::Mode,
        ] {
            let entries = group(field);
            if entries.len() >= 2 {
                out.push(Conflict {
                    kind: ConflictKind::Register,
                    file: *file,
                    epochs: Vec::new(),
                    participants: entries.iter().map(|w| w.write).collect(),
                });
            }
        }
        // Kind 1: delete_edit — each maximal deletion assignment against each
        // causally concurrent edit-producing write (epoch or opaque content)
        // whose base cut saw the file alive (§22.2).
        let deleted: Vec<&RegisterWrite> = group(Field::Live)
            .into_iter()
            .filter(|w| w.value == FieldValue::Live(LiveTag::Deleted))
            .collect();
        let edits: Vec<&RegisterWrite> = writes
            .iter()
            .copied()
            .filter(|w| w.op_tag == 3 || w.op_tag == 4)
            .collect();
        for deletion in &deleted {
            for edit in &edits {
                if !concurrent(&deletion.write.batch, &edit.write.batch) {
                    continue;
                }
                if !edit_base_saw_alive(conn, &edit.write.batch, *file)? {
                    continue;
                }
                // §9: epochs holds the distinct epoch IDs the conflict
                // concerns — for delete_edit that is the edited epoch, not
                // every epoch selection of the file, so an unrelated
                // concurrent epoch selection cannot change this ConflictId.
                let target_epoch = match &edit.value {
                    FieldValue::Content { epoch, .. } => *epoch,
                    FieldValue::Epoch(descriptor) => descriptor.id()?,
                    _ => continue,
                };
                let mut participants = vec![deletion.write, edit.write];
                participants.sort();
                out.push(Conflict {
                    kind: ConflictKind::DeleteEdit,
                    file: *file,
                    epochs: vec![target_epoch],
                    participants,
                });
            }
        }
        // Kind 2: epoch_edit — each maximal selected epoch against a concurrent
        // opaque assignment to another previously declared epoch (§22.2).
        for selection in group(Field::Epoch) {
            let FieldValue::Epoch(descriptor) = &selection.value else {
                continue;
            };
            let selected = descriptor.id()?;
            for edit in &edits {
                let FieldValue::Content { epoch, .. } = &edit.value else {
                    continue;
                };
                if *epoch == selected || !concurrent(&selection.write.batch, &edit.write.batch) {
                    continue;
                }
                // The edited epoch must have been declared earlier in the closure.
                let declared = all_writes.iter().any(|w| {
                    w.field == Field::Epoch.tag()
                        && decode_epoch_value(&w.value)
                            .map(|d| d.id().map(|id| id == *epoch).unwrap_or(false))
                            .unwrap_or(false)
                });
                if !declared {
                    continue;
                }
                let mut epochs = vec![selected, *epoch];
                epochs.sort();
                epochs.dedup();
                let mut participants = vec![selection.write, edit.write];
                participants.sort();
                out.push(Conflict {
                    kind: ConflictKind::EpochEdit,
                    file: *file,
                    epochs,
                    participants,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        (
            a.kind.tag(),
            a.file,
            a.epochs.clone(),
            a.participants.clone(),
        )
            .cmp(&(
                b.kind.tag(),
                b.file,
                b.epochs.clone(),
                b.participants.clone(),
            ))
    });
    Ok(out)
}

/// Whether the parent cut of `batch` saw `file` alive: the maximal liveness
/// alternatives at that cut contain `Alive` (§22.2 delete/edit rule).
fn edit_base_saw_alive(conn: &Connection, batch: &BatchId, file: FileId) -> Result<bool, Error> {
    let mut statement = conn.prepare("SELECT parent FROM batch_parents WHERE child = ?1")?;
    let rows = statement.query_map([batch.as_bytes().as_slice()], |row| {
        row.get::<_, Vec<u8>>(0)
    })?;
    let mut parents = Vec::new();
    for row in rows {
        let bytes = row?;
        parents.push(BatchId::from_bytes(&bytes).map_err(|err| Error::Encoding(err.to_string()))?);
    }
    let register = register_impl(conn, &parents)?;
    Ok(register.iter().any(|w| {
        w.file == file && w.field == Field::Live && w.value == FieldValue::Live(LiveTag::Alive)
    }))
}

/// The canonical `SemanticManifestV1` at a cut (§7.2): the T02/T13 comparison
/// unit. Entries are sorted by raw `FileId` and every inner array is sorted
/// and deduplicated by `FileEntry::encode`, so the `SemanticManifestId` never
/// depends on arrival order, cache contents, or chunk boundaries.
///
/// # Errors
/// Same cut gate as [`register_at`].
pub fn semantic_manifest_at(
    conn: &Connection,
    space: &SpaceId,
    cut: &[BatchId],
) -> Result<SemanticManifest, Error> {
    check_cut(conn, cut)?;
    let files = files_impl(conn, cut)?;
    let conflicts = conflicts_impl(conn, cut)?;
    let mut entries = Vec::with_capacity(files.len());
    for projection in files {
        let mut ids: Vec<ConflictId> = Vec::new();
        for conflict in &conflicts {
            if conflict.file == projection.file {
                ids.push(conflict.id()?);
            }
        }
        ids.sort();
        ids.dedup();
        entries.push(FileEntry {
            file: projection.file,
            live: projection.live,
            paths: projection.paths,
            epochs: projection.epochs,
            contents: projection.contents,
            modes: projection.modes,
            conflicts: ids,
        });
    }
    Ok(SemanticManifest {
        space: *space,
        entries,
    })
}
