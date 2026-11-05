//! I06 P1/P2 integration tests: the public facade (create/open/busy/§14
//! error codes) and the keyed transaction interface with branch-frontier CAS.

use std::path::PathBuf;

use puyudb_core::api::{Database, Error};
use puyudb_core::identity::{ArtifactId, BranchId, ChunkId, SpaceId, WriterEpoch};
use puyudb_core::profile::{self, ArtifactManifest, BatchBody, MetadataArtifact};
use puyudb_core::publication::Publication;

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
    Publication {
        body,
        signature_key: &body.public_key,
        signature,
        artifacts: std::slice::from_ref(manifest),
        chunks,
        peer: 1,
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
