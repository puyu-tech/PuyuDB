// SPDX-License-Identifier: Apache-2.0

//! T04 process-kill evidence through the public Database facade (I06 P5):
//! a killed owner releases the OS lock, an ack received before the kill
//! implies the batch is fully visible after reopen, and a retry after the
//! unknown outcome returns the same stored receipt (spec §13.1).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use puyudb_core::api::Database;
use puyudb_core::identity::{BranchId, ChunkId, SpaceId, WriterEpoch};
use puyudb_core::profile::{self, ArtifactManifest, BatchBody, MetadataArtifact};
use puyudb_core::publication::Publication;

const SPACE: SpaceId = SpaceId::from_array([1; 16]);
const BRANCH: BranchId = BranchId::from_array([2; 16]);
const KEY_SEED: [u8; 32] = [3; 32];
const WRITER_EPOCH: WriterEpoch = WriterEpoch::from_array([4; 16]);

fn child_dir() -> Option<PathBuf> {
    std::env::var_os("PUYU_API_CRASH").map(PathBuf::from)
}

fn temp() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "puyu-i06-crash-{}-{}",
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

/// Build (and stage through the facade) the root batch for `distinct`.
fn prepared(
    db: &Database,
    distinct: &[u8],
) -> (BatchBody, ArtifactManifest, Vec<ChunkId>, [u8; 64]) {
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
    let body = BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: 1,
        previous: None,
        parents: Vec::new(),
        lamport: 1,
        native_artifacts: Vec::new(),
        metadata: manifest.id().expect("id"),
    };
    let signature = profile::sign_batch(&KEY_SEED, &body.space, &body.id().expect("id"));
    (body, manifest, chunks, signature)
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

/// Spawn this test binary as a child that runs the `child` branch (selected
/// by the env var) and wait until it wrote its `ready` marker, then kill it.
fn spawn_and_kill(test_name: &str, dir: &std::path::Path) {
    let mut child = Command::new(std::env::current_exe().expect("exe"))
        .arg(test_name)
        .arg("--exact")
        .env("PUYU_API_CRASH", dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let ready = dir.join("ready");
    let started = std::time::Instant::now();
    while !ready.exists() {
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let output = child.wait_with_output().expect("wait");
            panic!(
                "child never became ready: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Kill before any publish: the owner lock must be released by the crash and
/// nothing may be visible; a keyed transact then commits and is durable.
#[test]
fn kill_before_publish_releases_lock_and_leaves_nothing_visible() {
    if let Some(dir) = child_dir() {
        let db = Database::create_space(&dir, &SPACE).expect("child create");
        let bytes = b"staged-before-kill";
        let id = profile::chunk_id(bytes);
        db.stage_chunk(id, bytes).expect("stage");
        std::fs::write(dir.join("ready"), b"1").expect("ready");
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    let dir = temp();
    spawn_and_kill(
        "kill_before_publish_releases_lock_and_leaves_nothing_visible",
        &dir,
    );

    // Crash released the OS owner lock (GATE B N2): a fresh open in this
    // process succeeds.
    let db = Database::open_space(&dir, &SPACE).expect("reopen after kill");
    assert!(!db.bootstrapped().expect("bootstrapped"));
    assert!(db.branch_frontier(&BRANCH).expect("frontier").is_empty());

    // Ack implies durable: transact through the facade, reopen, and the batch
    // is fully visible (receipt, frontier, is_published).
    let (body, manifest, chunks, signature) = prepared(&db, b"after-kill");
    let receipt = db
        .transact(
            b"caller",
            b"k1",
            &[],
            &publication(&body, &manifest, &chunks, &signature),
        )
        .expect("transact after kill");
    drop(db);
    let db = Database::open_space(&dir, &SPACE).expect("reopen");
    assert!(db.is_published(&receipt.batch_id).expect("is_published"));
    assert_eq!(
        db.branch_frontier(&BRANCH).expect("frontier"),
        vec![receipt.batch_id]
    );
    assert_eq!(
        db.receipt_for(b"caller", b"k1").expect("receipt"),
        Some(receipt)
    );
    // Unknown-after-commit: a retry with the same key and content returns the
    // SAME stored receipt, before any CAS check (§13.1).
    let (body2, manifest2, chunks2, signature2) = prepared(&db, b"after-kill");
    let retry = db
        .transact(
            b"caller",
            b"k1",
            &[],
            &publication(&body2, &manifest2, &chunks2, &signature2),
        )
        .expect("idempotent retry");
    assert_eq!(retry, receipt);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Kill after the child received its ack: the committed batch must still be
/// fully visible after reopen, with the same stored receipt.
#[test]
fn kill_after_ack_keeps_the_batch_visible_and_the_receipt_stable() {
    if let Some(dir) = child_dir() {
        let db = Database::create_space(&dir, &SPACE).expect("child create");
        let (body, manifest, chunks, signature) = prepared(&db, b"acked");
        db.transact(
            b"caller",
            b"k",
            &[],
            &publication(&body, &manifest, &chunks, &signature),
        )
        .expect("child transact acked");
        std::fs::write(dir.join("ready"), b"1").expect("ready");
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    let dir = temp();
    spawn_and_kill(
        "kill_after_ack_keeps_the_batch_visible_and_the_receipt_stable",
        &dir,
    );

    // The crash released the lock; the acked batch survived the kill.
    let db = Database::open_space(&dir, &SPACE).expect("reopen after kill");
    let receipt = db
        .receipt_for(b"caller", b"k")
        .expect("receipt")
        .expect("stored receipt after kill");
    assert!(db.is_published(&receipt.batch_id).expect("is_published"));
    assert_eq!(
        db.branch_frontier(&BRANCH).expect("frontier"),
        vec![receipt.batch_id]
    );
    // Retry after the unknown outcome: same key, same content, same receipt.
    let (body, manifest, chunks, signature) = prepared(&db, b"acked");
    let retry = db
        .transact(
            b"caller",
            b"k",
            &[],
            &publication(&body, &manifest, &chunks, &signature),
        )
        .expect("retry after kill");
    assert_eq!(retry, receipt);
    let _ = std::fs::remove_dir_all(&dir);
}
