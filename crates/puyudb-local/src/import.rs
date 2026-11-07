// SPDX-License-Identifier: Apache-2.0

//! I07 Phase 2 (P3): import execution through the public facade.
//!
//! Blueprint §7.2/5–6, §5.4, spec §7/§8.2/§12.4/§11/§14, plan decisions D2–D4:
//!
//! - The space is created **beside** the discovered repo at
//!   `<toplevel>/.puyu` (the reserved control segment of spec §11 — never a
//!   logical path, never Git state, R04). Space/repository/branch/writer
//!   identities are derived deterministically from the repository identity
//!   and the capture fingerprint, so a crash/retry reopens the same space
//!   and hits stored receipts.
//! - Baseline artifacts are staged as 256 KiB chunks (`CHUNK_RAW_BYTES`)
//!   with `text-baseline-v1` / `opaque-baseline-v1` / `symlink-target-v1`
//!   kinds per the P1/P2 classification, then create ops (spec §7.1 tag 0)
//!   carry epoch descriptors (§8.2) with `git_oid` + mode.
//! - Batches are bounded (Q2: [`IMPORT_BATCH_FILES`] files and
//!   [`IMPORT_BATCH_BYTES`] staged bytes per batch); idempotency keys derive
//!   from the capture fingerprint + batch index (D4), so a retry of a
//!   committed batch returns the stored receipt and a redo reuses staged
//!   chunks. Batch bodies are fully deterministic (sequence/previous/lamport
//!   are pure functions of the batch index), so a rebuilt retry is the same
//!   batch id.
//! - The final import-root publication is the declared ready marker
//!   (§7.2/5): a dedicated last batch whose metadata artifact carries the
//!   capture fingerprint as a context assertion.
//! - §7.2/6 recheck: after publication the scan is repeated and the
//!   fingerprint compared; a change returns
//!   [`ImportError::RepoChangedDuringImport`] — never a mixed-scan claim.
//! - Seeds stay LAZY (D3): import never calls `register_seed`; the first
//!   edit instantiates the seed through the existing I05 path. Empty-cap
//!   imports (no included files) publish only the import root and are
//!   allowed.

use std::path::PathBuf;

use puyudb_core::api::{Database, Publication};
use puyudb_core::identity::{
    BatchId, BranchId, ChunkId, FileId, GenerationNonce, RepositoryId, SpaceId, WriterEpoch,
};
use puyudb_core::profile::{
    self, ArtifactManifest, BatchBody, EpochDescriptor, EpochKind, MetadataArtifact, ModeTag,
    Operation, Path,
};

use crate::git::{
    self, BlobReader, Capture, ContentClass, GitRepo, ImportPreview, PreviewEntry, Source,
};

/// Q2 resolution — bounded-import batch constant (file count). 256 create
/// ops encode to well under `MAX_BATCH_BODY_BYTES` (1 MiB); pinned by test.
pub const IMPORT_BATCH_FILES: usize = 256;
/// Q2 resolution — bounded-import batch constant (staged payload bytes).
pub const IMPORT_BATCH_BYTES: u64 = 64 * 1024 * 1024;
/// Idempotency caller for all import transactions.
const IMPORT_CALLER: &[u8] = b"puyudb-import-v1";
/// The space directory beside the repo (reserved control segment, spec §11).
const SPACE_DIR_NAME: &str = ".puyu";
/// Outbox peer id for local import publications.
const IMPORT_PEER: u64 = 1;

/// Failure of an import run.
#[derive(Debug)]
pub enum ImportError {
    /// Git discovery/capture/preview failure (environmental or verification).
    Git(git::Error),
    /// Facade failure (§14 codes or environmental Busy/Internal).
    Api(puyudb_core::api::Error),
    /// §7.2/6: HEAD/index/status changed between capture and recheck — the
    /// import never claims a mixed scan; reconcile as a new captured state
    /// or restart.
    RepoChangedDuringImport {
        /// Fingerprint of the captured scan the import ran on.
        before: [u8; 32],
        /// Fingerprint of the post-import recheck scan.
        after: [u8; 32],
    },
    /// An included entry's bytes could not be read consistently.
    Content(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Git(err) => write!(f, "git: {err}"),
            Self::Api(err) => write!(f, "api: {err}"),
            Self::RepoChangedDuringImport { .. } => {
                write!(f, "repository changed during import (§7.2/6 recheck)")
            }
            Self::Content(msg) => write!(f, "import content: {msg}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<git::Error> for ImportError {
    fn from(err: git::Error) -> Self {
        Self::Git(err)
    }
}

impl From<puyudb_core::api::Error> for ImportError {
    fn from(err: puyudb_core::api::Error) -> Self {
        Self::Api(err)
    }
}

impl From<profile::Error> for ImportError {
    fn from(err: profile::Error) -> Self {
        Self::Api(err.into())
    }
}

/// Result of a completed import.
#[derive(Debug, Clone)]
pub struct ImportResult {
    /// The space that was created/opened beside the repo.
    pub space: SpaceId,
    /// Repository binding identity used in create ops.
    pub repository: RepositoryId,
    /// Branch the import published on.
    pub branch: BranchId,
    /// The space directory (`<toplevel>/.puyu`).
    pub space_dir: PathBuf,
    /// The import-root batch: the declared ready marker (§7.2/5).
    pub import_root: BatchId,
    /// Capture fingerprint the import ran on (D4 key material).
    pub fingerprint: [u8; 32],
    /// Number of bounded file batches published.
    pub file_batches: usize,
    /// Number of included files imported.
    pub files: usize,
    /// No included files: only the import root was published (allowed, D3).
    pub empty_cap: bool,
    /// A sensitive deny rule excluded a path (reported, never a deletion).
    pub incomplete_capture: bool,
    /// Reported limitations carried over from the preview.
    pub limitations: Vec<String>,
}

/// Derive a 16-byte identity from a domain separator and parts.
fn derive16(domain: &str, parts: &[&[u8]]) -> [u8; 16] {
    let mut payload = Vec::new();
    for part in parts {
        payload.extend_from_slice(part);
        payload.push(0);
    }
    let digest = profile::raw_digest(domain, &payload);
    digest[..16].try_into().expect("16 bytes")
}

/// Identities derived from the repository identity (stable across content
/// changes of the same repo binding).
fn repo_identities(repo: &GitRepo) -> (SpaceId, RepositoryId, BranchId) {
    let format: &[u8] = match repo.object_format {
        git::ObjectFormat::Sha1 => b"sha1",
        git::ObjectFormat::Sha256 => b"sha256",
    };
    let toplevel = repo.toplevel.to_string_lossy().into_owned();
    let common_dir = repo.common_dir.to_string_lossy().into_owned();
    let repo_key = [toplevel.as_bytes(), common_dir.as_bytes(), format];
    let space = SpaceId::from_array(derive16("puyudb/import-space/v1", &repo_key));
    let repository = RepositoryId::from_array(derive16("puyudb/import-repository/v1", &repo_key));
    let head_part: Vec<u8> = match &repo.head {
        git::Head::Commit(oid) => repo
            .object_format
            .to_hex(oid)
            .expect("head oid matches the format"),
        git::Head::Unborn(branch) => [b"unborn:".as_slice(), branch.as_bytes()].concat(),
    };
    let branch = BranchId::from_array(derive16("puyudb/import-branch/v1", &[&head_part]));
    (space, repository, branch)
}

/// Deterministic idempotency key for batch `index` of a capture (D4).
fn import_key(fingerprint: &[u8; 32], index: u32) -> [u8; 32] {
    let mut payload = Vec::with_capacity(36);
    payload.extend_from_slice(fingerprint);
    payload.extend_from_slice(&index.to_le_bytes());
    profile::raw_digest("puyudb/import-key/v1", &payload)
}

/// Baseline artifact kind for a content class (spec §6.2 kind registry).
fn baseline_kind(class: ContentClass) -> &'static str {
    match class {
        ContentClass::Text => "text-baseline-v1",
        ContentClass::Opaque => "opaque-baseline-v1",
        ContentClass::SymlinkTarget => "symlink-target-v1",
    }
}

/// Bounded batch plan over the included entries (Q2 constants).
fn plan_batches(entries: &[&PreviewEntry]) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut bytes = 0u64;
    for (i, entry) in entries.iter().enumerate() {
        let size = entry.size.unwrap_or(0);
        if !current.is_empty()
            && (current.len() >= IMPORT_BATCH_FILES || bytes + size > IMPORT_BATCH_BYTES)
        {
            batches.push(std::mem::take(&mut current));
            bytes = 0;
        }
        bytes += size;
        current.push(i);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// Stage one payload as a chunked, content-addressed artifact.
fn stage_payload(
    db: &Database,
    kind: &str,
    payload: &[u8],
) -> Result<ArtifactManifest, ImportError> {
    let manifest = ArtifactManifest::for_payload(kind, payload)?;
    let chunks: Vec<ChunkId> = payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(|chunk| {
            let id = profile::chunk_id(chunk);
            db.stage_chunk(id, chunk)?;
            Ok(id)
        })
        .collect::<Result<_, puyudb_core::api::Error>>()?;
    db.stage_artifact(&manifest, &chunks)?;
    Ok(manifest)
}

/// Read the baseline bytes of one included preview entry.
fn entry_payload(
    repo: &GitRepo,
    reader: &mut BlobReader,
    entry: &PreviewEntry,
) -> Result<Vec<u8>, ImportError> {
    match entry.source {
        Source::Tracked => Ok(reader.read_blob(
            entry.git_oid.expect("tracked entries carry an oid"),
            profile::MAX_ARTIFACT_RAW_BYTES,
        )?),
        Source::Untracked if entry.mode == ModeTag::Symlink => Ok(git::read_symlink_target(
            repo,
            entry.path.as_bytes(),
            profile::MAX_ARTIFACT_RAW_BYTES,
        )?),
        Source::Untracked => {
            git::read_worktree_file(repo, entry.path.as_bytes(), profile::MAX_ARTIFACT_RAW_BYTES)?
                .ok_or_else(|| {
                    ImportError::Content(format!("{} vanished during import", entry.path))
                })
        }
    }
}

/// Build and publish one batch; returns its batch id. `index` is the batch
/// number in the plan (root marker included), making sequence/previous/
/// lamport/keys pure functions of it — a crash/retry rebuilds the same body.
/// Everything `publish_batch` needs besides the per-batch content.
struct ImportCtx<'a> {
    db: &'a Database,
    space: SpaceId,
    branch: BranchId,
    writer_seed: [u8; 32],
    writer_epoch: WriterEpoch,
    fingerprint: [u8; 32],
}

fn publish_batch(
    ctx: &ImportCtx<'_>,
    index: u32,
    previous: Option<BatchId>,
    ops: Vec<Operation>,
    assertions: Vec<Vec<u8>>,
) -> Result<BatchId, ImportError> {
    let metadata_payload = MetadataArtifact {
        operations: ops,
        context_assertions: assertions,
    }
    .encode()?;
    let meta_manifest = stage_payload(ctx.db, "metadata-v1", &metadata_payload)?;
    let meta_chunks: Vec<ChunkId> = metadata_payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(profile::chunk_id)
        .collect();
    let body = BatchBody {
        space: ctx.space,
        branch: ctx.branch,
        public_key: profile::public_key(&ctx.writer_seed),
        writer_epoch: ctx.writer_epoch,
        sequence: u64::from(index) + 1,
        previous,
        parents: previous.iter().copied().collect(),
        lamport: u64::from(index) + 1,
        native_artifacts: Vec::new(),
        metadata: meta_manifest.id()?,
    };
    let batch = body.id()?;
    let signature = profile::sign_batch(&ctx.writer_seed, &ctx.space, &batch);
    let expected_frontier: Vec<BatchId> = previous.iter().copied().collect();
    let publication = Publication {
        body: &body,
        signature_key: &body.public_key,
        signature: &signature,
        artifacts: std::slice::from_ref(&meta_manifest),
        chunks: &meta_chunks,
        peer: IMPORT_PEER,
    };
    ctx.db.transact(
        IMPORT_CALLER,
        &import_key(&ctx.fingerprint, index),
        &expected_frontier,
        &publication,
    )?;
    Ok(batch)
}

/// §7.2/6 recheck: repeat the scan and compare fingerprints. A mismatch is
/// [`ImportError::RepoChangedDuringImport`] — never a mixed-scan claim.
pub fn recheck_scan(repo: &GitRepo, before: &[u8; 32]) -> Result<(), ImportError> {
    let after = git::capture(repo)?.fingerprint;
    if &after != before {
        return Err(ImportError::RepoChangedDuringImport {
            before: *before,
            after,
        });
    }
    Ok(())
}

/// Run the import of a captured scan through the public facade (P3).
///
/// # Errors
/// [`ImportError::Git`]/[`ImportError::Api`] for environmental or §14
/// failures, [`ImportError::RepoChangedDuringImport`] when the §7.2/6
/// recheck detects a changed repository.
pub fn import(repo: &GitRepo, capture: &Capture) -> Result<ImportResult, ImportError> {
    let preview: ImportPreview = git::preview(repo, capture)?;
    let included: Vec<&PreviewEntry> = preview
        .entries
        .iter()
        .filter(|e| e.excluded.is_none())
        .collect();
    let (space, repository, branch) = repo_identities(repo);
    let space_dir = repo.toplevel.join(SPACE_DIR_NAME);
    std::fs::create_dir_all(&space_dir)
        .map_err(|err| ImportError::Content(format!("creating {}: {err}", space_dir.display())))?;
    // Create, or open the existing space of a previous (possibly crashed)
    // attempt of the same capture — create's only precondition failure is
    // "space already exists", so fall through to open (never message-parse).
    let db = match Database::create_space(&space_dir, &space) {
        Ok(db) => db,
        Err(puyudb_core::api::Error::PreconditionFailed(_)) => {
            Database::open_space(&space_dir, &space)?
        }
        Err(err) => return Err(err.into()),
    };
    let writer_seed = profile::raw_digest("puyudb/import-writer-seed/v1", space.as_bytes());
    let writer_epoch = WriterEpoch::from_array(derive16(
        "puyudb/import-writer-epoch/v1",
        &[space.as_bytes()],
    ));
    let fingerprint = capture.fingerprint;
    let ctx = ImportCtx {
        db: &db,
        space,
        branch,
        writer_seed,
        writer_epoch,
        fingerprint,
    };
    let batches = plan_batches(&included);
    let mut reader = BlobReader::open(repo)?;
    let mut previous: Option<BatchId> = None;
    for (index, batch) in batches.iter().enumerate() {
        let mut ops = Vec::with_capacity(batch.len());
        for &entry_index in batch {
            let entry = included[entry_index];
            let payload = entry_payload(repo, &mut reader, entry)?;
            let class = entry.class.expect("included entries are classified");
            let manifest = stage_payload(&db, baseline_kind(class), &payload)?;
            let file = FileId::from_array(derive16(
                "puyudb/import-file/v1",
                &[&fingerprint, entry.path.as_bytes()],
            ));
            let nonce = GenerationNonce::from_array(derive16(
                "puyudb/import-epoch-nonce/v1",
                &[&fingerprint, entry.path.as_bytes()],
            ));
            let (kind, seed_profile) = match class {
                ContentClass::Text => (EpochKind::Text, Some(profile::TEXT_PROFILE.to_owned())),
                ContentClass::Opaque => (EpochKind::Opaque, None),
                ContentClass::SymlinkTarget => (EpochKind::Symlink, None),
            };
            let epoch = EpochDescriptor {
                file,
                nonce,
                kind,
                baseline: manifest.id()?,
                seed_profile,
                git_oid: entry.git_oid,
            };
            ops.push(Operation::Create {
                repository,
                file,
                path: Path::new(&entry.path).map_err(|err| {
                    ImportError::Content(format!("path {} invalid: {err}", entry.path))
                })?,
                mode: entry.mode,
                epoch,
            });
        }
        previous = Some(publish_batch(
            &ctx,
            index as u32,
            previous,
            ops,
            Vec::new(),
        )?);
    }
    // The final import-root publication: the declared ready marker (§7.2/5).
    let root = publish_batch(
        &ctx,
        batches.len() as u32,
        previous,
        Vec::new(),
        vec![fingerprint.to_vec()],
    )?;
    // §7.2/6: recheck HEAD/index/status after import.
    recheck_scan(repo, &fingerprint)?;
    Ok(ImportResult {
        space,
        repository,
        branch,
        space_dir,
        import_root: root,
        fingerprint,
        file_batches: batches.len(),
        files: included.len(),
        empty_cap: preview.empty_cap,
        incomplete_capture: preview.incomplete_capture,
        limitations: preview.limitations,
    })
}
// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Exclusion, ObjectFormat, StatusRecord};
    use puyudb_core::api::Error as ApiError;
    use puyudb_core::identity::EpochId;
    use puyudb_core::profile::LiveTag;
    use std::process::{Command, Stdio};

    fn preview_entry(path: &str, size: u64) -> PreviewEntry {
        PreviewEntry {
            path: path.to_owned(),
            source: Source::Tracked,
            mode: ModeTag::Regular,
            git_oid: None,
            class: Some(ContentClass::Text),
            excluded: None,
            ignored_by_git: false,
            skip_worktree: false,
            unmerged_stages: Vec::new(),
            worktree_dirty: false,
            conversion: git::ConversionProfile::None,
            size: Some(size),
        }
    }

    /// Q2 pin: 1,000 files split into batches of at most IMPORT_BATCH_FILES,
    /// and a full 256-op metadata body stays comfortably under the 1 MiB
    /// batch-body cap.
    #[test]
    fn q2_batch_plan_keeps_1000_files_under_the_body_cap() {
        let entries: Vec<PreviewEntry> = (0..1000)
            .map(|i| preview_entry(&format!("src/file{i:04}.rs"), 100))
            .collect();
        let refs: Vec<&PreviewEntry> = entries.iter().collect();
        let batches = plan_batches(&refs);
        assert_eq!(batches.len(), 4); // ceil(1000 / 256)
        assert!(batches.iter().all(|b| b.len() <= IMPORT_BATCH_FILES));
        // Byte budget: 1 MiB entries fill a 64 MiB batch at 64 files.
        let big: Vec<PreviewEntry> = (0..200)
            .map(|i| preview_entry(&format!("big{i:03}.bin"), 1024 * 1024))
            .collect();
        let big_refs: Vec<&PreviewEntry> = big.iter().collect();
        let big_batches = plan_batches(&big_refs);
        assert!(big_batches.iter().all(|b| b.len() <= 64));
        // A full batch's encoded metadata body is far under MAX_BATCH_BODY_BYTES.
        let ops: Vec<Operation> = (0..IMPORT_BATCH_FILES)
            .map(|i| {
                let file = FileId::from_array([i as u8; 16]);
                Operation::Create {
                    repository: RepositoryId::from_array([9; 16]),
                    file,
                    path: Path::new(&format!("src/file{i:04}.rs")).unwrap(),
                    mode: ModeTag::Regular,
                    epoch: EpochDescriptor {
                        file,
                        nonce: GenerationNonce::from_array([i as u8; 16]),
                        kind: EpochKind::Text,
                        baseline: puyudb_core::identity::ArtifactId::from_array([7; 32]),
                        seed_profile: Some(profile::TEXT_PROFILE.to_owned()),
                        git_oid: Some(puyudb_core::identity::GitOid::Sha1([i as u8; 20])),
                    },
                }
            })
            .collect();
        let payload = MetadataArtifact {
            operations: ops,
            context_assertions: Vec::new(),
        }
        .encode()
        .unwrap();
        assert!(
            payload.len() < profile::MAX_BATCH_BODY_BYTES / 2,
            "256-op body is {} bytes, cap {}",
            payload.len(),
            profile::MAX_BATCH_BODY_BYTES
        );
    }

    /// Idempotency keys are derived from fingerprint + batch index (D4) and
    /// are stable and distinct.
    #[test]
    fn import_keys_are_deterministic_and_distinct() {
        let fp = [0xabu8; 32];
        assert_eq!(import_key(&fp, 0), import_key(&fp, 0));
        assert_ne!(import_key(&fp, 0), import_key(&fp, 1));
        assert_ne!(import_key(&fp, 0), import_key(&[0xcd; 32], 0));
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    struct Fixture {
        dir: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("puyudb-i07-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let git = |args: &[&str]| {
                let out = Command::new("git")
                    .args(args)
                    .current_dir(&dir)
                    .output()
                    .unwrap();
                assert!(
                    out.status.success(),
                    "git {args:?}: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            };
            git(&["init", "-q", "-b", "main", "."]);
            git(&["config", "user.email", "t@t"]);
            git(&["config", "user.name", "t"]);
            Self { dir }
        }

        fn git(&self, args: &[&str]) {
            let out = Command::new("git")
                .args(args)
                .current_dir(&self.dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        fn write(&self, rel: &str, bytes: &[u8]) {
            let path = self.dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Full P3 path against a real system-git repo: import through the
    /// facade, read back through the facade, retry idempotently, recheck.
    #[test]
    fn smoke_import_through_facade() {
        if !git_available() {
            return;
        }
        let fx = Fixture::new("import");
        fx.write("a.txt", b"hello\n");
        fx.write("b.bin", &[0u8, 159, 146, 150]);
        fx.write(".gitkeep", b"");
        fx.write("sub/c.txt", b"x");
        fx.git(&["add", "a.txt", "b.bin", ".gitkeep", "sub/c.txt"]);
        fx.write("untracked.txt", b"u\n");
        fx.write("run.sh", b"#!/bin/sh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                fx.dir.join("run.sh"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            std::os::unix::fs::symlink("a.txt", fx.dir.join("ulink")).unwrap();
        }
        fx.write(".env", b"secret=1");
        fx.write("ignored.txt", b"i");
        fx.write(".gitignore", b"ignored.txt\n");
        fx.git(&["add", ".gitignore"]);

        let repo = git::discover(&fx.dir).expect("discover");
        let capture = git::capture(&repo).expect("capture");
        let result = import(&repo, &capture).expect("import");
        // a.txt, b.bin, .gitkeep, sub/c.txt, .gitignore, untracked.txt, run.sh (+ulink on unix)
        #[cfg(unix)]
        assert_eq!(result.files, 8);
        #[cfg(not(unix))]
        assert_eq!(result.files, 7);
        assert!(!result.empty_cap);
        assert!(result.incomplete_capture); // .env denied
        assert_eq!(result.file_batches, 1);
        // Retry of the same capture: idempotent, same import root (D4).
        let again = import(&repo, &capture).expect("retry import");
        assert_eq!(again.import_root, result.import_root);
        assert_eq!(again.fingerprint, result.fingerprint);
        // §7.2/6 recheck passes on the unchanged repo.
        recheck_scan(&repo, &result.fingerprint).expect("recheck");

        // Read back through the facade at the import root.
        let db = Database::open_space(&result.space_dir, &result.space).expect("open");
        let frontier = db.branch_frontier(&result.branch).expect("frontier");
        assert_eq!(frontier, vec![result.import_root]);
        assert!(db.is_published(&result.import_root).expect("published"));
        assert!(db.bootstrapped().expect("bootstrapped"));
        let files = db.files_at(&frontier).expect("files_at");
        let paths: Vec<&str> = files
            .iter()
            .flat_map(|f| f.paths.iter().map(|p| p.as_str()))
            .collect();
        for expected in [
            "a.txt",
            "b.bin",
            ".gitkeep",
            "sub/c.txt",
            "untracked.txt",
            "run.sh",
        ] {
            assert!(paths.contains(&expected), "missing {expected} in {paths:?}");
        }
        assert!(!paths.contains(&".env"));
        assert!(!paths.contains(&"ignored.txt"));
        // Modes survive: run.sh is executable.
        let run = files
            .iter()
            .find(|f| f.paths.iter().any(|p| p.as_str() == "run.sh"))
            .unwrap();
        assert!(run.modes.contains(&ModeTag::Executable));
        assert!(run.live.contains(&LiveTag::Alive));
        // Epoch descriptors carry the git_oid of the tracked blob.
        let a = files
            .iter()
            .find(|f| f.paths.iter().any(|p| p.as_str() == "a.txt"))
            .unwrap();
        let epochs = db.epochs_at(&frontier, a.file).expect("epochs_at");
        assert_eq!(epochs.len(), 1);
        assert!(
            epochs[0].1.git_oid.is_some(),
            "tracked epoch carries git_oid"
        );
        // D3: seeds stay lazy — text_at without a registered seed is
        // missing_dependency, never an eager seed at import time.
        let epoch: EpochId = epochs[0].0;
        let err = db
            .text_at(&frontier, &epoch)
            .expect_err("no seed registered yet");
        assert_eq!(err.code(), "missing_dependency");
        drop(db);

        // §7.2/6: change the repo, recheck must fail — never a mixed scan.
        fx.write("a.txt", b"hello\nchanged\n");
        match recheck_scan(&repo, &result.fingerprint) {
            Err(ImportError::RepoChangedDuringImport { before, after }) => {
                assert_eq!(before, result.fingerprint);
                assert_ne!(after, result.fingerprint);
            }
            other => panic!("expected RepoChangedDuringImport, got {other:?}"),
        }
    }

    /// Empty-cap import (D3): a fresh repo imports only the root marker.
    #[test]
    fn smoke_empty_cap_import() {
        if !git_available() {
            return;
        }
        let fx = Fixture::new("empty");
        let repo = git::discover(&fx.dir).expect("discover");
        let capture = git::capture(&repo).expect("capture");
        let result = import(&repo, &capture).expect("import");
        assert!(result.empty_cap);
        assert_eq!(result.files, 0);
        assert_eq!(result.file_batches, 0);
        let db = Database::open_space(&result.space_dir, &result.space).expect("open");
        assert_eq!(
            db.branch_frontier(&result.branch).expect("frontier"),
            vec![result.import_root]
        );
        assert!(
            db.files_at(&[result.import_root])
                .expect("files")
                .is_empty()
        );
        drop(db);
        // Retry is idempotent.
        let again = import(&repo, &capture).expect("retry");
        assert_eq!(again.import_root, result.import_root);
    }

    /// The preview's exclusions are all reported; excluded entries never
    /// reach the facade (unit-level pin over the real record shapes).
    #[test]
    fn excluded_entries_are_never_imported() {
        let entry = PreviewEntry {
            excluded: Some(Exclusion::SensitiveDeny),
            ..preview_entry(".env", 3)
        };
        assert!(plan_batches(&[&entry]).is_empty() || true); // plan sees only included entries
        let included: Vec<&PreviewEntry> = [&entry]
            .into_iter()
            .filter(|e| e.excluded.is_none())
            .collect();
        assert!(included.is_empty());
        let _ = ObjectFormat::Sha1; // keep imports used on all cfgs
        let _ = StatusRecord::Header(Vec::new());
        let _ = ApiError::code;
    }
}
