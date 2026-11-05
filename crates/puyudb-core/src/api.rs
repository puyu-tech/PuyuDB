// SPDX-License-Identifier: Apache-2.0

//! Public embedded facade for one local space (I06 P1/P2).
//!
//! [`Database`] is the entry point for embedding PuyuDB in a synchronous
//! application: no Tokio, no network, no CLI, no required runtime. Per §8.1,
//! [`Database::open_space`] never silently becomes an RPC client — it either
//! acquires the owner lock or fails with [`Error::Busy`]. All writes go
//! through [`Database::transact`], which takes a fully built [`Publication`]
//! (never a callback, so no core lock is held across user code) and reports
//! the §14 stable error codes.

use std::path::Path;

use thiserror::Error;

use crate::identity::{BatchId, ChunkId, EpochId, SpaceId};
use crate::profile::ArtifactManifest;
use crate::publication::{self, OpenMode, Store};

pub use crate::publication::{Publication, Receipt};

/// A second open of an owned space (§8.1).
///
/// The hint is deliberately non-secret: it names the owner-lock file and
/// nothing else. A second open never silently succeeds.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("database is owned by another handle (owner lock: {hint})")]
pub struct DatabaseBusy {
    hint: String,
}

impl DatabaseBusy {
    /// Non-secret hint: the path of the owner-lock file.
    #[must_use]
    pub fn hint(&self) -> &str {
        &self.hint
    }
}

/// Public failure of the embedded facade.
///
/// The first eleven variants are exactly the §14 stable codes of
/// `docs/spec/canonical-profile-v1.md`; [`Error::code`] returns them and
/// callers must never parse the rendered message instead. [`Error::Busy`] and
/// [`Error::Internal`] are environmental (owner lock, io, sqlite, unpatched
/// sqlite): they are deliberately *not* §14 codes and both report the
/// non-stable sentinel `"internal"` — distinguish them by variant, never by
/// code text.
#[derive(Debug, Error)]
pub enum Error {
    /// §14 `invalid_encoding`.
    #[error("invalid_encoding: {0}")]
    InvalidEncoding(String),
    /// §14 `unsupported_capability`. Profile `unsupported_version` folds
    /// into this variant at this facade.
    #[error("unsupported_capability: {0}")]
    UnsupportedCapability(String),
    /// §14 `hash_mismatch`.
    #[error("hash_mismatch: {0}")]
    HashMismatch(String),
    /// §14 `invalid_signature`.
    #[error("invalid_signature: {0}")]
    InvalidSignature(String),
    /// §14 `writer_equivocation`.
    #[error("writer_equivocation: {0}")]
    WriterEquivocation(String),
    /// §14 `missing_dependency`.
    #[error("missing_dependency: {0}")]
    MissingDependency(String),
    /// §14 `invalid_native_scope`.
    #[error("invalid_native_scope: {0}")]
    InvalidNativeScope(String),
    /// §14 `invalid_operation`.
    #[error("invalid_operation: {0}")]
    InvalidOperation(String),
    /// §14 `precondition_failed`: create on an existing space, open of a
    /// missing space, branch-frontier CAS mismatch, or idempotency-key reuse
    /// with different request content.
    #[error("precondition_failed: {0}")]
    PreconditionFailed(String),
    /// §14 `content_unavailable`.
    #[error("content_unavailable: {0}")]
    ContentUnavailable(String),
    /// §14 `corruption`.
    #[error("corruption: {0}")]
    Corruption(String),
    /// Another handle or process owns the space (§8.1).
    #[error("database busy: {0}")]
    Busy(#[from] DatabaseBusy),
    /// Environmental failure (io, sqlite, unpatched sqlite). Explicitly not
    /// a §14 code; never mislabel these as `corruption`.
    #[error("internal: {0}")]
    Internal(String),
}

impl Error {
    /// The §14 stable code for the profile-mapped variants. [`Error::Busy`]
    /// and [`Error::Internal`] are environmental and return the non-stable
    /// sentinel `"internal"`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidEncoding(_) => "invalid_encoding",
            Self::UnsupportedCapability(_) => "unsupported_capability",
            Self::HashMismatch(_) => "hash_mismatch",
            Self::InvalidSignature(_) => "invalid_signature",
            Self::WriterEquivocation(_) => "writer_equivocation",
            Self::MissingDependency(_) => "missing_dependency",
            Self::InvalidNativeScope(_) => "invalid_native_scope",
            Self::InvalidOperation(_) => "invalid_operation",
            Self::PreconditionFailed(_) => "precondition_failed",
            Self::ContentUnavailable(_) => "content_unavailable",
            Self::Corruption(_) => "corruption",
            Self::Busy(_) | Self::Internal(_) => "internal",
        }
    }
}

impl From<publication::Error> for Error {
    fn from(error: publication::Error) -> Self {
        use publication::Error as P;
        match error {
            // Environmental: mapped by the facade open path to Busy with the
            // owner-lock hint; anything reaching here is a plain internal error.
            P::Busy => Self::Internal("owner lock held by another handle".into()),
            P::Io(err) => Self::Internal(format!("io: {err}")),
            P::Sqlite(err) => Self::Internal(format!("sqlite: {err}")),
            P::UnpatchedSqlite(version) => Self::Internal(format!(
                "sqlite {version} predates the 3.51.3 WAL-reset fix"
            )),
            P::ChunkMismatch => {
                Self::HashMismatch("staged bytes do not hash to the chunk id".into())
            }
            P::ChunkTooLarge => Self::InvalidEncoding("chunk exceeds the 256 KiB raw cap".into()),
            P::MissingChunk => Self::ContentUnavailable("a required chunk is not staged".into()),
            P::Rejected(detail) => Self::InvalidOperation(detail),
            P::Precondition(detail) => Self::PreconditionFailed(detail),
            P::MissingDependency => {
                Self::MissingDependency("a parent or previous batch is not published".into())
            }
            P::Equivocation => {
                Self::WriterEquivocation("two batches claim one stream position".into())
            }
            P::InvalidNativeScope(detail) => Self::InvalidNativeScope(detail),
            P::Projection(err) => Self::from(err),
            P::Manifest(detail) => Self::Corruption(format!("space manifest: {detail}")),
            P::UnsupportedFormat(detail) => {
                Self::UnsupportedCapability(format!("unsupported format: {detail}"))
            }
            P::SpaceMismatch => {
                Self::PreconditionFailed("manifest and database name different spaces".into())
            }
            P::Corrupt(detail) => Self::Corruption(detail),
        }
    }
}

impl From<crate::projection::Error> for Error {
    fn from(error: crate::projection::Error) -> Self {
        use crate::projection::Error as Pr;
        match error {
            Pr::Sqlite(err) => Self::Internal(format!("sqlite: {err}")),
            Pr::Profile(err) => Self::from(err),
            Pr::Encoding(detail) => Self::InvalidEncoding(detail),
            Pr::InvalidOperation(detail) => Self::InvalidOperation(detail),
        }
    }
}

impl From<crate::profile::Error> for Error {
    fn from(error: crate::profile::Error) -> Self {
        match error {
            crate::profile::Error::Encoding(detail) => Self::InvalidEncoding(detail),
            crate::profile::Error::Version(detail) => {
                Self::UnsupportedCapability(format!("unsupported_version: {detail}"))
            }
            crate::profile::Error::Capability(detail) => Self::UnsupportedCapability(detail),
            crate::profile::Error::HashMismatch(detail) => Self::HashMismatch(detail),
            crate::profile::Error::Signature(detail) => Self::InvalidSignature(detail),
        }
    }
}

/// Map an open failure, giving [`publication::Error::Busy`] the non-secret
/// owner-lock hint (§8.1).
fn map_open(dir: &Path, error: publication::Error) -> Error {
    match error {
        publication::Error::Busy => Error::Busy(DatabaseBusy {
            hint: dir.join(publication::OWNER_LOCK_NAME).display().to_string(),
        }),
        other => Error::from(other),
    }
}

/// An owned handle to one local space.
///
/// Wraps the internal publication ledger. The synchronous engine needs no
/// runtime, and [`Database::transact`] takes a ready-made [`Publication`], so
/// no core lock is ever held across user code (§8.1).
pub struct Database {
    store: Store,
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

impl Database {
    /// Create a new space in `dir` and take the owner lock.
    ///
    /// # Errors
    /// Returns [`Error::PreconditionFailed`] ("space already exists") when
    /// the space already exists, and [`Error::Busy`] when another handle or
    /// process owns it.
    pub fn create_space(dir: impl AsRef<Path>, space: &SpaceId) -> Result<Self, Error> {
        let dir = dir.as_ref();
        // Fast path with the right code; the authoritative check runs under
        // the owner lock inside the store.
        if dir.join(publication::MANIFEST_NAME).exists() {
            return Err(Error::PreconditionFailed("space already exists".into()));
        }
        let store = Store::open_vfs_mode(dir, None, space, OpenMode::Create)
            .map_err(|err| map_open(dir, err))?;
        Ok(Self { store })
    }

    /// Open an existing space in `dir` and take the owner lock.
    ///
    /// # Errors
    /// Returns [`Error::PreconditionFailed`] ("space not found") when the
    /// space does not exist, and [`Error::Busy`] when another handle or
    /// process owns it — a second open never silently succeeds (§8.1).
    pub fn open_space(dir: impl AsRef<Path>, space: &SpaceId) -> Result<Self, Error> {
        let dir = dir.as_ref();
        if !dir.join(publication::MANIFEST_NAME).exists() {
            return Err(Error::PreconditionFailed("space not found".into()));
        }
        let store = Store::open_vfs_mode(dir, None, space, OpenMode::Open)
            .map_err(|err| map_open(dir, err))?;
        Ok(Self { store })
    }

    /// Durably stage one content chunk (thin forwarder to the ledger).
    ///
    /// # Errors
    /// Returns [`Error::InvalidEncoding`] above the frozen 256 KiB raw cap and
    /// [`Error::HashMismatch`] when `id` is not the digest of `bytes`.
    pub fn stage_chunk(&self, id: ChunkId, bytes: &[u8]) -> Result<(), Error> {
        Ok(self.store.stage_chunk(id, bytes)?)
    }

    /// Stage a complete content-addressed artifact (manifest + chunks).
    ///
    /// # Errors
    /// Returns [`Error::ContentUnavailable`]/[`Error::InvalidOperation`] when
    /// a chunk is not staged or the payload does not match the manifest.
    pub fn stage_artifact(
        &self,
        manifest: &ArtifactManifest,
        chunks: &[ChunkId],
    ) -> Result<(), Error> {
        Ok(self.store.stage_artifact(manifest, chunks)?)
    }

    /// Register the epoch-owned seed artifact for a text epoch (§12.4).
    ///
    /// # Errors
    /// Returns [`Error::ContentUnavailable`]/[`Error::InvalidOperation`] on
    /// unverified bytes and [`Error::InvalidNativeScope`] when the seed does
    /// not match the epoch's baseline.
    pub fn register_seed(
        &self,
        epoch: &EpochId,
        manifest: &ArtifactManifest,
        chunks: &[ChunkId],
    ) -> Result<(), Error> {
        Ok(self.store.register_seed(epoch, manifest, chunks)?)
    }

    /// Publish `publication` under `(caller, key)` with a branch-frontier CAS.
    ///
    /// No closures or callbacks: the caller builds the [`Publication`] before
    /// the call. `expected_frontier` is compared against the visible heads of
    /// the batch's branch inside the write transaction, after the idempotency
    /// check and before admission (§13.1: a retry of an own committed batch
    /// returns the stored receipt before the CAS, so retries never fail CAS
    /// on their own batch). A mismatch is [`Error::PreconditionFailed`] and
    /// leaves the key reusable (cancel-before-publish); after commit a retry
    /// with the same key and content returns the stored receipt
    /// (unknown-after-commit).
    ///
    /// # Errors
    /// Same mapping as the §14 codes; see [`Error`].
    pub fn transact(
        &self,
        caller: &[u8],
        key: &[u8],
        expected_frontier: &[BatchId],
        publication: &Publication<'_>,
    ) -> Result<Receipt, Error> {
        Ok(self
            .store
            .publish_transact(caller, key, expected_frontier, publication)?)
    }

    /// The committed receipt for an idempotency key, if one exists.
    ///
    /// # Errors
    /// Returns [`Error::Internal`] when the read fails.
    pub fn receipt_for(&self, caller: &[u8], key: &[u8]) -> Result<Option<Receipt>, Error> {
        Ok(self.store.receipt_for(caller, key)?)
    }
}
