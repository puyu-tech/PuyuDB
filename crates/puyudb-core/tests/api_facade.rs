//! I06 P1–P4 integration tests: the public facade (create/open/busy/§14
//! error codes), the keyed transaction interface with branch-frontier CAS,
//! the read API at explicit cuts, and owner-lock release on drop.

use std::path::PathBuf;

use puyudb_core::api::{Availability, Database, Error};
use puyudb_core::identity::{
    ArtifactId, BatchId, BranchId, ChunkId, FileId, GenerationNonce, RepositoryId, SpaceId,
    WriterEpoch,
};
use puyudb_core::profile::{
    self, ArtifactManifest, BatchBody, EpochDescriptor, EpochKind, MetadataArtifact, ModeTag,
    NativeArtifact, Operation, Path,
};
use puyudb_core::publication::Publication;
use puyudb_crdt::dt_text::TextDoc;

const SPACE: SpaceId = SpaceId::from_array([1; 16]);
const BRANCH: BranchId = BranchId::from_array([2; 16]);
const KEY_SEED: [u8; 32] = [3; 32];
const WRITER_EPOCH: WriterEpoch = WriterEpoch::from_array([4; 16]);

fn temp() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "puyu-i06-api-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp");
    path
}

/// A valid metadata artifact payload carrying `distinct` in one context
/// assertion, so fixture artifacts stay content-distinct.
fn metadata_payload(distinct: &[u8]) -> Vec<u8> {
    MetadataArtifact {
        operations: Vec::new(),
        context_assertions: vec![distinct.to_vec()],
    }
    .encode()
    .expect("metadata artifact")
}

/// Stage a metadata artifact payload through the facade.
fn staged(db: &Database, distinct: &[u8]) -> (ArtifactManifest, Vec<ChunkId>) {
    let payload = metadata_payload(distinct);
    let manifest = ArtifactManifest::for_payload("metadata-v1", &payload).expect("manifest");
    let chunks = payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(|c| {
            let id = profile::chunk_id(c);
            db.stage_chunk(id, c).expect("stage");
            id
        })
        .collect();
    (manifest, chunks)
}

fn root_body(metadata: ArtifactId) -> BatchBody {
    BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: 1,
        previous: None,
        parents: Vec::new(),
        lamport: 1,
        native_artifacts: Vec::new(),
        metadata,
    }
}

/// The next batch of the same stream over the single-parent frontier.
fn child_body(previous: &BatchBody, metadata: ArtifactId) -> BatchBody {
    let parent = previous.id().expect("id");
    BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: previous.sequence + 1,
        previous: Some(parent),
        parents: vec![parent],
        lamport: previous.lamport + 1,
        native_artifacts: Vec::new(),
        metadata,
    }
}

fn sign(body: &BatchBody) -> [u8; 64] {
    profile::sign_batch(&KEY_SEED, &body.space, &body.id().expect("id"))
}

fn publication<'a>(
    body: &'a BatchBody,
    manifest: &'a ArtifactManifest,
    chunks: &'a [ChunkId],
    signature: &'a [u8; 64],
) -> Publication<'a> {
    publication_multi(body, std::slice::from_ref(manifest), chunks, signature)
}

fn publication_multi<'a>(
    body: &'a BatchBody,
    artifacts: &'a [ArtifactManifest],
    chunks: &'a [ChunkId],
    signature: &'a [u8; 64],
) -> Publication<'a> {
    Publication {
        body,
        signature_key: &body.public_key,
        signature,
        artifacts,
        chunks,
        peer: 1,
    }
}

/// Frame an arbitrary payload as an artifact and stage its chunks through
/// the facade.
fn frame_payload(db: &Database, kind: &str, payload: &[u8]) -> (ArtifactManifest, Vec<ChunkId>) {
    let manifest = ArtifactManifest::for_payload(kind, payload).expect("manifest");
    let chunks = payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(|c| {
            let id = profile::chunk_id(c);
            db.stage_chunk(id, c).expect("stage");
            id
        })
        .collect();
    (manifest, chunks)
}

/// A metadata artifact payload carrying real operations plus a distinct
/// context assertion.
fn ops_payload(ops: Vec<Operation>, salt: u8) -> Vec<u8> {
    MetadataArtifact {
        operations: ops,
        context_assertions: vec![vec![salt]],
    }
    .encode()
    .expect("metadata artifact")
}

fn text_epoch_descriptor(baseline: ArtifactId) -> EpochDescriptor {
    EpochDescriptor {
        file: FileId::from_array([1; 16]),
        nonce: GenerationNonce::from_array([1; 16]),
        kind: EpochKind::Text,
        baseline,
        seed_profile: Some(profile::TEXT_PROFILE.to_owned()),
        git_oid: None,
    }
}

#[test]
fn create_open_roundtrip() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    drop(db);
    let db = Database::open_space(&dir, &SPACE).expect("open after close");
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn double_create_fails_precondition() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let err = Database::create_space(&dir, &SPACE).expect_err("second create while owned");
    assert_eq!(err.code(), "precondition_failed");
    drop(db);
    let err = Database::create_space(&dir, &SPACE).expect_err("create after close");
    assert_eq!(err.code(), "precondition_failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn open_missing_fails_precondition() {
    let dir = temp();
    let err = Database::open_space(&dir, &SPACE).expect_err("open missing");
    assert_eq!(err.code(), "precondition_failed");
    let err = Database::open_space(dir.join("no-such-dir"), &SPACE).expect_err("open missing dir");
    assert_eq!(err.code(), "precondition_failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn second_open_returns_busy_with_hint() {
    let dir = temp();
    let first = Database::create_space(&dir, &SPACE).expect("create");
    let err = Database::open_space(&dir, &SPACE).expect_err("second open while owned");
    match err {
        Error::Busy(busy) => {
            assert!(
                busy.hint().contains("owner.lock"),
                "hint names the owner lock: {}",
                busy.hint()
            );
        }
        other => panic!("expected Error::Busy, got {other:?}"),
    }
    drop(first);
    let _second = Database::open_space(&dir, &SPACE).expect("open after the owner released");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn transact_happy_path_returns_receipt_and_is_visible() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let (manifest, chunks) = staged(&db, b"one");
    let body = root_body(manifest.id().expect("artifact id"));
    let signature = sign(&body);
    let publication = publication(&body, &manifest, &chunks, &signature);
    let receipt = db
        .transact(b"caller", b"key-1", &[], &publication)
        .expect("transact");
    // The stored receipt is only returned for a visible (non-quarantined)
    // batch, so this proves the batch became visible.
    assert_eq!(
        db.receipt_for(b"caller", b"key-1").expect("receipt lookup"),
        Some(receipt)
    );
    // A retry with the same key and content returns the stored receipt even
    // with an empty frontier (the branch now has a head, so the CAS would
    // fail): idempotency runs before the CAS (§13.1).
    let retry = db
        .transact(b"caller", b"key-1", &[], &publication)
        .expect("idempotent retry");
    assert_eq!(retry, receipt);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cas_mismatch_fails_precondition_and_leaves_key_reusable() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let (manifest1, chunks1) = staged(&db, b"first");
    let body1 = root_body(manifest1.id().expect("artifact id"));
    let signature1 = sign(&body1);
    let publication1 = publication(&body1, &manifest1, &chunks1, &signature1);
    let first = db
        .transact(b"caller", b"key-1", &[], &publication1)
        .expect("first transact");

    let (manifest2, chunks2) = staged(&db, b"second");
    let body2 = child_body(&body1, manifest2.id().expect("artifact id"));
    let signature2 = sign(&body2);
    let publication2 = publication(&body2, &manifest2, &chunks2, &signature2);

    // The branch now has one visible head, so an empty expected frontier is
    // not satisfied: CAS mismatch, cancel-before-publish.
    let err = db
        .transact(b"caller", b"key-2", &[], &publication2)
        .expect_err("cas mismatch");
    assert_eq!(err.code(), "precondition_failed");
    assert!(
        db.receipt_for(b"caller", b"key-2")
            .expect("receipt lookup")
            .is_none(),
        "a failed CAS must not consume the key"
    );

    // The same key with the correct frontier commits.
    let second = db
        .transact(b"caller", b"key-2", &[first.batch_id], &publication2)
        .expect("transact with the correct frontier");
    assert_eq!(
        db.receipt_for(b"caller", b"key-2").expect("receipt lookup"),
        Some(second)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn idempotent_retry_after_reopen_returns_identical_receipt() {
    let dir = temp();
    let receipt = {
        let db = Database::create_space(&dir, &SPACE).expect("create");
        let (manifest, chunks) = staged(&db, b"payload");
        let body = root_body(manifest.id().expect("artifact id"));
        let signature = sign(&body);
        let publication = publication(&body, &manifest, &chunks, &signature);
        let receipt = db
            .transact(b"caller", b"key", &[], &publication)
            .expect("transact");
        drop(db);
        receipt
    };

    let db = Database::open_space(&dir, &SPACE).expect("reopen");
    // Unknown-after-commit: the stored receipt survives the reopen.
    assert_eq!(
        db.receipt_for(b"caller", b"key").expect("receipt lookup"),
        Some(receipt)
    );
    // Rebuild the byte-identical publication and retry with the same key and
    // an empty frontier: the stored receipt is returned before the CAS check,
    // so the retry never fails CAS on its own committed batch (§13.1).
    let (manifest, chunks) = staged(&db, b"payload");
    let body = root_body(manifest.id().expect("artifact id"));
    let signature = sign(&body);
    let publication = publication(&body, &manifest, &chunks, &signature);
    let retry = db
        .transact(b"caller", b"key", &[], &publication)
        .expect("retry after reopen");
    assert_eq!(retry, receipt);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn same_key_different_content_fails_precondition() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let (manifest1, chunks1) = staged(&db, b"content-a");
    let body1 = root_body(manifest1.id().expect("artifact id"));
    let signature1 = sign(&body1);
    let publication1 = publication(&body1, &manifest1, &chunks1, &signature1);
    let first = db
        .transact(b"caller", b"key", &[], &publication1)
        .expect("first transact");

    let (manifest2, chunks2) = staged(&db, b"content-b");
    let body2 = child_body(&body1, manifest2.id().expect("artifact id"));
    let signature2 = sign(&body2);
    let publication2 = publication(&body2, &manifest2, &chunks2, &signature2);
    let err = db
        .transact(b"caller", b"key", &[first.batch_id], &publication2)
        .expect_err("key reuse with different content");
    assert_eq!(err.code(), "precondition_failed");
    // The original receipt is untouched.
    assert_eq!(
        db.receipt_for(b"caller", b"key").expect("receipt lookup"),
        Some(first)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn error_code_strings_cover_the_twelve_stable_codes() {
    let cases: [(Error, &str); 12] = [
        (Error::InvalidEncoding("d".into()), "invalid_encoding"),
        (Error::UnsupportedVersion("d".into()), "unsupported_version"),
        (
            Error::UnsupportedCapability("d".into()),
            "unsupported_capability",
        ),
        (Error::HashMismatch("d".into()), "hash_mismatch"),
        (Error::InvalidSignature("d".into()), "invalid_signature"),
        (Error::WriterEquivocation("d".into()), "writer_equivocation"),
        (Error::MissingDependency("d".into()), "missing_dependency"),
        (
            Error::InvalidNativeScope("d".into()),
            "invalid_native_scope",
        ),
        (Error::InvalidOperation("d".into()), "invalid_operation"),
        (Error::PreconditionFailed("d".into()), "precondition_failed"),
        (Error::ContentUnavailable("d".into()), "content_unavailable"),
        (Error::Corruption("d".into()), "corruption"),
    ];
    for (err, code) in cases {
        assert_eq!(err.code(), code);
    }
    // Environmental failures are deliberately not §14 codes.
    assert_eq!(Error::Internal("d".into()).code(), "internal");
}

#[test]
fn owner_lock_is_released_on_drop() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    drop(db);
    // The OS lock went away with the handle: a second open succeeds (§8.1;
    // drop and crash release the flock the same way).
    let _db = Database::open_space(&dir, &SPACE).expect("open after drop");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn read_apis_roundtrip_on_a_text_epoch() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    assert!(!db.bootstrapped().expect("bootstrapped"));
    assert!(db.branch_frontier(&BRANCH).expect("frontier").is_empty());

    // Baseline artifact and a text epoch over it, staged through the facade.
    let (baseline_manifest, baseline_ids) = frame_payload(&db, "text-baseline-v1", b"hello");
    db.stage_artifact(&baseline_manifest, &baseline_ids)
        .expect("stage baseline");
    let epoch_desc = text_epoch_descriptor(baseline_manifest.id().expect("id"));
    let epoch = epoch_desc.id().expect("epoch id");
    let create = Operation::Create {
        repository: RepositoryId::from_array([9; 16]),
        file: FileId::from_array([1; 16]),
        path: Path::new("a.txt").expect("path"),
        mode: ModeTag::Regular,
        epoch: epoch_desc.clone(),
    };
    let (meta_manifest, meta_ids) =
        frame_payload(&db, "metadata-v1", &ops_payload(vec![create], 1));
    let meta_artifact = meta_manifest.id().expect("id");
    let body = root_body(meta_artifact);
    let signature = sign(&body);
    let receipt = db
        .transact(
            b"caller",
            b"root",
            &[],
            &publication(&body, &meta_manifest, &meta_ids, &signature),
        )
        .expect("transact root");
    let root = receipt.batch_id;

    // Register the epoch seed through the facade.
    let seed_name = profile::seed_actor_name(&epoch);
    let mut doc = TextDoc::empty();
    let agent = doc.agent_id(&seed_name);
    doc.add_insert(agent, &[], 0, "hello");
    let seed_bytes = doc.encode();
    let (seed_manifest, seed_ids) = frame_payload(
        &db,
        "native-change-v1",
        &NativeArtifact {
            epoch,
            profile: profile::TEXT_PROFILE.to_owned(),
            native: seed_bytes.clone(),
        }
        .encode()
        .expect("native artifact"),
    );
    db.register_seed(&epoch, &seed_manifest, &seed_ids)
        .expect("register seed");

    // Read APIs at the root cut.
    assert!(db.bootstrapped().expect("bootstrapped"));
    assert_eq!(db.branch_frontier(&BRANCH).expect("frontier"), vec![root]);
    assert!(db.is_published(&root).expect("is_published"));
    assert!(
        !db.is_published(&BatchId::from_array([0x77; 32]))
            .expect("is_published")
    );
    let files = db.files_at(&[root]).expect("files_at");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file, FileId::from_array([1; 16]));
    assert_eq!(files[0].paths, vec![Path::new("a.txt").expect("path")]);
    assert_eq!(
        db.paths_at(&[root]).expect("paths_at"),
        vec![(
            Path::new("a.txt").expect("path"),
            vec![FileId::from_array([1; 16])]
        )]
    );
    let epochs = db
        .epochs_at(&[root], FileId::from_array([1; 16]))
        .expect("epochs_at");
    assert_eq!(epochs.len(), 1);
    assert_eq!(epochs[0].0, epoch);
    assert!(db.conflicts_at(&[root]).expect("conflicts").is_empty());
    assert!(db.prefix_conflicts_at(&[root]).expect("prefix").is_empty());
    let manifest = db.semantic_manifest_at(&[root]).expect("manifest");
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].file, FileId::from_array([1; 16]));
    let _manifest_id = manifest.id().expect("manifest id");
    assert_eq!(db.text_at(&[root], &epoch).expect("text_at"), "hello");
    assert_eq!(
        db.artifact_availability(&meta_artifact)
            .expect("availability"),
        Some(Availability::Resident)
    );
    assert_eq!(
        db.receipt_for(b"caller", b"root").expect("receipt lookup"),
        Some(receipt)
    );

    // A native edit batch through transact; the text read reflects it.
    let mut full = TextDoc::load(&seed_bytes).expect("load seed");
    let seed_frontier = full.local_frontier();
    let writer_name = profile::writer_actor_name(
        &profile::public_key(&KEY_SEED),
        &BRANCH,
        &WRITER_EPOCH,
        &epoch,
    );
    let writer = full.agent_id(&writer_name);
    full.add_insert(writer, &seed_frontier, 5, " world");
    let change = full.encode_from(&seed_frontier);
    let (change_manifest, change_ids) = frame_payload(
        &db,
        "native-change-v1",
        &NativeArtifact {
            epoch,
            profile: profile::TEXT_PROFILE.to_owned(),
            native: change,
        }
        .encode()
        .expect("native artifact"),
    );
    let change_artifact_id = change_manifest.id().expect("id");
    let (meta2, meta2_ids) = frame_payload(&db, "metadata-v1", &ops_payload(Vec::new(), 2));
    let mut body2 = child_body(&body, meta2.id().expect("id"));
    body2.native_artifacts = vec![change_artifact_id];
    let signature2 = sign(&body2);
    let chunks2: Vec<ChunkId> = [change_ids, meta2_ids].concat();
    let artifacts2 = [change_manifest, meta2];
    let receipt2 = db
        .transact(
            b"caller",
            b"edit",
            &[root],
            &publication_multi(&body2, &artifacts2, &chunks2, &signature2),
        )
        .expect("transact edit");
    assert_eq!(
        db.branch_frontier(&BRANCH).expect("frontier"),
        vec![receipt2.batch_id]
    );
    assert_eq!(
        db.text_at(&[receipt2.batch_id], &epoch).expect("text_at"),
        "hello world"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn frontier_and_conflicts_at_concurrent_heads() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    // Root: create file 1 at "a.txt" with a text epoch (the baseline artifact
    // is not needed for projection-only reads).
    let epoch_desc = text_epoch_descriptor(ArtifactId::from_array([1; 32]));
    let create = Operation::Create {
        repository: RepositoryId::from_array([9; 16]),
        file: FileId::from_array([1; 16]),
        path: Path::new("a.txt").expect("path"),
        mode: ModeTag::Regular,
        epoch: epoch_desc,
    };
    let (meta, meta_ids) = frame_payload(&db, "metadata-v1", &ops_payload(vec![create], 1));
    let body = root_body(meta.id().expect("id"));
    let signature = sign(&body);
    let root = db
        .transact(
            b"caller",
            b"root",
            &[],
            &publication(&body, &meta, &meta_ids, &signature),
        )
        .expect("transact root")
        .batch_id;

    // Two causally concurrent children on fresh writer streams assigning
    // conflicting paths ("dir" vs "dir/file" — also a prefix conflict). Each
    // extends from the current frontier, so the CAS expectation moves with it.
    let mut expected = vec![root];
    let mut heads = Vec::new();
    for (seed, path, salt) in [([5u8; 32], "dir", 2u8), ([6u8; 32], "dir/file", 3u8)] {
        let (m, ids) = frame_payload(
            &db,
            "metadata-v1",
            &ops_payload(
                vec![Operation::Path {
                    file: FileId::from_array([1; 16]),
                    path: Path::new(path).expect("path"),
                }],
                salt,
            ),
        );
        let body = BatchBody {
            space: SPACE,
            branch: BRANCH,
            public_key: profile::public_key(&seed),
            writer_epoch: WRITER_EPOCH,
            sequence: 1,
            previous: None,
            parents: vec![root],
            lamport: 2,
            native_artifacts: Vec::new(),
            metadata: m.id().expect("id"),
        };
        let signature = profile::sign_batch(&seed, &body.space, &body.id().expect("id"));
        let receipt = db
            .transact(
                b"caller",
                &[salt],
                &expected,
                &publication(&body, &m, &ids, &signature),
            )
            .expect("concurrent transact");
        heads.push(receipt.batch_id);
        expected = vec![receipt.batch_id];
    }
    heads.sort_unstable();
    assert_eq!(db.branch_frontier(&BRANCH).expect("frontier"), heads);
    let conflicts = db.conflicts_at(&heads).expect("conflicts");
    assert!(
        conflicts
            .iter()
            .any(|c| c.kind == profile::ConflictKind::Register
                && c.file == FileId::from_array([1; 16])),
        "expected a register conflict on the concurrent path writes: {conflicts:?}"
    );
    let prefix = db.prefix_conflicts_at(&heads).expect("prefix");
    let dir_path = Path::new("dir").expect("path");
    let nested_path = Path::new("dir/file").expect("path");
    assert!(
        prefix.len() == 1
            && (prefix[0] == (dir_path.clone(), nested_path.clone())
                || prefix[0] == (nested_path.clone(), dir_path.clone())),
        "expected the dir / dir/file prefix conflict: {prefix:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_at_error_paths_map_to_the_real_stable_codes() {
    use puyudb_core::identity::EpochId;

    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    // Root creating file 1 with a declared text epoch (the seed is never
    // registered here).
    let epoch_desc = text_epoch_descriptor(ArtifactId::from_array([1; 32]));
    let epoch = epoch_desc.id().expect("epoch id");
    let create = Operation::Create {
        repository: RepositoryId::from_array([9; 16]),
        file: FileId::from_array([1; 16]),
        path: Path::new("a.txt").expect("path"),
        mode: ModeTag::Regular,
        epoch: epoch_desc,
    };
    let (meta, meta_ids) = frame_payload(&db, "metadata-v1", &ops_payload(vec![create], 1));
    let body = root_body(meta.id().expect("id"));
    let signature = sign(&body);
    let root = db
        .transact(
            b"caller",
            b"root",
            &[],
            &publication(&body, &meta, &meta_ids, &signature),
        )
        .expect("transact root")
        .batch_id;

    // Declared at the cut but no registered seed => missing_dependency.
    let err = db.text_at(&[root], &epoch).expect_err("no seed");
    assert_eq!(err.code(), "missing_dependency");
    // Not a declared text epoch at this cut => invalid_operation.
    let err = db
        .text_at(&[root], &EpochId::from_array([0x99; 32]))
        .expect_err("undeclared epoch");
    assert_eq!(err.code(), "invalid_operation");
    let _ = std::fs::remove_dir_all(&dir);
}

/// T12 offline embed acceptance: a full create/transact/read/reopen cycle in
/// a fresh temp dir, with no network, no account, and no daemon — plus an
/// honest structural pin that puyudb-core declares no async/network/daemon
/// dependency, so the embed path cannot open a socket.
#[test]
fn offline_embed_acceptance_no_network_no_account_no_daemon() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(manifest_path).expect("read core manifest");
    for dep in [
        "tokio",
        "hyper",
        "reqwest",
        "tonic",
        "async-std",
        "async_std",
        "socket2",
        "mio",
        "daemon",
    ] {
        assert!(
            !manifest.contains(dep),
            "puyudb-core must not declare a dependency containing {dep:?}"
        );
    }

    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let (meta, meta_ids) = frame_payload(&db, "metadata-v1", &ops_payload(Vec::new(), 42));
    let body = root_body(meta.id().expect("id"));
    let signature = sign(&body);
    let receipt = db
        .transact(
            b"caller",
            b"offline",
            &[],
            &publication(&body, &meta, &meta_ids, &signature),
        )
        .expect("transact offline");
    assert_eq!(
        db.branch_frontier(&BRANCH).expect("frontier"),
        vec![receipt.batch_id]
    );
    assert!(db.bootstrapped().expect("bootstrapped"));
    drop(db);
    let db = Database::open_space(&dir, &SPACE).expect("reopen offline");
    assert_eq!(
        db.receipt_for(b"caller", b"offline").expect("receipt"),
        Some(receipt)
    );
    assert!(db.is_published(&receipt.batch_id).expect("visible"));
    let _ = std::fs::remove_dir_all(&dir);
}
