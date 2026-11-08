// SPDX-License-Identifier: Apache-2.0

//! Physical checkpoints and the bounded `cache/` directory (I08 P3;
//! blueprint §6.2/§6.4 step 6/§6.6, D3).
//!
//! A checkpoint is a disposable, hash-checked cached native view of ONE
//! version: `cache/checkpoints/<version-hex>.ckpt` holds the merged native
//! document of every text epoch declared in that version's closure. It is
//! keyed by the VersionId itself, so a read at version V can never observe a
//! checkpoint carrying post-cut changes (§6.6); an ordinary checkpoint never
//! replaces the immutable changes an older retained version needs — canonical
//! history alone survives `rm -rf cache/` and every read re-derives from it.
//!
//! The on-disk format is internal (plan Q2): canonical CBOR, self-digested.
//! It is not a wire format; shipping it on the wire would need a new ADR.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::canon::{self, Item};
use crate::identity::{EpochId, SpaceId, VersionId};
use crate::profile;
use crate::projection::{self, Error as ProjectionError};
use crate::publication::Error;
use crate::version;

/// Cache directory name inside the space directory (§6.2 layout).
pub(crate) const CACHE_DIR: &str = "cache";
/// Checkpoint subdirectory inside `cache/`.
const CHECKPOINT_DIR: &str = "checkpoints";
/// Domain separator for the internal checkpoint self-digest.
const CHECKPOINT_DOMAIN: &[u8] = b"puyudb/checkpoint/v1";
/// Checkpoint file extension.
const CHECKPOINT_EXT: &str = "ckpt";

/// Default replay-cost threshold for auto-checkpoint generation (plan Q1).
/// Replay cost is the closure size of the version (each batch contributes at
/// most `MAX_BATCH_NATIVE_ARTIFACTS` native changes, so closure size bounds
/// the replay work per §5.8). 1024 is recorded as the default: the smallest
/// constant intended to keep W07's 1m-batch p95 inside the §16.3 targets, pinned
/// by test and local-configurable; P4's W07 evidence may revise it via ADR
/// (§16.3 revision policy).
pub const DEFAULT_REPLAY_COST_THRESHOLD: u64 = 1024;
/// Default bounded budget for the whole `cache/` tree in bytes (§6.2
/// "disposable, bounded, hash-checked"). Local-configurable downward.
pub const DEFAULT_CACHE_BUDGET_BYTES: u64 = 64 * 1024 * 1024;
/// Per-checkpoint cap: a view larger than this is not cached; the read falls
/// back to full replay.
pub const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;
/// Snapshot/checkpoint name cap (local policy; §10 freezes no name width).
pub(crate) const MAX_NAME_BYTES: usize = 255;

fn encoding(detail: impl Into<String>) -> Error {
    Error::Projection(ProjectionError::Encoding(detail.into()))
}

fn profile_err(err: profile::Error) -> Error {
    Error::Projection(ProjectionError::Profile(err))
}

/// One cached native view: the merged document bytes of every text epoch
/// declared in the checkpointed version's closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// Space the checkpoint belongs to.
    pub space: SpaceId,
    /// Version (frontier) the view is cached at.
    pub version: VersionId,
    /// `(epoch, merged native document bytes)` entries, sorted by epoch.
    pub docs: Vec<(EpochId, Vec<u8>)>,
}

impl Checkpoint {
    /// Canonical encoding of the checkpoint body (digest field excluded).
    fn body(&self) -> Result<Vec<u8>, Error> {
        let mut docs = self.docs.clone();
        docs.sort_by_key(|(epoch, _)| *epoch);
        let mut out = Vec::new();
        canon::write_array_header(&mut out, 4).map_err(|e| encoding(e.to_string()))?;
        canon::write_uint(&mut out, profile::PROFILE_VERSION)
            .map_err(|e| encoding(e.to_string()))?;
        canon::write_bytes(&mut out, self.space.as_bytes()).map_err(|e| encoding(e.to_string()))?;
        canon::write_bytes(&mut out, self.version.as_bytes())
            .map_err(|e| encoding(e.to_string()))?;
        canon::write_array_header(&mut out, docs.len()).map_err(|e| encoding(e.to_string()))?;
        for (epoch, doc) in &docs {
            canon::write_array_header(&mut out, 2).map_err(|e| encoding(e.to_string()))?;
            canon::write_bytes(&mut out, epoch.as_bytes()).map_err(|e| encoding(e.to_string()))?;
            canon::write_bytes(&mut out, doc).map_err(|e| encoding(e.to_string()))?;
        }
        Ok(out)
    }

    /// Full canonical file bytes including the self-digest.
    fn encode(&self) -> Result<Vec<u8>, Error> {
        let body = self.body()?;
        let digest = checkpoint_digest(&body);
        let mut out = Vec::new();
        canon::write_array_header(&mut out, 2).map_err(|e| encoding(e.to_string()))?;
        canon::write_bytes(&mut out, &digest).map_err(|e| encoding(e.to_string()))?;
        canon::write_bytes(&mut out, &body).map_err(|e| encoding(e.to_string()))?;
        Ok(out)
    }

    /// Decode and hash-check file bytes; `None` when the bytes are not a
    /// valid checkpoint (corrupt caches are discarded, never trusted).
    fn decode(bytes: &[u8]) -> Option<Self> {
        let item = canon::decode(bytes).ok()?;
        let record = match &item {
            Item::Array(items) if items.len() == 2 => items,
            _ => return None,
        };
        let digest = match &record[0] {
            Item::Bytes(d) if d.len() == 32 => d.clone(),
            _ => return None,
        };
        let body = match &record[1] {
            Item::Bytes(b) => b,
            _ => return None,
        };
        if checkpoint_digest(body).as_slice() != digest.as_slice() {
            return None;
        }
        let body_item = canon::decode(body).ok()?;
        let record = match &body_item {
            Item::Array(items) if items.len() == 4 => items,
            _ => return None,
        };
        if !matches!(&record[0], Item::Uint(v) if *v == profile::PROFILE_VERSION) {
            return None;
        }
        let space = match &record[1] {
            Item::Bytes(b) if b.len() == 16 => SpaceId::from_bytes(b).ok()?,
            _ => return None,
        };
        let version = match &record[2] {
            Item::Bytes(b) if b.len() == 32 => VersionId::from_bytes(b).ok()?,
            _ => return None,
        };
        let docs_item = match &record[3] {
            Item::Array(items) => items,
            _ => return None,
        };
        let mut docs = Vec::with_capacity(docs_item.len());
        for entry in docs_item {
            let pair = match entry {
                Item::Array(items) if items.len() == 2 => items,
                _ => return None,
            };
            let epoch = match &pair[0] {
                Item::Bytes(b) if b.len() == 32 => EpochId::from_bytes(b).ok()?,
                _ => return None,
            };
            let doc = match &pair[1] {
                Item::Bytes(b) => b.clone(),
                _ => return None,
            };
            docs.push((epoch, doc));
        }
        Some(Self {
            space,
            version,
            docs,
        })
    }
}

/// Self-digest of a checkpoint body.
fn checkpoint_digest(body: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CHECKPOINT_DOMAIN);
    hasher.update([0u8]);
    hasher.update(body);
    hasher.finalize().into()
}

/// The `cache/` directory of a space.
pub(crate) fn cache_dir(space_dir: &Path) -> PathBuf {
    space_dir.join(CACHE_DIR)
}

/// The checkpoint file path for one version (key derivation is internal per
/// plan Q2: the VersionId hex itself).
pub(crate) fn checkpoint_path(space_dir: &Path, version: &VersionId) -> PathBuf {
    cache_dir(space_dir)
        .join(CHECKPOINT_DIR)
        .join(format!("{}.{CHECKPOINT_EXT}", version))
}

/// Replay cost of a version: the size of its ancestor closure (§5.8 — each
/// batch is one replay unit, bounded by the batch caps).
///
/// # Errors
/// Returns `invalid_encoding` for a non-visible head (via the cut gate) and
/// SQLite errors.
pub fn replay_cost(conn: &Connection, heads: &[crate::identity::BatchId]) -> Result<u64, Error> {
    projection::check_cut_pub(conn, heads)?;
    let set = projection::closure_set_pub(conn, heads)?;
    Ok(set.len() as u64)
}

/// Write one hash-checked checkpoint atomically (temp file + rename), then
/// enforce the cache budget by evicting oldest checkpoint files. Returns the
/// file size written.
///
/// # Errors
/// Returns io errors; a checkpoint larger than [`MAX_CHECKPOINT_BYTES`] is
/// refused with `invalid_encoding` before any allocation-heavy write.
pub(crate) fn write_checkpoint(
    space_dir: &Path,
    checkpoint: &Checkpoint,
    budget_bytes: u64,
) -> Result<u64, Error> {
    let bytes = checkpoint.encode()?;
    if bytes.len() as u64 > MAX_CHECKPOINT_BYTES {
        return Err(encoding("checkpoint exceeds the per-file cap"));
    }
    let dir = cache_dir(space_dir).join(CHECKPOINT_DIR);
    fs::create_dir_all(&dir)?;
    let path = checkpoint_path(space_dir, &checkpoint.version);
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        checkpoint.version,
        std::process::id()
    ));
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &path)?;
    let size = bytes.len() as u64;
    enforce_cache_budget(space_dir, budget_bytes)?;
    Ok(size)
}

/// Read and hash-check the checkpoint of one version. `Ok(None)` when the
/// file is missing OR corrupt — a corrupt cache file is discarded and the
/// read falls back to replay (D3: caches are disposable, never an error).
///
/// # Errors
/// Returns io errors other than a missing file.
pub(crate) fn read_checkpoint(
    space_dir: &Path,
    space: &SpaceId,
    version: &VersionId,
) -> Result<Option<Checkpoint>, Error> {
    let path = checkpoint_path(space_dir, version);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::Io(err)),
    };
    match Checkpoint::decode(&bytes) {
        Some(cp) if &cp.space == space && &cp.version == version => Ok(Some(cp)),
        // Hash-valid but for another space/version, or invalid: discard.
        _ => {
            let _ = fs::remove_file(&path);
            Ok(None)
        }
    }
}

/// Enforce the bounded cache budget: evict oldest checkpoint files (by file
/// name, which is the version hex, eviction order by modified time) until the
/// tree fits. Checkpoints are freely discardable (D3, §6.6).
///
/// # Errors
/// Returns io errors.
pub(crate) fn enforce_cache_budget(space_dir: &Path, budget_bytes: u64) -> Result<(), Error> {
    let dir = cache_dir(space_dir).join(CHECKPOINT_DIR);
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(CHECKPOINT_EXT) {
            continue;
        }
        let meta = entry.metadata()?;
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        total += meta.len();
        files.push((modified, path, meta.len()));
    }
    if total <= budget_bytes {
        return Ok(());
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, path, len) in files {
        if total <= budget_bytes {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
    Ok(())
}

/// Build the checkpoint (merged native document per declared text epoch) for
/// the version at `heads`, and write it. `None` when the replay cost is below
/// `threshold` or the view exceeds the per-checkpoint cap (then the read path
/// stays on full replay).
///
/// # Errors
/// Returns SQLite/native errors from the replay; io errors from the write.
pub(crate) fn maybe_generate(
    conn: &Connection,
    space_dir: &Path,
    space: &SpaceId,
    heads: &[crate::identity::BatchId],
    threshold: u64,
    cache_budget_bytes: u64,
) -> Result<Option<u64>, Error> {
    let frontier = version::normalize(conn, space, heads)?;
    let version = frontier.id().map_err(profile_err)?;
    let cost = replay_cost(conn, &frontier.heads)?;
    if cost < threshold {
        return Ok(None);
    }
    let closure = projection::closure_set_pub(conn, &frontier.heads)?;
    let epochs = crate::history::declared_epochs(conn, &closure)?;
    let mut docs = Vec::with_capacity(epochs.len());
    for epoch in epochs {
        let doc = crate::native::doc_bytes_at(conn, &frontier.heads, &epoch, None)?;
        docs.push((epoch, doc));
    }
    let checkpoint = Checkpoint {
        space: *space,
        version,
        docs,
    };
    match write_checkpoint(space_dir, &checkpoint, cache_budget_bytes) {
        Ok(size) => Ok(Some(size)),
        Err(Error::Projection(ProjectionError::Encoding(_))) => Ok(None), // over cap: replay stays the path
        Err(err) => Err(err),
    }
}
