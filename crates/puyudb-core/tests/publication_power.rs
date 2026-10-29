//! Power-loss model for the publication ledger.
//!
//! The shim VFS discards bytes that never reached a successful sync, or leaves
//! a torn write, then refuses further writes. Reopen uses the normal VFS.

use std::sync::{Mutex, MutexGuard};

use puyudb_core::identity::{ArtifactId, BatchId, BranchId, SpaceId, WriterEpoch};
use puyudb_core::profile::{self, BatchBody, chunk_id};
use puyudb_core::publication::{Publication, Store};
use sqlite_fault::{self, install, reset};

/// The shim VFS keeps one process-wide fault switch and one snapshot per open
/// file, so two tests must never run at once.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|err| err.into_inner())
}

const SPACE: SpaceId = SpaceId::from_array([1; 16]);

fn dir(label: &str) -> std::path::PathBuf {
    // The clock alone is not unique across parallel tests on every platform,
    // so a counter keeps each directory distinct.
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "puyu-power-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("dir");
    path
}

fn signed<'a>(
    body: &'a BatchBody,
    chunks: &'a [puyudb_core::identity::ChunkId],
    signature: &'a [u8; 64],
) -> Publication<'a> {
    Publication {
        body,
        signature_key: &body.public_key,
        signature,
        chunks,
        peer: 1,
    }
}

struct Prepared {
    body: BatchBody,
    signature: [u8; 64],
    batch: BatchId,
}

fn prepared() -> Prepared {
    let body = genesis();
    let batch = body.id().expect("id");
    let signature = profile::sign_batch(&[3; 32], &body.space, &batch);
    Prepared {
        body,
        signature,
        batch,
    }
}

fn genesis() -> BatchBody {
    BatchBody {
        space: SpaceId::from_array([1; 16]),
        branch: BranchId::from_array([2; 16]),
        public_key: profile::public_key(&[3; 32]),
        writer_epoch: WriterEpoch::from_array([4; 16]),
        sequence: 1,
        previous: None,
        parents: Vec::new(),
        lamport: 1,
        native_artifacts: Vec::new(),
        metadata: ArtifactId::from_array([6; 32]),
    }
}

fn open_fault(path: &std::path::Path) -> Store {
    install();
    reset();
    Store::open_vfs(path, Some("puyu-fault"), &SPACE).expect("fault open")
}

#[test]
fn fault_vfs_can_publish_when_disarmed() {
    let _serial = serial();
    let path = dir("ok");
    let bytes = b"durable-chunk";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    let prepared = prepared();
    store.stage_chunk(id, bytes).expect("stage");
    store
        .publish(&signed(&prepared.body, &[id], &prepared.signature))
        .expect("publish");
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(store.is_published(&prepared.batch).expect("read"));
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn lost_sync_during_publish_leaves_no_publication() {
    let _serial = serial();
    let path = dir("sync");
    let bytes = b"staged-only";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    let prepared = prepared();
    store.stage_chunk(id, bytes).expect("stage");
    sqlite_fault::fail_next_sync();
    assert!(
        store
            .publish(&signed(&prepared.body, &[id], &prepared.signature))
            .is_err()
    );
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(!store.is_published(&prepared.batch).expect("read"));
    assert_eq!(store.staged_without_publication().expect("staged"), 1);
    store
        .publish(&signed(&prepared.body, &[id], &prepared.signature))
        .expect("retry");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn torn_write_during_publish_leaves_no_publication() {
    let _serial = serial();
    let path = dir("tear");
    let bytes = b"tear-chunk";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    let prepared = prepared();
    store.stage_chunk(id, bytes).expect("stage");
    sqlite_fault::tear_next_write();
    assert!(
        store
            .publish(&signed(&prepared.body, &[id], &prepared.signature))
            .is_err()
    );
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(!store.is_published(&prepared.batch).expect("read"));
    store
        .publish(&signed(&prepared.body, &[id], &prepared.signature))
        .expect("retry");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn acked_batch_survives_a_later_armed_fault() {
    let _serial = serial();
    let path = dir("acked");
    let bytes = b"acked-chunk";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    let prepared = prepared();
    store.stage_chunk(id, bytes).expect("stage");
    store
        .publish(&signed(&prepared.body, &[id], &prepared.signature))
        .expect("publish");
    // The ack already returned; a later fault must not undo it.
    sqlite_fault::fail_next_sync();
    assert!(store.stage_chunk(chunk_id(b"other"), b"other").is_err());
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(store.is_published(&prepared.batch).expect("read"));
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn reset_under_open_file_fails_sync_without_touching_the_file() {
    let _serial = serial();
    let path = dir("reset-open");
    let bytes = b"reset-chunk";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    let prepared = prepared();
    store.stage_chunk(id, bytes).expect("stage");
    store
        .publish(&signed(&prepared.body, &[id], &prepared.signature))
        .expect("publish");
    // Misuse the shim on purpose: reset() shrinks the snapshot vec while the
    // file is still open. The armed sync must fail without restoring an empty
    // snapshot (which would truncate the database).
    reset();
    sqlite_fault::fail_next_sync();
    let mut second = prepared.body.clone();
    second.sequence = 2;
    second.lamport = 2;
    second.previous = Some(prepared.batch);
    second.parents = vec![prepared.batch];
    second.metadata = ArtifactId::from_array([7; 32]);
    let second_sig = profile::sign_batch(&[3; 32], &second.space, &second.id().expect("id"));
    assert!(store.publish(&signed(&second, &[], &second_sig)).is_err());
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(store.is_published(&prepared.batch).expect("read"));
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn lost_sync_during_stage_does_not_ack_the_chunk() {
    let _serial = serial();
    let path = dir("stage");
    let bytes = b"not-durable";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    sqlite_fault::fail_next_sync();
    assert!(store.stage_chunk(id, bytes).is_err());
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert_eq!(store.staged_without_publication().expect("staged"), 0);
    let _ = std::fs::remove_dir_all(&path);
}
