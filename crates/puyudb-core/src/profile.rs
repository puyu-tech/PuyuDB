// SPDX-License-Identifier: Apache-2.0

//! Frozen canonical profile: records, digests, hashing and strict validation.
//!
//! Normative source: `docs/spec/canonical-profile-v1.md`. This module encodes the
//! layouts and validates them; it deliberately does **not** provide structured
//! decoders for every record, because no consumer needs one yet (storage and
//! projections arrive in I04/I05). [`canon::decode`] plus
//! [`validate_record`] already reject every non-canonical or out-of-profile
//! record, and re-encoding the decoded item tree must reproduce the input bytes.
//!
//! Nothing here is a publication path: there is no storage, admission, quota or
//! network code.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::canon::{self, Item};
use crate::identity::{
    ArtifactId, BatchId, BranchId, ChunkId, ConflictId, EpochId, FileId, GenerationNonce, GitOid,
    RepositoryId, SpaceId, VersionId, WriterEpoch,
};

/// Frozen profile version; the first element of every record.
pub const PROFILE_VERSION: u64 = 1;
/// Maximum canonical batch-body size.
pub const MAX_BATCH_BODY_BYTES: usize = 1_048_576;
/// Maximum direct parents of one batch.
pub const MAX_BATCH_PARENTS: usize = 128;
/// Maximum native artifacts referenced by one batch.
pub const MAX_BATCH_NATIVE_ARTIFACTS: usize = 4_096;
/// Maximum heads in one frontier record.
pub const MAX_FRONTIER_HEADS: usize = 65_536;
/// Maximum chunks in one artifact manifest.
pub const MAX_ARTIFACT_CHUNKS: usize = 4_096;
/// Raw size of every chunk except the final one.
pub const CHUNK_RAW_BYTES: usize = 262_144;
/// Declared artifact profile cap (not a quota system).
pub const MAX_ARTIFACT_RAW_BYTES: u64 = 1_073_741_824;
/// Maximum bytes of an artifact kind or profile string.
pub const MAX_KIND_TEXT_BYTES: usize = 64;
/// Maximum metadata operations per artifact.
pub const MAX_METADATA_OPERATIONS: usize = 65_536;
/// Maximum context assertions per artifact.
pub const MAX_CONTEXT_ASSERTIONS: usize = 4_096;
/// Maximum context-event payload.
pub const MAX_CONTEXT_EVENT_BYTES: usize = 65_536;
/// Maximum logical path bytes.
pub const MAX_PATH_BYTES: usize = 4_096;
/// Maximum bytes of one path component.
pub const MAX_PATH_COMPONENT_BYTES: usize = 255;
/// Maximum text epoch size in v1.
pub const MAX_TEXT_EPOCH_BYTES: usize = 8_388_608;
/// Maximum operation index inside a metadata artifact.
pub const MAX_OPERATION_INDEX: u64 = 65_535;

/// Frozen native text profile string.
pub const TEXT_PROFILE: &str = "dt-text-v1";

const BATCH_DOMAIN: &[u8] = b"puyudb/batch/v1";
const SIGNATURE_DOMAIN: &[u8] = b"puyudb/signature/v1";
const FRONTIER_DOMAIN: &[u8] = b"puyudb/frontier/v1";
const CHUNK_DOMAIN: &[u8] = b"puyudb/chunk/v1";
const ARTIFACT_DOMAIN: &[u8] = b"puyudb/artifact/v1";
const CONTENT_DOMAIN: &[u8] = b"puyudb/content/v1";
const EPOCH_DOMAIN: &[u8] = b"puyudb/epoch/v1";
const CONFLICT_DOMAIN: &[u8] = b"puyudb/conflict/v1";
const BRANCH_DESCRIPTOR_DOMAIN: &[u8] = b"puyudb/branch-descriptor/v1";
const BRANCH_SIGNATURE_DOMAIN: &[u8] = b"puyudb/branch-signature/v1";
const SEMANTIC_MANIFEST_DOMAIN: &[u8] = b"puyudb/semantic-manifest/v1";
const DT_SEED_ACTOR_DOMAIN: &[u8] = b"puyudb/dt-seed-actor/v1";
const DT_ACTOR_DOMAIN: &[u8] = b"puyudb/dt-actor/v1";

/// Assigned artifact kinds.
pub const ASSIGNED_KINDS: [&str; 6] = [
    "native-change-v1",
    "metadata-v1",
    "text-baseline-v1",
    "opaque-baseline-v1",
    "symlink-target-v1",
    "branch-descriptor-v1",
];
/// Kinds reserved for later issues; a v1 decoder rejects them.
pub const RESERVED_KINDS: [&str; 2] = ["named-snapshot-v1", "archive-coverage-v1"];

const LIVE_TAGS: [u64; 2] = [0, 1];
const MODE_TAGS: [u64; 3] = [0, 1, 2];
const EPOCH_KIND_TAGS: [u64; 3] = [0, 1, 2];
const FIELD_TAGS: [u64; 3] = [0, 1, 2];
const CONTEXT_SCHEMA_TAGS: [u64; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
const OPERATION_TAGS: [u64; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
const RESOLUTION_TAGS: [u64; 5] = [0, 1, 2, 3, 4];
const CONFLICT_KIND_TAGS: [u64; 3] = [0, 1, 2];
const RESERVED_SEGMENTS: [&str; 2] = [".git", ".puyu"];

/// Profile failure carrying the stable public code.
#[derive(Debug, Error)]
pub enum Error {
    /// Structurally non-canonical or out-of-profile input.
    #[error("invalid_encoding: {0}")]
    Encoding(String),
    /// Unknown profile version.
    #[error("unsupported_version: {0}")]
    Version(String),
    /// Known-but-unassigned or reserved semantics.
    #[error("unsupported_capability: {0}")]
    Capability(String),
    /// Declared length or digest disagrees with the record.
    #[error("hash_mismatch: {0}")]
    HashMismatch(String),
    /// Signature does not verify for the declared preimage.
    #[error("invalid_signature: {0}")]
    Signature(String),
}

impl Error {
    /// Stable public code; never parse the rendered message instead.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Encoding(_) => "invalid_encoding",
            Self::Version(_) => "unsupported_version",
            Self::Capability(_) => "unsupported_capability",
            Self::HashMismatch(_) => "hash_mismatch",
            Self::Signature(_) => "invalid_signature",
        }
    }

    fn encoding(detail: impl Into<String>) -> Self {
        Self::Encoding(detail.into())
    }
}

impl From<canon::Error> for Error {
    fn from(error: canon::Error) -> Self {
        Self::Encoding(error.to_string())
    }
}

fn digest(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0u8]);
    for field in fields {
        hasher.update(field);
    }
    hasher.finalize().into()
}

// --------------------------------------------------------------------------
// Digests
// --------------------------------------------------------------------------

/// `BatchId` of one canonical batch body.
#[must_use]
pub fn batch_id(body: &[u8]) -> BatchId {
    BatchId::from_array(digest(BATCH_DOMAIN, &[body]))
}

/// `VersionId` of one canonical frontier body.
#[must_use]
pub fn version_id(body: &[u8]) -> VersionId {
    VersionId::from_array(digest(FRONTIER_DOMAIN, &[body]))
}

/// `ChunkId` of one raw chunk.
#[must_use]
pub fn chunk_id(raw: &[u8]) -> ChunkId {
    ChunkId::from_array(digest(CHUNK_DOMAIN, &[raw]))
}

/// Kind-specific raw content digest.
#[must_use]
pub fn raw_digest(kind: &str, payload: &[u8]) -> [u8; 32] {
    digest(CONTENT_DOMAIN, &[kind.as_bytes(), &[0u8], payload])
}

/// `ArtifactId` of one canonical manifest body.
#[must_use]
pub fn artifact_id(body: &[u8]) -> ArtifactId {
    ArtifactId::from_array(digest(ARTIFACT_DOMAIN, &[body]))
}

/// `EpochId` of one canonical epoch descriptor.
#[must_use]
pub fn epoch_id(body: &[u8]) -> EpochId {
    EpochId::from_array(digest(EPOCH_DOMAIN, &[body]))
}

/// `ConflictId` of one canonical conflict record.
#[must_use]
pub fn conflict_id(body: &[u8]) -> ConflictId {
    ConflictId::from_array(digest(CONFLICT_DOMAIN, &[body]))
}

/// Identity of one signed branch descriptor body.
#[must_use]
pub fn branch_descriptor_id(body: &[u8]) -> [u8; 32] {
    digest(BRANCH_DESCRIPTOR_DOMAIN, &[body])
}

/// `SemanticManifestId` of one canonical semantic manifest.
#[must_use]
pub fn semantic_manifest_id(body: &[u8]) -> [u8; 32] {
    digest(SEMANTIC_MANIFEST_DOMAIN, &[body])
}

// --------------------------------------------------------------------------
// Paths
// --------------------------------------------------------------------------

/// Logical path: UTF-8, slash-separated, byte-exact, never normalized.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Path(String);

impl Path {
    /// Validate and wrap a logical path.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] for absolute paths, dot components, empty
    /// components, NUL, `.git`/`.puyu` segments, and the frozen size caps.
    pub fn new(value: &str) -> Result<Self, Error> {
        if value.is_empty() {
            return Err(Error::encoding("path is empty"));
        }
        if value.len() > MAX_PATH_BYTES {
            return Err(Error::encoding("path exceeds the profile cap"));
        }
        if value.contains('\0') {
            return Err(Error::encoding("path contains NUL"));
        }
        if value.starts_with('/') || value.ends_with('/') {
            return Err(Error::encoding("path is absolute or has a trailing slash"));
        }
        for segment in value.split('/') {
            if segment.is_empty() {
                return Err(Error::encoding("path has an empty component"));
            }
            if segment.len() > MAX_PATH_COMPONENT_BYTES {
                return Err(Error::encoding("path component exceeds the profile cap"));
            }
            if segment == "." || segment == ".." {
                return Err(Error::encoding("path contains a dot component"));
            }
            if RESERVED_SEGMENTS.contains(&segment) {
                return Err(Error::encoding("path uses a reserved control segment"));
            }
        }
        Ok(Self(value.to_owned()))
    }

    /// The validated path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// --------------------------------------------------------------------------
// Actor names and signatures
// --------------------------------------------------------------------------

/// Frozen DT seed actor name: `s:` plus 43 base64url characters.
#[must_use]
pub fn seed_actor_name(epoch: &EpochId) -> String {
    format!(
        "s:{}",
        base64url_43(&digest(DT_SEED_ACTOR_DOMAIN, &[epoch.as_bytes()]))
    )
}

/// Frozen DT writer actor name binding the whole writer context.
#[must_use]
pub fn writer_actor_name(
    public_key: &[u8; 32],
    branch: &BranchId,
    writer_epoch: &WriterEpoch,
    epoch: &EpochId,
) -> String {
    format!(
        "w:{}",
        base64url_43(&digest(
            DT_ACTOR_DOMAIN,
            &[
                public_key,
                branch.as_bytes(),
                writer_epoch.as_bytes(),
                epoch.as_bytes()
            ]
        ))
    )
}

fn base64url_43(value: &[u8; 32]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(43);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in value {
        accumulator = (accumulator << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(char::from(
                ALPHABET[((accumulator >> bits) & 0x3f) as usize],
            ));
        }
        accumulator &= (1u32 << bits) - 1;
    }
    if bits > 0 {
        // 32 bytes leave four bits over, which become the final character.
        out.push(char::from(
            ALPHABET[((accumulator << (6 - bits)) & 0x3f) as usize],
        ));
    }
    debug_assert_eq!(out.len(), 43);
    out
}

/// Batch signature preimage: `space16 || batch32` after the domain separator.
#[must_use]
pub fn batch_signature_preimage(space: &SpaceId, batch: &BatchId) -> Vec<u8> {
    let mut out = Vec::with_capacity(SIGNATURE_DOMAIN.len() + 1 + 16 + 32);
    out.extend_from_slice(SIGNATURE_DOMAIN);
    out.push(0);
    out.extend_from_slice(space.as_bytes());
    out.extend_from_slice(batch.as_bytes());
    out
}

/// Branch descriptor signature preimage.
#[must_use]
pub fn branch_signature_preimage(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BRANCH_SIGNATURE_DOMAIN.len() + 1 + body.len());
    out.extend_from_slice(BRANCH_SIGNATURE_DOMAIN);
    out.push(0);
    out.extend_from_slice(body);
    out
}

fn signing_key(seed: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(seed)
}

/// Deterministic Ed25519 signature over a batch preimage (fixtures and tests).
#[must_use]
pub fn sign_batch(seed: &[u8; 32], space: &SpaceId, batch: &BatchId) -> [u8; 64] {
    signing_key(seed)
        .sign(&batch_signature_preimage(space, batch))
        .to_bytes()
}

/// Deterministic Ed25519 signature over a branch descriptor body.
#[must_use]
pub fn sign_branch_descriptor(seed: &[u8; 32], body: &[u8]) -> [u8; 64] {
    signing_key(seed)
        .sign(&branch_signature_preimage(body))
        .to_bytes()
}

/// Verifying key bytes for a seed (fixtures and tests).
#[must_use]
pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    signing_key(seed).verifying_key().to_bytes()
}

fn verify_strict(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    key.verify_strict(message, &Signature::from_bytes(signature))
        .is_ok()
}

/// Verify a batch signature with strict Ed25519 rules.
///
/// # Errors
/// Returns [`Error::Signature`] when the key or signature is malformed or the
/// signature does not verify.
pub fn verify_batch(
    body: &BatchBody,
    signature_key: &[u8; 32],
    signature: &[u8; 64],
) -> Result<BatchId, Error> {
    // §4.2: the signature key must equal the body's writer key. Verifying the
    // signature and the body separately would let one signer attest a batch that
    // names a different writer.
    if signature_key != &body.public_key {
        return Err(Error::Signature(
            "signature key does not match the batch body's writer key".to_owned(),
        ));
    }
    let batch = body.id()?;
    if verify_strict(
        signature_key,
        &batch_signature_preimage(&body.space, &batch),
        signature,
    ) {
        Ok(batch)
    } else {
        Err(Error::Signature(
            "signature does not verify for the batch preimage".to_owned(),
        ))
    }
}

// --------------------------------------------------------------------------
// Records: encoders
// --------------------------------------------------------------------------

fn wrap(write: impl FnOnce(&mut Vec<u8>) -> Result<(), Error>) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    write(&mut out)?;
    Ok(out)
}

fn push_bytes(out: &mut Vec<u8>, value: &[u8]) -> Result<(), Error> {
    canon::write_bytes(out, value)?;
    Ok(())
}

fn push_text(out: &mut Vec<u8>, value: &str) -> Result<(), Error> {
    canon::write_text(out, value)?;
    Ok(())
}

fn push_uints(out: &mut Vec<u8>, values: impl ExactSizeIterator<Item = u64>) -> Result<(), Error> {
    canon::write_array_header(out, values.len())?;
    for value in values {
        canon::write_uint(out, value)?;
    }
    Ok(())
}

/// Tags used by liveness assignments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveTag {
    /// Deleted.
    Deleted,
    /// Alive.
    Alive,
}

impl LiveTag {
    const fn tag(self) -> u64 {
        match self {
            Self::Deleted => 0,
            Self::Alive => 1,
        }
    }
}

/// Supported mode tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeTag {
    /// Regular non-executable file.
    Regular,
    /// Regular executable file.
    Executable,
    /// Symlink entry.
    Symlink,
}

impl ModeTag {
    const fn tag(self) -> u64 {
        match self {
            Self::Regular => 0,
            Self::Executable => 1,
            Self::Symlink => 2,
        }
    }
}

/// Epoch content kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochKind {
    /// UTF-8 text with a native seed.
    Text,
    /// Opaque bytes.
    Opaque,
    /// Inert symlink target bytes.
    Symlink,
}

impl EpochKind {
    const fn tag(self) -> u64 {
        match self {
            Self::Text => 0,
            Self::Opaque => 1,
            Self::Symlink => 2,
        }
    }
}

/// Conflict kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Concurrent register alternatives.
    Register,
    /// Concurrent deletion and edit.
    DeleteEdit,
    /// Concurrent epoch selection and edit.
    EpochEdit,
}

impl ConflictKind {
    const fn tag(self) -> u64 {
        match self {
            Self::Register => 0,
            Self::DeleteEdit => 1,
            Self::EpochEdit => 2,
        }
    }
}

/// Immutable batch publication body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchBody {
    /// Space this batch belongs to.
    pub space: SpaceId,
    /// Branch this batch advances.
    pub branch: BranchId,
    /// Writer verifying key.
    pub public_key: [u8; 32],
    /// Writer-stream incarnation.
    pub writer_epoch: WriterEpoch,
    /// Stream sequence, starting at 1.
    pub sequence: u64,
    /// Preceding batch of the same stream.
    pub previous: Option<BatchId>,
    /// Declared parent cut, sorted and minimal.
    pub parents: Vec<BatchId>,
    /// Causal clock, `1 + max(parent lamport)`.
    pub lamport: u64,
    /// Native artifacts in edit order.
    pub native_artifacts: Vec<ArtifactId>,
    /// Metadata artifact for this batch.
    pub metadata: ArtifactId,
}

impl BatchBody {
    /// Encode the canonical body.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] when a cap is exceeded.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.parents.len() > MAX_BATCH_PARENTS {
            return Err(Error::encoding("parents exceed the profile cap"));
        }
        if self.native_artifacts.len() > MAX_BATCH_NATIVE_ARTIFACTS {
            return Err(Error::encoding("native artifacts exceed the profile cap"));
        }
        let body = wrap(|out| {
            canon::write_array_header(out, 11)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, self.space.as_bytes())?;
            push_bytes(out, self.branch.as_bytes())?;
            push_bytes(out, &self.public_key)?;
            push_bytes(out, self.writer_epoch.as_bytes())?;
            canon::write_uint(out, self.sequence)?;
            match &self.previous {
                Some(previous) => push_bytes(out, previous.as_bytes())?,
                None => canon::write_null(out),
            }
            canon::write_array_header(out, self.parents.len())?;
            for parent in &self.parents {
                push_bytes(out, parent.as_bytes())?;
            }
            canon::write_uint(out, self.lamport)?;
            canon::write_array_header(out, self.native_artifacts.len())?;
            for artifact in &self.native_artifacts {
                push_bytes(out, artifact.as_bytes())?;
            }
            push_bytes(out, self.metadata.as_bytes())
        })?;
        if body.len() > MAX_BATCH_BODY_BYTES {
            return Err(Error::encoding("batch body exceeds the profile cap"));
        }
        Ok(body)
    }

    /// Identity of the canonical body.
    ///
    /// # Errors
    /// Propagates [`BatchBody::encode`] failures.
    pub fn id(&self) -> Result<BatchId, Error> {
        Ok(batch_id(&self.encode()?))
    }
}

/// Normalized frontier for one space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontier {
    /// Space this frontier belongs to.
    pub space: SpaceId,
    /// Sorted, unique, minimal heads.
    pub heads: Vec<BatchId>,
}

impl Frontier {
    /// Encode the canonical frontier.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] when the head cap is exceeded.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.heads.len() > MAX_FRONTIER_HEADS {
            return Err(Error::encoding("frontier heads exceed the profile cap"));
        }
        let mut heads = self.heads.clone();
        heads.sort_unstable();
        heads.dedup();
        wrap(|out| {
            canon::write_array_header(out, 3)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, self.space.as_bytes())?;
            canon::write_array_header(out, heads.len())?;
            for head in &heads {
                push_bytes(out, head.as_bytes())?;
            }
            Ok(())
        })
    }

    /// Identity of the canonical frontier.
    ///
    /// # Errors
    /// Propagates [`Frontier::encode`] failures.
    pub fn id(&self) -> Result<VersionId, Error> {
        Ok(version_id(&self.encode()?))
    }
}

/// Artifact manifest framing one raw payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactManifest {
    /// Assigned kind text.
    pub kind: String,
    /// Declared raw payload length.
    pub raw_length: u64,
    /// Kind-bound raw content digest.
    pub raw_digest: [u8; 32],
    /// Ordered chunk identities.
    pub chunks: Vec<ChunkId>,
}

impl ArtifactManifest {
    /// Frame a payload into chunks at [`CHUNK_RAW_BYTES`].
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] for an oversized kind, payload or chunk list.
    pub fn for_payload(kind: &str, payload: &[u8]) -> Result<Self, Error> {
        if kind.len() > MAX_KIND_TEXT_BYTES {
            return Err(Error::encoding("artifact kind exceeds the profile cap"));
        }
        if payload.len() as u64 > MAX_ARTIFACT_RAW_BYTES {
            return Err(Error::encoding("artifact exceeds the profile cap"));
        }
        let chunks: Vec<ChunkId> = payload.chunks(CHUNK_RAW_BYTES).map(chunk_id).collect();
        if chunks.len() > MAX_ARTIFACT_CHUNKS {
            return Err(Error::encoding(
                "artifact needs more chunks than the profile allows",
            ));
        }
        Ok(Self {
            kind: kind.to_owned(),
            raw_length: payload.len() as u64,
            raw_digest: raw_digest(kind, payload),
            chunks,
        })
    }

    /// Encode the canonical manifest.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] when the chunk cap is exceeded.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.chunks.len() > MAX_ARTIFACT_CHUNKS {
            return Err(Error::encoding("chunk list exceeds the profile cap"));
        }
        wrap(|out| {
            canon::write_array_header(out, 5)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_text(out, &self.kind)?;
            canon::write_uint(out, self.raw_length)?;
            push_bytes(out, &self.raw_digest)?;
            canon::write_array_header(out, self.chunks.len())?;
            for chunk in &self.chunks {
                push_bytes(out, chunk.as_bytes())?;
            }
            Ok(())
        })
    }

    /// Identity of the canonical manifest.
    ///
    /// # Errors
    /// Propagates [`ArtifactManifest::encode`] failures.
    pub fn id(&self) -> Result<ArtifactId, Error> {
        Ok(artifact_id(&self.encode()?))
    }
}

/// Native text artifact payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeArtifact {
    /// Content epoch this artifact belongs to.
    pub epoch: EpochId,
    /// Frozen engine profile string.
    pub profile: String,
    /// Producer-owned native bytes.
    pub native: Vec<u8>,
}

impl NativeArtifact {
    /// Encode the canonical payload.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] when the profile string or payload is oversized.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.profile.len() > MAX_KIND_TEXT_BYTES {
            return Err(Error::encoding("native profile exceeds the profile cap"));
        }
        wrap(|out| {
            canon::write_array_header(out, 4)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, self.epoch.as_bytes())?;
            push_text(out, &self.profile)?;
            push_bytes(out, &self.native)
        })
    }
}

/// Content-epoch descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpochDescriptor {
    /// File this epoch belongs to.
    pub file: FileId,
    /// Discriminator making otherwise identical epochs distinct.
    pub nonce: GenerationNonce,
    /// Content kind.
    pub kind: EpochKind,
    /// Baseline content artifact.
    pub baseline: ArtifactId,
    /// Required for text epochs, absent otherwise.
    pub seed_profile: Option<String>,
    /// Optional Git object association.
    pub git_oid: Option<GitOid>,
}

impl EpochDescriptor {
    /// Encode the canonical descriptor.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] for an inconsistent kind/profile pair.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        match (&self.kind, &self.seed_profile) {
            (EpochKind::Text, Some(profile)) => {
                if profile != TEXT_PROFILE {
                    return Err(Error::Capability(
                        "only dt-text-v1 is a frozen text profile".to_owned(),
                    ));
                }
            }
            (EpochKind::Text, None) => {
                return Err(Error::encoding("text epochs require the DT seed profile"));
            }
            (_, Some(_)) => {
                return Err(Error::encoding(
                    "non-text epochs must not declare a native seed profile",
                ));
            }
            (_, None) => {}
        }
        wrap(|out| {
            canon::write_array_header(out, 7)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, self.file.as_bytes())?;
            push_bytes(out, self.nonce.as_bytes())?;
            canon::write_uint(out, self.kind.tag())?;
            push_bytes(out, self.baseline.as_bytes())?;
            match &self.seed_profile {
                Some(profile) => push_text(out, profile)?,
                None => canon::write_null(out),
            }
            match &self.git_oid {
                Some(oid) => push_git_oid(out, oid)?,
                None => canon::write_null(out),
            }
            Ok(())
        })
    }

    /// Identity of the canonical descriptor.
    ///
    /// # Errors
    /// Propagates [`EpochDescriptor::encode`] failures.
    pub fn id(&self) -> Result<EpochId, Error> {
        Ok(epoch_id(&self.encode()?))
    }
}

fn push_git_oid(out: &mut Vec<u8>, oid: &GitOid) -> Result<(), Error> {
    canon::write_array_header(out, 3)?;
    canon::write_uint(out, PROFILE_VERSION)?;
    canon::write_uint(out, oid.algorithm_tag())?;
    push_bytes(out, oid.digest())
}

/// Identity of one field write: the batch and the operation's array position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WriteIdent {
    /// Publishing batch.
    pub batch: BatchId,
    /// Zero-based position in the metadata artifact's operation array.
    pub operation_index: u64,
}

/// Reconstructed cross-field conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Conflict kind.
    pub kind: ConflictKind,
    /// File the conflict concerns.
    pub file: FileId,
    /// Epochs involved, sorted unique; empty for register conflicts.
    pub epochs: Vec<EpochId>,
    /// Maximal concurrent participants, sorted by batch then index.
    pub participants: Vec<WriteIdent>,
}

impl Conflict {
    /// Encode the canonical conflict record.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] when an index exceeds the profile cap.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        for participant in &self.participants {
            if participant.operation_index > MAX_OPERATION_INDEX {
                return Err(Error::encoding("operation index exceeds the profile cap"));
            }
        }
        wrap(|out| {
            canon::write_array_header(out, 5)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            canon::write_uint(out, self.kind.tag())?;
            push_bytes(out, self.file.as_bytes())?;
            canon::write_array_header(out, self.epochs.len())?;
            for epoch in &self.epochs {
                push_bytes(out, epoch.as_bytes())?;
            }
            canon::write_array_header(out, self.participants.len())?;
            for participant in &self.participants {
                canon::write_array_header(out, 2)?;
                push_bytes(out, participant.batch.as_bytes())?;
                canon::write_uint(out, participant.operation_index)?;
            }
            Ok(())
        })
    }

    /// Identity of the canonical record.
    ///
    /// # Errors
    /// Propagates [`Conflict::encode`] failures.
    pub fn id(&self) -> Result<ConflictId, Error> {
        Ok(conflict_id(&self.encode()?))
    }
}

/// Signed space-catalog branch descriptor body (signature excluded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchDescriptor {
    /// Space the branch belongs to.
    pub space: SpaceId,
    /// Branch identity.
    pub branch: BranchId,
    /// Captured initial version; never rewritten.
    pub initial_version: VersionId,
    /// Creator verifying key.
    pub creator: [u8; 32],
}

impl BranchDescriptor {
    /// Encode the canonical descriptor body.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] if encoding fails.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        wrap(|out| {
            canon::write_array_header(out, 5)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, self.space.as_bytes())?;
            push_bytes(out, self.branch.as_bytes())?;
            push_bytes(out, self.initial_version.as_bytes())?;
            push_bytes(out, &self.creator)
        })
    }

    /// Identity of the descriptor body.
    ///
    /// # Errors
    /// Propagates [`BranchDescriptor::encode`] failures.
    pub fn id(&self) -> Result<[u8; 32], Error> {
        Ok(branch_descriptor_id(&self.encode()?))
    }
}

/// Signed envelope around a branch descriptor body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchEnvelope {
    /// Canonical descriptor body bytes.
    pub body: Vec<u8>,
    /// Ed25519 signature over the branch preimage.
    pub signature: [u8; 64],
}

impl BranchEnvelope {
    /// Encode the canonical envelope.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] if encoding fails.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        wrap(|out| {
            canon::write_array_header(out, 3)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, &self.body)?;
            push_bytes(out, &self.signature)
        })
    }
}

/// Per-file alternative set used for cross-replica comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// File identity.
    pub file: FileId,
    /// Maximal liveness alternatives.
    pub live: Vec<LiveTag>,
    /// Maximal path alternatives.
    pub paths: Vec<Path>,
    /// Maximal epoch alternatives.
    pub epochs: Vec<EpochId>,
    /// Maximal opaque content alternatives.
    pub contents: Vec<ArtifactId>,
    /// Maximal mode alternatives.
    pub modes: Vec<ModeTag>,
    /// Conflicts visible at this cut.
    pub conflicts: Vec<ConflictId>,
}

impl FileEntry {
    /// Encode the canonical entry with sorted, deduplicated alternatives.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] if encoding fails.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut live: Vec<u64> = self.live.iter().map(|tag| tag.tag()).collect();
        live.sort_unstable();
        live.dedup();
        let mut modes: Vec<u64> = self.modes.iter().map(|tag| tag.tag()).collect();
        modes.sort_unstable();
        modes.dedup();
        let mut paths: Vec<&str> = self.paths.iter().map(Path::as_str).collect();
        paths.sort_unstable();
        paths.dedup();
        let mut epochs = self.epochs.clone();
        epochs.sort_unstable();
        epochs.dedup();
        let mut contents = self.contents.clone();
        contents.sort_unstable();
        contents.dedup();
        let mut conflicts = self.conflicts.clone();
        conflicts.sort_unstable();
        conflicts.dedup();
        wrap(|out| {
            canon::write_array_header(out, 8)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, self.file.as_bytes())?;
            push_uints(out, live.iter().copied())?;
            canon::write_array_header(out, paths.len())?;
            for path in &paths {
                push_text(out, path)?;
            }
            canon::write_array_header(out, epochs.len())?;
            for epoch in &epochs {
                push_bytes(out, epoch.as_bytes())?;
            }
            canon::write_array_header(out, contents.len())?;
            for content in &contents {
                push_bytes(out, content.as_bytes())?;
            }
            push_uints(out, modes.iter().copied())?;
            canon::write_array_header(out, conflicts.len())?;
            for conflict in &conflicts {
                push_bytes(out, conflict.as_bytes())?;
            }
            Ok(())
        })
    }
}

/// Derived semantic projection of one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticManifest {
    /// Space the manifest describes.
    pub space: SpaceId,
    /// Per-file entries; encoding sorts them by raw file identity.
    pub entries: Vec<FileEntry>,
}

impl SemanticManifest {
    /// Encode the canonical manifest.
    ///
    /// # Errors
    /// Propagates entry encoding failures.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut encoded: Vec<(FileId, Vec<u8>)> = self
            .entries
            .iter()
            .map(|entry| Ok((entry.file, entry.encode()?)))
            .collect::<Result<_, Error>>()?;
        encoded.sort_by_key(|(file, _)| *file);
        wrap(|out| {
            canon::write_array_header(out, 3)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            push_bytes(out, self.space.as_bytes())?;
            canon::write_array_header(out, encoded.len())?;
            for (_, body) in &encoded {
                out.extend_from_slice(body);
            }
            Ok(())
        })
    }

    /// Identity of the canonical manifest.
    ///
    /// # Errors
    /// Propagates [`SemanticManifest::encode`] failures.
    pub fn id(&self) -> Result<[u8; 32], Error> {
        Ok(semantic_manifest_id(&self.encode()?))
    }
}

/// Resolution payload for a conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Keep the deletion.
    RetainDelete,
    /// Restore the edited content.
    RestoreEdit,
    /// Keep the named epoch.
    RetainEpoch(EpochId),
    /// Incorporate old-epoch work with a provenance artifact.
    Incorporate {
        /// New explicit epoch.
        epoch: EpochId,
        /// Provenance artifact recording the incorporation.
        provenance: ArtifactId,
    },
    /// Choose one alternative of one field.
    Choose {
        /// Field tag: 0 path, 1 content value, 2 mode.
        field: u64,
        /// Chosen write identity.
        write: WriteIdent,
    },
}

/// One metadata operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// Create a fresh `FileId`.
    Create {
        /// Repository binding.
        repository: RepositoryId,
        /// New file identity.
        file: FileId,
        /// Initial path.
        path: Path,
        /// Initial mode.
        mode: ModeTag,
        /// Initial epoch.
        epoch: EpochDescriptor,
    },
    /// Causal path assignment.
    Path {
        /// Target file.
        file: FileId,
        /// Assigned path.
        path: Path,
    },
    /// Liveness assignment.
    Live {
        /// Target file.
        file: FileId,
        /// Liveness tag.
        live: LiveTag,
    },
    /// Epoch selection.
    Epoch {
        /// Target file.
        file: FileId,
        /// Selected epoch.
        epoch: EpochDescriptor,
    },
    /// Opaque value assignment.
    Opaque {
        /// Target file.
        file: FileId,
        /// Declared opaque epoch.
        epoch: EpochId,
        /// Content artifact.
        artifact: ArtifactId,
    },
    /// Bounded generic context event.
    Context {
        /// Context schema tag.
        schema: u64,
        /// Event identity.
        event: [u8; 16],
        /// Opaque bounded payload.
        bytes: Vec<u8>,
    },
    /// Git association assertion.
    Git {
        /// Repository binding.
        repository: RepositoryId,
        /// Git object.
        oid: GitOid,
        /// Associated version.
        version: VersionId,
        /// Relation tag.
        relation: u64,
    },
    /// Mode assignment.
    Mode {
        /// Target file.
        file: FileId,
        /// Assigned mode.
        mode: ModeTag,
    },
    /// Explicit conflict resolution.
    Resolution {
        /// Conflict being resolved.
        conflict: ConflictId,
        /// Chosen resolution.
        resolution: Resolution,
    },
    /// Provenance assertion for reapplied or imported work.
    Provenance {
        /// Source batches.
        sources: Vec<BatchId>,
        /// Relation tag.
        relation: u64,
        /// Optional detail artifact.
        detail: Option<ArtifactId>,
    },
}

impl Operation {
    /// Encode the operation array.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] or [`Error::Capability`] for invalid input.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        self.write(&mut out)?;
        Ok(out)
    }

    fn write(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        match self {
            Self::Create {
                repository,
                file,
                path,
                mode,
                epoch,
            } => {
                canon::write_array_header(out, 6)?;
                canon::write_uint(out, 0)?;
                push_bytes(out, repository.as_bytes())?;
                push_bytes(out, file.as_bytes())?;
                push_text(out, path.as_str())?;
                canon::write_uint(out, mode.tag())?;
                out.extend_from_slice(&epoch.encode()?);
                Ok(())
            }
            Self::Path { file, path } => {
                canon::write_array_header(out, 3)?;
                canon::write_uint(out, 1)?;
                push_bytes(out, file.as_bytes())?;
                push_text(out, path.as_str())
            }
            Self::Live { file, live } => {
                canon::write_array_header(out, 3)?;
                canon::write_uint(out, 2)?;
                push_bytes(out, file.as_bytes())?;
                Ok(canon::write_uint(out, live.tag())?)
            }
            Self::Epoch { file, epoch } => {
                canon::write_array_header(out, 3)?;
                canon::write_uint(out, 3)?;
                push_bytes(out, file.as_bytes())?;
                out.extend_from_slice(&epoch.encode().map_err(|_| canon::Error::Truncated)?);
                Ok(())
            }
            Self::Opaque {
                file,
                epoch,
                artifact,
            } => {
                canon::write_array_header(out, 4)?;
                canon::write_uint(out, 4)?;
                push_bytes(out, file.as_bytes())?;
                push_bytes(out, epoch.as_bytes())?;
                push_bytes(out, artifact.as_bytes())
            }
            Self::Context {
                schema,
                event,
                bytes,
            } => {
                canon::write_array_header(out, 4)?;
                canon::write_uint(out, 5)?;
                canon::write_uint(out, *schema)?;
                push_bytes(out, event)?;
                push_bytes(out, bytes)
            }
            Self::Git {
                repository,
                oid,
                version,
                relation,
            } => {
                canon::write_array_header(out, 5)?;
                canon::write_uint(out, 6)?;
                push_bytes(out, repository.as_bytes())?;
                push_git_oid(out, oid)?;
                push_bytes(out, version.as_bytes())?;
                Ok(canon::write_uint(out, *relation)?)
            }
            Self::Mode { file, mode } => {
                canon::write_array_header(out, 3)?;
                canon::write_uint(out, 7)?;
                push_bytes(out, file.as_bytes())?;
                Ok(canon::write_uint(out, mode.tag())?)
            }
            Self::Resolution {
                conflict,
                resolution,
            } => {
                canon::write_array_header(out, 4)?;
                canon::write_uint(out, 8)?;
                push_bytes(out, conflict.as_bytes())?;
                match resolution {
                    Resolution::RetainDelete => {
                        canon::write_uint(out, 0)?;
                        canon::write_null(out);
                    }
                    Resolution::RestoreEdit => {
                        canon::write_uint(out, 1)?;
                        canon::write_null(out);
                    }
                    Resolution::RetainEpoch(epoch) => {
                        canon::write_uint(out, 2)?;
                        push_bytes(out, epoch.as_bytes())?;
                    }
                    Resolution::Incorporate { epoch, provenance } => {
                        canon::write_uint(out, 3)?;
                        canon::write_array_header(out, 2)?;
                        push_bytes(out, epoch.as_bytes())?;
                        push_bytes(out, provenance.as_bytes())?;
                    }
                    Resolution::Choose { field, write } => {
                        canon::write_uint(out, 4)?;
                        canon::write_array_header(out, 2)?;
                        canon::write_uint(out, *field)?;
                        canon::write_array_header(out, 2)?;
                        push_bytes(out, write.batch.as_bytes())?;
                        canon::write_uint(out, write.operation_index)?;
                    }
                }
                Ok(())
            }
            Self::Provenance {
                sources,
                relation,
                detail,
            } => {
                canon::write_array_header(out, 4)?;
                canon::write_uint(out, 9)?;
                canon::write_array_header(out, sources.len())?;
                for source in sources {
                    push_bytes(out, source.as_bytes())?;
                }
                canon::write_uint(out, *relation)?;
                match detail {
                    Some(artifact) => push_bytes(out, artifact.as_bytes())?,
                    None => canon::write_null(out),
                }
                Ok(())
            }
        }
    }
}

/// Metadata artifact carrying operations and context assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataArtifact {
    /// Ordered operations; order is semantic.
    pub operations: Vec<Operation>,
    /// Bounded context assertions.
    pub context_assertions: Vec<Vec<u8>>,
}

impl MetadataArtifact {
    /// Encode the canonical artifact.
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] when a cap is exceeded.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.operations.len() > MAX_METADATA_OPERATIONS {
            return Err(Error::encoding("operations exceed the profile cap"));
        }
        if self.context_assertions.len() > MAX_CONTEXT_ASSERTIONS {
            return Err(Error::encoding("context assertions exceed the profile cap"));
        }
        let mut encoded = Vec::with_capacity(self.operations.len());
        for operation in &self.operations {
            encoded.push(operation.encode()?);
        }
        wrap(|out| {
            canon::write_array_header(out, 3)?;
            canon::write_uint(out, PROFILE_VERSION)?;
            canon::write_array_header(out, encoded.len())?;
            for operation in &encoded {
                out.extend_from_slice(operation);
            }
            canon::write_array_header(out, self.context_assertions.len())?;
            for assertion in &self.context_assertions {
                push_bytes(out, assertion)?;
            }
            Ok(())
        })
    }
}

// --------------------------------------------------------------------------
// Strict validation of encoded records
// --------------------------------------------------------------------------

/// Record names accepted by [`validate_record`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Record {
    /// §4.1 batch body.
    BatchBody,
    /// §4.2 batch signature.
    BatchSignature,
    /// §5 frontier.
    Frontier,
    /// §6.1 artifact manifest.
    ArtifactManifest,
    /// §6.3 native artifact.
    NativeArtifact,
    /// §7 metadata artifact.
    MetadataArtifact,
    /// §8.2 epoch descriptor.
    EpochDescriptor,
    /// §8.1 Git object.
    GitOid,
    /// §9 conflict.
    Conflict,
    /// §8.3 branch descriptor body.
    BranchDescriptor,
    /// §8.3 branch envelope.
    BranchEnvelope,
    /// §7.2 semantic manifest.
    SemanticManifest,
    /// §7.2 per-file entry.
    FileEntry,
}

impl Record {
    /// Map a fixture record name to its record kind.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "batch_body" => Self::BatchBody,
            "batch_signature" => Self::BatchSignature,
            "frontier" => Self::Frontier,
            "artifact_manifest" => Self::ArtifactManifest,
            "native_artifact" => Self::NativeArtifact,
            "metadata_artifact" => Self::MetadataArtifact,
            "epoch_descriptor" => Self::EpochDescriptor,
            "git_oid" => Self::GitOid,
            "conflict" => Self::Conflict,
            "branch_descriptor" => Self::BranchDescriptor,
            "branch_envelope" => Self::BranchEnvelope,
            "semantic_manifest" => Self::SemanticManifest,
            "file_entry" => Self::FileEntry,
            _ => return None,
        })
    }
}

fn as_list<'a>(item: &'a Item, length: usize, what: &str) -> Result<&'a [Item], Error> {
    match item {
        Item::Array(items) if items.len() == length => Ok(items),
        Item::Array(items) => Err(Error::encoding(format!(
            "{what} has {} elements, expected {length}",
            items.len()
        ))),
        _ => Err(Error::encoding(format!("{what} is not an array"))),
    }
}

fn as_array<'a>(item: &'a Item, what: &str) -> Result<&'a [Item], Error> {
    match item {
        Item::Array(items) => Ok(items),
        _ => Err(Error::encoding(format!("{what} is not an array"))),
    }
}

fn as_uint(item: &Item, what: &str, maximum: u64) -> Result<u64, Error> {
    match item {
        Item::Uint(value) if *value <= maximum => Ok(*value),
        Item::Uint(_) => Err(Error::encoding(format!("{what} exceeds the profile cap"))),
        _ => Err(Error::encoding(format!("{what} is not a uint"))),
    }
}

fn as_bytes<'a>(item: &'a Item, width: usize, what: &str) -> Result<&'a [u8], Error> {
    match item {
        Item::Bytes(bytes) if bytes.len() == width => Ok(bytes),
        Item::Bytes(_) => Err(Error::encoding(format!("{what} has wrong width"))),
        _ => Err(Error::encoding(format!("{what} is not a byte string"))),
    }
}

fn as_text<'a>(item: &'a Item, what: &str, maximum: usize) -> Result<&'a str, Error> {
    match item {
        Item::Text(text) if text.len() <= maximum => Ok(text),
        Item::Text(_) => Err(Error::encoding(format!("{what} exceeds the profile cap"))),
        _ => Err(Error::encoding(format!("{what} is not a text string"))),
    }
}

fn as_tag(item: &Item, allowed: &[u64], what: &str) -> Result<u64, Error> {
    let value = as_uint(item, what, u64::MAX)?;
    if allowed.contains(&value) {
        Ok(value)
    } else {
        Err(Error::Capability(format!("{what} is not an assigned tag")))
    }
}

fn check_version(item: &Item) -> Result<(), Error> {
    let version = as_uint(item, "profile version", u64::MAX)?;
    if version == PROFILE_VERSION {
        Ok(())
    } else {
        Err(Error::Version(
            "only profile version 1 is frozen".to_owned(),
        ))
    }
}

fn check_sorted_unique<T: Ord + Copy>(values: &[T], what: &str) -> Result<(), Error> {
    if values.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(Error::encoding(format!("{what} is not sorted and unique")));
    }
    Ok(())
}

fn bytes32_list(item: &Item, what: &str, cap: usize) -> Result<Vec<[u8; 32]>, Error> {
    let items = as_array(item, what)?;
    if items.len() > cap {
        return Err(Error::encoding(format!("{what} exceeds the profile cap")));
    }
    let mut values = Vec::with_capacity(items.len());
    for entry in items {
        let raw = as_bytes(entry, 32, what)?;
        let mut value = [0u8; 32];
        value.copy_from_slice(raw);
        values.push(value);
    }
    Ok(values)
}

fn validate_git_oid(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 3, "git_oid")?;
    check_version(&record[0])?;
    let algorithm = as_uint(&record[1], "git_oid algorithm", u64::MAX)?;
    let width = match algorithm {
        1 => 20,
        2 => 32,
        _ => return Err(Error::Capability("unknown Git OID algorithm".to_owned())),
    };
    as_bytes(&record[2], width, "git_oid digest")?;
    Ok(())
}

fn validate_epoch_descriptor(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 7, "epoch_descriptor")?;
    check_version(&record[0])?;
    as_bytes(&record[1], 16, "epoch file")?;
    as_bytes(&record[2], 16, "generation nonce")?;
    let kind = as_tag(&record[3], &EPOCH_KIND_TAGS, "epoch kind tag")?;
    as_bytes(&record[4], 32, "baseline artifact")?;
    match (kind, &record[5]) {
        (0, Item::Text(profile)) => {
            if profile != TEXT_PROFILE {
                return Err(Error::Capability(
                    "only dt-text-v1 is a frozen text profile".to_owned(),
                ));
            }
        }
        (0, _) => return Err(Error::encoding("text epochs require the DT seed profile")),
        (_, Item::Null) => {}
        (_, _) => {
            return Err(Error::encoding(
                "non-text epochs must not declare a native seed profile",
            ));
        }
    }
    if !matches!(record[6], Item::Null) {
        validate_git_oid(&record[6])?;
    }
    Ok(())
}

fn validate_operation(item: &Item) -> Result<(), Error> {
    let record = as_array(item, "operation")?;
    let Some(first) = record.first() else {
        return Err(Error::encoding("operation is not a non-empty array"));
    };
    let tag = as_tag(first, &OPERATION_TAGS, "operation tag")?;
    match tag {
        0 => {
            let fields = as_list(item, 6, "create operation")?;
            as_bytes(&fields[1], 16, "repository")?;
            as_bytes(&fields[2], 16, "file")?;
            Path::new(as_text(&fields[3], "path", MAX_PATH_BYTES)?)?;
            as_tag(&fields[4], &MODE_TAGS, "mode tag")?;
            validate_epoch_descriptor(&fields[5])
        }
        1 => {
            let fields = as_list(item, 3, "path operation")?;
            as_bytes(&fields[1], 16, "file")?;
            Path::new(as_text(&fields[2], "path", MAX_PATH_BYTES)?)?;
            Ok(())
        }
        2 => {
            let fields = as_list(item, 3, "liveness operation")?;
            as_bytes(&fields[1], 16, "file")?;
            as_tag(&fields[2], &LIVE_TAGS, "live tag")?;
            Ok(())
        }
        3 => {
            let fields = as_list(item, 3, "epoch operation")?;
            as_bytes(&fields[1], 16, "file")?;
            validate_epoch_descriptor(&fields[2])
        }
        4 => {
            let fields = as_list(item, 4, "opaque operation")?;
            as_bytes(&fields[1], 16, "file")?;
            as_bytes(&fields[2], 32, "opaque epoch")?;
            as_bytes(&fields[3], 32, "opaque artifact")?;
            Ok(())
        }
        5 => {
            let fields = as_list(item, 4, "context operation")?;
            as_tag(&fields[1], &CONTEXT_SCHEMA_TAGS, "context schema tag")?;
            as_bytes(&fields[2], 16, "event id")?;
            match &fields[3] {
                Item::Bytes(bytes) if bytes.len() <= MAX_CONTEXT_EVENT_BYTES => Ok(()),
                Item::Bytes(_) => Err(Error::encoding("context event exceeds the cap")),
                _ => Err(Error::encoding("context event is not bytes")),
            }
        }
        6 => {
            let fields = as_list(item, 5, "git association operation")?;
            as_bytes(&fields[1], 16, "repository")?;
            validate_git_oid(&fields[2])?;
            as_bytes(&fields[3], 32, "version")?;
            as_uint(&fields[4], "git relation tag", u64::MAX)?;
            Ok(())
        }
        7 => {
            let fields = as_list(item, 3, "mode operation")?;
            as_bytes(&fields[1], 16, "file")?;
            as_tag(&fields[2], &MODE_TAGS, "mode tag")?;
            Ok(())
        }
        8 => {
            let fields = as_list(item, 4, "resolution operation")?;
            as_bytes(&fields[1], 32, "conflict id")?;
            let resolution = as_tag(&fields[2], &RESOLUTION_TAGS, "resolution tag")?;
            match (resolution, &fields[3]) {
                (0 | 1, Item::Null) => Ok(()),
                (0 | 1, _) => Err(Error::encoding("this resolution tag takes a null payload")),
                (2, Item::Bytes(epoch)) if epoch.len() == 32 => Ok(()),
                (2, _) => Err(Error::encoding("retained epoch has wrong width")),
                (3, payload) => {
                    let nested = as_list(payload, 2, "incorporation payload")?;
                    as_bytes(&nested[0], 32, "incorporated epoch")?;
                    as_bytes(&nested[1], 32, "provenance artifact")?;
                    Ok(())
                }
                (_, payload) => {
                    let nested = as_list(payload, 2, "alternative payload")?;
                    as_tag(&nested[0], &FIELD_TAGS, "field tag")?;
                    let identity = as_list(&nested[1], 2, "write identity")?;
                    as_bytes(&identity[0], 32, "participant batch")?;
                    as_uint(&identity[1], "operation index", MAX_OPERATION_INDEX)?;
                    Ok(())
                }
            }
        }
        _ => {
            let fields = as_list(item, 4, "provenance operation")?;
            let sources = as_array(&fields[1], "source batches")?;
            for source in sources {
                as_bytes(source, 32, "source batch")?;
            }
            as_uint(&fields[2], "provenance relation tag", u64::MAX)?;
            if !matches!(fields[3], Item::Null) {
                as_bytes(&fields[3], 32, "detail artifact")?;
            }
            Ok(())
        }
    }
}

fn validate_metadata_artifact(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 3, "metadata_artifact")?;
    check_version(&record[0])?;
    let operations = as_array(&record[1], "operations")?;
    if operations.len() > MAX_METADATA_OPERATIONS {
        return Err(Error::encoding("operations exceed the profile cap"));
    }
    for operation in operations {
        validate_operation(operation)?;
    }
    let assertions = as_array(&record[2], "context assertions")?;
    if assertions.len() > MAX_CONTEXT_ASSERTIONS {
        return Err(Error::encoding("context assertions exceed the cap"));
    }
    for assertion in assertions {
        if !matches!(assertion, Item::Bytes(_)) {
            return Err(Error::encoding("context assertion is not bytes"));
        }
    }
    Ok(())
}

fn validate_batch_body(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 11, "batch_body")?;
    check_version(&record[0])?;
    as_bytes(&record[1], 16, "space")?;
    as_bytes(&record[2], 16, "branch")?;
    as_bytes(&record[3], 32, "public key")?;
    as_bytes(&record[4], 16, "writer epoch")?;
    let sequence = as_uint(&record[5], "sequence", u64::MAX)?;
    let has_previous = !matches!(record[6], Item::Null);
    if has_previous {
        as_bytes(&record[6], 32, "previous batch")?;
    }
    let parents = bytes32_list(&record[7], "parents", MAX_BATCH_PARENTS)?;
    check_sorted_unique(&parents, "parents")?;
    let lamport = as_uint(&record[8], "lamport", u64::MAX)?;
    if sequence < 1 {
        return Err(Error::encoding("sequences start at 1"));
    }
    if has_previous {
        if sequence < 2 {
            return Err(Error::encoding(
                "a batch with a previous batch has sequence >= 2",
            ));
        }
        if parents.is_empty() {
            return Err(Error::encoding(
                "previous must be inside the declared parent closure",
            ));
        }
    } else if sequence != 1 {
        return Err(Error::encoding("a stream's first batch has sequence 1"));
    }
    // A stream's first batch may declare the intent-base cut as parents; only a
    // batch with no parents at all is the space's first and has lamport 1.
    if parents.is_empty() {
        if lamport != 1 {
            return Err(Error::encoding("a batch without parents has lamport 1"));
        }
    } else if lamport < 2 {
        return Err(Error::encoding("a batch with parents has lamport >= 2"));
    }
    bytes32_list(&record[9], "native artifacts", MAX_BATCH_NATIVE_ARTIFACTS)?;
    as_bytes(&record[10], 32, "metadata artifact")?;
    Ok(())
}

fn validate_frontier(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 3, "frontier")?;
    check_version(&record[0])?;
    as_bytes(&record[1], 16, "space")?;
    let heads = bytes32_list(&record[2], "frontier heads", MAX_FRONTIER_HEADS)?;
    check_sorted_unique(&heads, "frontier heads")?;
    Ok(())
}

fn validate_artifact_manifest(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 5, "artifact_manifest")?;
    check_version(&record[0])?;
    let kind = as_text(&record[1], "artifact kind", MAX_KIND_TEXT_BYTES)?;
    if RESERVED_KINDS.contains(&kind) {
        return Err(Error::Capability(
            "reserved artifact kinds are not decodable here".to_owned(),
        ));
    }
    if !ASSIGNED_KINDS.contains(&kind) {
        return Err(Error::Capability("unassigned artifact kind".to_owned()));
    }
    let raw_length = as_uint(&record[2], "raw length", MAX_ARTIFACT_RAW_BYTES)?;
    if kind == "text-baseline-v1" && raw_length > MAX_TEXT_EPOCH_BYTES as u64 {
        return Err(Error::encoding("text baselines exceed the text-epoch cap"));
    }
    as_bytes(&record[3], 32, "raw digest")?;
    let chunks = as_array(&record[4], "chunk list")?;
    if chunks.len() > MAX_ARTIFACT_CHUNKS {
        return Err(Error::encoding("chunk list exceeds the profile cap"));
    }
    for chunk in chunks {
        as_bytes(chunk, 32, "chunk id")?;
    }
    let expected = raw_length.div_ceil(CHUNK_RAW_BYTES as u64);
    if chunks.len() as u64 != expected {
        // Code names a digest disagreement; only the chunk count is checked
        // here. Raw digest verification happens at hydration, not admission.
        return Err(Error::HashMismatch(
            "declared length disagrees with the chunk list".to_owned(),
        ));
    }
    Ok(())
}

fn validate_native_artifact(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 4, "native_artifact")?;
    check_version(&record[0])?;
    as_bytes(&record[1], 32, "epoch")?;
    let profile = as_text(&record[2], "native profile", MAX_KIND_TEXT_BYTES)?;
    if profile != TEXT_PROFILE {
        return Err(Error::Capability(
            "only the dt-text-v1 profile is frozen".to_owned(),
        ));
    }
    match &record[3] {
        Item::Bytes(bytes) if bytes.len() as u64 <= MAX_ARTIFACT_RAW_BYTES => Ok(()),
        Item::Bytes(_) => Err(Error::encoding("native bytes exceed the profile cap")),
        _ => Err(Error::encoding("native bytes is not a byte string")),
    }
}

fn validate_conflict(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 5, "conflict")?;
    check_version(&record[0])?;
    let kind = as_tag(&record[1], &CONFLICT_KIND_TAGS, "conflict kind tag")?;
    as_bytes(&record[2], 16, "conflict file")?;
    let epochs = bytes32_list(&record[3], "conflict epochs", usize::MAX)?;
    check_sorted_unique(&epochs, "conflict epochs")?;
    if kind == 0 && !epochs.is_empty() {
        return Err(Error::encoding("register conflicts concern no epoch"));
    }
    let participants = as_array(&record[4], "participants")?;
    let mut identities = Vec::with_capacity(participants.len());
    for participant in participants {
        let nested = as_list(participant, 2, "participant")?;
        let batch = as_bytes(&nested[0], 32, "participant batch")?;
        let index = as_uint(&nested[1], "operation index", MAX_OPERATION_INDEX)?;
        let mut key = [0u8; 32];
        key.copy_from_slice(batch);
        identities.push((key, index));
    }
    check_sorted_unique(&identities, "participants")?;
    Ok(())
}

fn validate_branch_descriptor(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 5, "branch_descriptor")?;
    check_version(&record[0])?;
    as_bytes(&record[1], 16, "space")?;
    as_bytes(&record[2], 16, "branch")?;
    as_bytes(&record[3], 32, "initial version")?;
    as_bytes(&record[4], 32, "creator key")?;
    Ok(())
}

fn validate_branch_envelope(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 3, "branch_envelope")?;
    check_version(&record[0])?;
    let Item::Bytes(body) = &record[1] else {
        return Err(Error::encoding("descriptor body is not bytes"));
    };
    let decoded = canon::decode(body)?;
    validate_branch_descriptor(&decoded)?;
    let signature = as_bytes(&record[2], 64, "signature")?;
    let fields = as_list(&decoded, 5, "branch_descriptor")?;
    let mut creator = [0u8; 32];
    creator.copy_from_slice(as_bytes(&fields[4], 32, "creator key")?);
    let mut sig = [0u8; 64];
    sig.copy_from_slice(signature);
    if verify_strict(&creator, &branch_signature_preimage(body), &sig) {
        Ok(())
    } else {
        Err(Error::Signature(
            "descriptor signature does not verify".to_owned(),
        ))
    }
}

fn validate_file_entry(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 8, "file_entry")?;
    check_version(&record[0])?;
    as_bytes(&record[1], 16, "file")?;
    for (index, allowed, what) in [
        (2usize, &LIVE_TAGS[..], "live tags"),
        (6, &MODE_TAGS[..], "mode tags"),
    ] {
        let values = as_array(&record[index], what)?;
        let mut tags = Vec::with_capacity(values.len());
        for value in values {
            tags.push(as_tag(value, allowed, what)?);
        }
        check_sorted_unique(&tags, what)?;
    }
    let paths = as_array(&record[3], "paths")?;
    let mut previous: Option<&str> = None;
    for path in paths {
        let text = as_text(path, "path", MAX_PATH_BYTES)?;
        Path::new(text)?;
        if previous.is_some_and(|last| last >= text) {
            return Err(Error::encoding("paths is not sorted and unique"));
        }
        previous = Some(text);
    }
    for (index, what) in [(4usize, "epochs"), (5, "contents"), (7, "conflicts")] {
        let values = bytes32_list(&record[index], what, usize::MAX)?;
        check_sorted_unique(&values, what)?;
    }
    Ok(())
}

fn validate_semantic_manifest(item: &Item) -> Result<(), Error> {
    let record = as_list(item, 3, "semantic_manifest")?;
    check_version(&record[0])?;
    as_bytes(&record[1], 16, "space")?;
    let entries = as_array(&record[2], "entries")?;
    let mut files: Vec<[u8; 32]> = Vec::with_capacity(entries.len());
    for entry in entries {
        validate_file_entry(entry)?;
        let fields = as_list(entry, 8, "file_entry")?;
        let mut file = [0u8; 32];
        file[..16].copy_from_slice(as_bytes(&fields[1], 16, "file")?);
        files.push(file);
    }
    check_sorted_unique(&files, "manifest entries")?;
    Ok(())
}

/// Validate one encoded record.
///
/// # Errors
/// Returns the profile error with the stable code a receiver must report.
/// [`Record::BatchSignature`] needs its preimage, so it requires
/// [`validate_batch_signature_record`] instead.
pub fn validate_record(record: Record, body: &[u8]) -> Result<(), Error> {
    if record == Record::BatchSignature {
        return Err(Error::encoding(
            "batch signature validation requires its preimage",
        ));
    }
    let item = canon::decode(body)?;
    match record {
        Record::BatchBody => validate_batch_body(&item),
        Record::Frontier => validate_frontier(&item),
        Record::ArtifactManifest => validate_artifact_manifest(&item),
        Record::NativeArtifact => validate_native_artifact(&item),
        Record::MetadataArtifact => validate_metadata_artifact(&item),
        Record::EpochDescriptor => validate_epoch_descriptor(&item),
        Record::GitOid => validate_git_oid(&item),
        Record::Conflict => validate_conflict(&item),
        Record::BranchDescriptor => validate_branch_descriptor(&item),
        Record::BranchEnvelope => validate_branch_envelope(&item),
        Record::SemanticManifest => validate_semantic_manifest(&item),
        Record::FileEntry => validate_file_entry(&item),
        Record::BatchSignature => unreachable!("handled above"),
    }
}

/// Validate a batch signature record against its declared preimage.
///
/// # Errors
/// Returns [`Error::Signature`] when verification fails.
pub fn validate_batch_signature_record(body: &[u8], preimage: &[u8]) -> Result<(), Error> {
    let item = canon::decode(body)?;
    let record = as_list(&item, 3, "batch_signature")?;
    check_version(&record[0])?;
    let key = as_bytes(&record[1], 32, "signing key")?;
    let signature = as_bytes(&record[2], 64, "signature")?;
    let mut public_key = [0u8; 32];
    public_key.copy_from_slice(key);
    let mut sig = [0u8; 64];
    sig.copy_from_slice(signature);
    if verify_strict(&public_key, preimage, &sig) {
        Ok(())
    } else {
        Err(Error::Signature(
            "signature does not verify for the declared preimage".to_owned(),
        ))
    }
}

// --------------------------------------------------------------------------
// Writer-stream rules
// --------------------------------------------------------------------------

/// Writer-stream failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreamError {
    /// The declared stream context changed without a fresh writer epoch.
    #[error("writer stream context changed")]
    ContextChanged,
    /// Sequence did not increase by exactly one.
    #[error("sequence is not previous + 1")]
    Sequence,
    /// `previous` does not name the preceding body.
    #[error("previous does not name the preceding batch")]
    Previous,
    /// Lamport did not advance.
    #[error("lamport did not advance")]
    Lamport,
    /// Two different batches claim one `(stream, sequence)`.
    #[error("writer equivocation")]
    Equivocation,
}

impl StreamError {
    /// Stable public code for this failure, matching §14.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Equivocation => "writer_equivocation",
            Self::ContextChanged | Self::Sequence | Self::Previous | Self::Lamport => {
                "invalid_encoding"
            }
        }
    }
}

fn same_stream(left: &BatchBody, right: &BatchBody) -> bool {
    left.space == right.space
        && left.branch == right.branch
        && left.public_key == right.public_key
        && left.writer_epoch == right.writer_epoch
}

/// Check that `next` continues the stream of `previous`.
///
/// # Errors
/// Returns [`StreamError`] for a changed context, a skipped sequence, a wrong
/// `previous`, or a lamport that did not advance. A missing `previous` is the
/// genesis case and is checked by [`validate_record`] instead.
pub fn validate_stream_step(previous: &BatchBody, next: &BatchBody) -> Result<(), StreamError> {
    if !same_stream(previous, next) {
        return Err(StreamError::ContextChanged);
    }
    let Some(expected_sequence) = previous.sequence.checked_add(1) else {
        return Err(StreamError::Sequence);
    };
    if next.sequence != expected_sequence {
        return Err(StreamError::Sequence);
    }
    let previous_id = previous.id().map_err(|_| StreamError::Sequence)?;
    if next.previous != Some(previous_id) {
        return Err(StreamError::Previous);
    }
    let Some(minimum_lamport) = previous.lamport.checked_add(1) else {
        return Err(StreamError::Lamport);
    };
    if next.lamport < minimum_lamport {
        return Err(StreamError::Lamport);
    }
    Ok(())
}

/// Detect two bodies claiming one stream position with different identities.
///
/// # Errors
/// Returns [`StreamError::Equivocation`] when the bodies are different batches
/// on the same stream and sequence. Both must be quarantined; neither wins.
pub fn detect_equivocation(left: &BatchBody, right: &BatchBody) -> Result<(), StreamError> {
    if !same_stream(left, right) || left.sequence != right.sequence {
        return Ok(());
    }
    let (Ok(left_id), Ok(right_id)) = (left.id(), right.id()) else {
        return Err(StreamError::Equivocation);
    };
    if left_id == right_id {
        Ok(())
    } else {
        Err(StreamError::Equivocation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (SpaceId, BranchId, FileId) {
        (
            SpaceId::from_array([1; 16]),
            BranchId::from_array([2; 16]),
            FileId::from_array([3; 16]),
        )
    }

    #[test]
    fn actor_names_are_forty_five_bytes_and_context_bound() {
        let (_, branch, file) = ids();
        let epoch = EpochId::from_array([7; 32]);
        let writer_epoch = WriterEpoch::from_array([8; 16]);
        let seed = seed_actor_name(&epoch);
        let writer = writer_actor_name(&[9; 32], &branch, &writer_epoch, &epoch);
        assert_eq!(seed.len(), 45);
        assert_eq!(writer.len(), 45);
        assert!(seed.starts_with("s:"));
        assert!(writer.starts_with("w:"));
        assert_ne!(seed[2..], writer[2..]);
        let other = writer_actor_name(
            &[9; 32],
            &branch,
            &writer_epoch,
            &EpochId::from_array([8; 32]),
        );
        assert_ne!(writer, other);
        let _ = file;
    }

    #[test]
    fn paths_reject_the_frozen_violations() {
        for value in [
            "",
            "/absolute",
            "a/../b",
            "a//b",
            "a/",
            "a/./b",
            "a/.git/config",
            "a/.puyu/state",
            "a/b\u{0}c",
        ] {
            assert!(Path::new(value).is_err(), "accepted {value}");
        }
        for value in [
            "docs/canonical.md",
            "A/b",
            "docs/é.md",
            ".github/workflows/ci.yml",
        ] {
            assert!(Path::new(value).is_ok(), "rejected {value}");
        }
        assert!(Path::new(&"a".repeat(MAX_PATH_COMPONENT_BYTES + 1)).is_err());
        assert!(Path::new(&"/".repeat(2)).is_err());
    }

    #[test]
    fn empty_seeds_are_epoch_bound() {
        let payload = |epoch: [u8; 32]| NativeArtifact {
            epoch: EpochId::from_array(epoch),
            profile: TEXT_PROFILE.to_owned(),
            native: Vec::new(),
        };
        let first = payload([1; 32]);
        let second = payload([2; 32]);
        let first_manifest =
            ArtifactManifest::for_payload("native-change-v1", &first.encode().unwrap()).unwrap();
        let second_manifest =
            ArtifactManifest::for_payload("native-change-v1", &second.encode().unwrap()).unwrap();
        assert_eq!(first_manifest.chunks.len(), 1);
        assert_ne!(first_manifest.id().unwrap(), second_manifest.id().unwrap());
    }

    #[test]
    fn stream_steps_advance_exactly_once() {
        let (space, branch, _) = ids();
        let genesis = BatchBody {
            space,
            branch,
            public_key: [4; 32],
            writer_epoch: WriterEpoch::from_array([5; 16]),
            sequence: 1,
            previous: None,
            parents: Vec::new(),
            lamport: 1,
            native_artifacts: Vec::new(),
            metadata: ArtifactId::from_array([6; 32]),
        };
        let mut next = genesis.clone();
        next.sequence = 2;
        next.lamport = 2;
        next.previous = Some(genesis.id().unwrap());
        next.parents = vec![genesis.id().unwrap()];
        assert!(validate_stream_step(&genesis, &next).is_ok());
        assert!(validate_record(Record::BatchBody, &next.encode().unwrap()).is_ok());

        let mut skipped = next.clone();
        skipped.sequence = 3;
        assert_eq!(
            validate_stream_step(&genesis, &skipped),
            Err(StreamError::Sequence)
        );

        let mut stale = next.clone();
        stale.lamport = 1;
        assert_eq!(
            validate_stream_step(&genesis, &stale),
            Err(StreamError::Lamport)
        );

        let mut forked = next.clone();
        forked.writer_epoch = WriterEpoch::from_array([9; 16]);
        assert_eq!(
            validate_stream_step(&genesis, &forked),
            Err(StreamError::ContextChanged)
        );

        let mut equivocation = next.clone();
        equivocation.metadata = ArtifactId::from_array([7; 32]);
        assert_eq!(
            detect_equivocation(&next, &equivocation),
            Err(StreamError::Equivocation)
        );
        assert!(detect_equivocation(&next, &next).is_ok());
    }

    #[test]
    fn semantic_manifest_sorts_entries_and_alternatives() {
        let (space, _, file) = ids();
        let entry = |file: FileId, path: &str| FileEntry {
            file,
            live: vec![LiveTag::Alive, LiveTag::Deleted],
            paths: vec![Path::new(path).unwrap()],
            epochs: Vec::new(),
            contents: Vec::new(),
            modes: vec![ModeTag::Executable, ModeTag::Regular],
            conflicts: Vec::new(),
        };
        let forward = SemanticManifest {
            space,
            entries: vec![
                entry(file, "b.md"),
                entry(FileId::from_array([0; 16]), "a.md"),
            ],
        };
        let reverse = SemanticManifest {
            space,
            entries: vec![
                entry(FileId::from_array([0; 16]), "a.md"),
                entry(file, "b.md"),
            ],
        };
        assert_eq!(forward.encode().unwrap(), reverse.encode().unwrap());
        assert_eq!(forward.id().unwrap(), reverse.id().unwrap());
        validate_record(Record::SemanticManifest, &forward.encode().unwrap()).unwrap();
    }

    #[test]
    fn signatures_bind_the_body_key_before_verification() {
        let seed = [11u8; 32];
        let (space, branch, _) = ids();
        let body = BatchBody {
            space,
            branch,
            public_key: public_key(&seed),
            writer_epoch: WriterEpoch::from_array([5; 16]),
            sequence: 1,
            previous: None,
            parents: Vec::new(),
            lamport: 1,
            native_artifacts: Vec::new(),
            metadata: ArtifactId::from_array([6; 32]),
        };
        let batch = body.id().expect("id");
        let signature = sign_batch(&seed, &space, &batch);
        assert_eq!(
            verify_batch(&body, &body.public_key, &signature).unwrap(),
            batch
        );

        let mut tampered = signature;
        tampered[0] ^= 1;
        assert!(verify_batch(&body, &body.public_key, &tampered).is_err());

        let mut non_canonical_s = signature;
        non_canonical_s[32..].copy_from_slice(&[0xff; 32]);
        assert!(verify_batch(&body, &body.public_key, &non_canonical_s).is_err());

        let other = sign_batch(&[12u8; 32], &space, &batch);
        let error = verify_batch(&body, &public_key(&[12u8; 32]), &other)
            .expect_err("signature key must match the body's writer key");
        assert_eq!(error.code(), "invalid_signature");
    }

    #[test]
    fn stream_steps_reject_overflow_instead_of_wrapping() {
        let (space, branch, _) = ids();
        let previous = BatchBody {
            space,
            branch,
            public_key: [4; 32],
            writer_epoch: WriterEpoch::from_array([5; 16]),
            sequence: u64::MAX,
            previous: None,
            parents: Vec::new(),
            lamport: u64::MAX,
            native_artifacts: Vec::new(),
            metadata: ArtifactId::from_array([6; 32]),
        };
        let mut next = previous.clone();
        next.sequence = 0;
        next.previous = Some(previous.id().expect("id"));
        next.parents = vec![previous.id().expect("id")];
        assert_eq!(
            validate_stream_step(&previous, &next),
            Err(StreamError::Sequence)
        );

        let mut sequence_only = previous.clone();
        sequence_only.sequence = u64::MAX - 1;
        sequence_only.lamport = 1;
        let mut wrapped = sequence_only.clone();
        wrapped.sequence = 0;
        wrapped.lamport = 2;
        wrapped.previous = Some(sequence_only.id().expect("id"));
        wrapped.parents = vec![sequence_only.id().expect("id")];
        assert_eq!(
            validate_stream_step(&sequence_only, &wrapped),
            Err(StreamError::Sequence)
        );

        let mut lamport_only = previous.clone();
        lamport_only.sequence = 1;
        lamport_only.lamport = u64::MAX;
        let mut next = lamport_only.clone();
        next.sequence = 2;
        next.lamport = 2;
        next.previous = Some(lamport_only.id().expect("id"));
        next.parents = vec![lamport_only.id().expect("id")];
        assert_eq!(
            validate_stream_step(&lamport_only, &next),
            Err(StreamError::Lamport)
        );
    }
}
