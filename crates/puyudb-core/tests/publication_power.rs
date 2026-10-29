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

fn dir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "puyu-power-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
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
    Store::open_vfs(path, Some("puyu-fault")).expect("fault open")
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
    let store = Store::open(&path).expect("reopen");
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
    let store = Store::open(&path).expect("reopen");
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
    let store = Store::open(&path).expect("reopen");
    assert!(!store.is_published(&prepared.batch).expect("read"));
    store
        .publish(&signed(&prepared.body, &[id], &prepared.signature))
        .expect("retry");
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
    let store = Store::open(&path).expect("reopen");
    assert_eq!(store.staged_without_publication().expect("staged"), 0);
    let _ = std::fs::remove_dir_all(&path);
}
