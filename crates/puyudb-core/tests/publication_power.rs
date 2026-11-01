//! Power-loss model for the publication ledger.
//!
//! The shim VFS discards bytes that never reached a successful sync, or leaves
//! a torn write, then refuses further writes. Reopen uses the normal VFS.

use std::sync::{Mutex, MutexGuard};

use puyudb_core::identity::{BatchId, BranchId, ChunkId, SpaceId, WriterEpoch};
use puyudb_core::profile::{self, ArtifactManifest, BatchBody, chunk_id};
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

struct Prepared {
    body: BatchBody,
    signature: [u8; 64],
    batch: BatchId,
    manifest: ArtifactManifest,
    payload: Vec<u8>,
}

fn prepared_with(payload: &[u8]) -> Prepared {
    let manifest = ArtifactManifest::for_payload("metadata-v1", payload).expect("manifest");
    let body = BatchBody {
        space: SPACE,
        branch: BranchId::from_array([2; 16]),
        public_key: profile::public_key(&[3; 32]),
        writer_epoch: WriterEpoch::from_array([4; 16]),
        sequence: 1,
        previous: None,
        parents: Vec::new(),
        lamport: 1,
        native_artifacts: Vec::new(),
        metadata: manifest.id().expect("artifact id"),
    };
    let batch = body.id().expect("id");
    let signature = profile::sign_batch(&[3; 32], &body.space, &batch);
    Prepared {
        body,
        signature,
        batch,
        manifest,
        payload: payload.to_vec(),
    }
}

fn prepared() -> Prepared {
    prepared_with(b"fault-metadata")
}

/// A child of `parent` carrying its own metadata artifact payload.
fn prepared_child(parent: &Prepared, payload: &[u8]) -> Prepared {
    let mut p = prepared_with(payload);
    p.body.sequence = parent.body.sequence + 1;
    p.body.lamport = parent.body.lamport + 1;
    p.body.previous = Some(parent.batch);
    p.body.parents = vec![parent.batch];
    p.batch = p.body.id().expect("id");
    p.signature = profile::sign_batch(&[3; 32], &p.body.space, &p.batch);
    p
}

/// Stage the prepared artifact's chunks and return their ids in order.
fn stage_prepared(store: &Store, p: &Prepared) -> Vec<ChunkId> {
    p.payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(|c| {
            let id = chunk_id(c);
            store.stage_chunk(id, c).expect("stage");
            id
        })
        .collect()
}

fn signed<'a>(p: &'a Prepared, ids: &'a [ChunkId], sig: &'a [u8; 64]) -> Publication<'a> {
    Publication {
        body: &p.body,
        signature_key: &p.body.public_key,
        signature: sig,
        artifacts: std::slice::from_ref(&p.manifest),
        chunks: ids,
        peer: 1,
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
    let store = open_fault(&path);
    let prepared = prepared();
    let ids = stage_prepared(&store, &prepared);
    store
        .publish(&signed(&prepared, &ids, &prepared.signature))
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
    let store = open_fault(&path);
    let prepared = prepared();
    let ids = stage_prepared(&store, &prepared);
    sqlite_fault::fail_next_sync();
    assert!(
        store
            .publish(&signed(&prepared, &ids, &prepared.signature))
            .is_err()
    );
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(!store.is_published(&prepared.batch).expect("read"));
    assert_eq!(store.staged_without_publication().expect("staged"), 1);
    store
        .publish(&signed(&prepared, &ids, &prepared.signature))
        .expect("retry");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn torn_write_during_publish_leaves_no_publication() {
    let _serial = serial();
    let path = dir("tear");
    let store = open_fault(&path);
    let prepared = prepared();
    let ids = stage_prepared(&store, &prepared);
    sqlite_fault::tear_next_write();
    assert!(
        store
            .publish(&signed(&prepared, &ids, &prepared.signature))
            .is_err()
    );
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(!store.is_published(&prepared.batch).expect("read"));
    store
        .publish(&signed(&prepared, &ids, &prepared.signature))
        .expect("retry");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn acked_batch_survives_a_later_armed_fault() {
    let _serial = serial();
    let path = dir("acked");
    let store = open_fault(&path);
    let prepared = prepared();
    let ids = stage_prepared(&store, &prepared);
    store
        .publish(&signed(&prepared, &ids, &prepared.signature))
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
    let store = open_fault(&path);
    let prepared = prepared();
    let ids = stage_prepared(&store, &prepared);
    store
        .publish(&signed(&prepared, &ids, &prepared.signature))
        .expect("publish");
    let second = prepared_child(&prepared, b"second-metadata");
    let second_ids = stage_prepared(&store, &second);
    // Misuse the shim on purpose: reset() while the file is still open. The
    // armed sync must fail through the defensive snapshot .get() without
    // touching the file (R01); reset no longer shrinks the vec, so ids stay
    // monotonic (R02).
    reset();
    sqlite_fault::fail_next_sync();
    assert!(
        store
            .publish(&signed(&second, &second_ids, &second.signature))
            .is_err()
    );
    drop(store);
    reset();
    let store = Store::open(&path, &SPACE).expect("reopen");
    assert!(store.is_published(&prepared.batch).expect("read"));
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn reset_between_two_fault_stores_does_not_alias_snapshot_ids() {
    let _serial = serial();
    let dir_a = dir("alias-a");
    let dir_b = dir("alias-b");
    let store_a = open_fault(&dir_a);
    let prepared = prepared();
    let ids_a = stage_prepared(&store_a, &prepared);
    store_a
        .publish(&signed(&prepared, &ids_a, &prepared.signature))
        .expect("publish a");
    // Misuse on purpose: reset() while store_a is open, then a second fault
    // store. Snapshot ids must keep increasing, never alias across the reset.
    reset();
    let store_b = open_fault(&dir_b);
    let ids_b = stage_prepared(&store_b, &prepared);
    sqlite_fault::fail_next_sync();
    assert!(
        store_b
            .publish(&signed(&prepared, &ids_b, &prepared.signature))
            .is_err()
    );
    drop(store_a);
    drop(store_b);
    reset();
    let a = Store::open(&dir_a, &SPACE).expect("reopen a");
    assert!(a.is_published(&prepared.batch).expect("read a"));
    // With aliased ids the armed sync would have restored store A's image into
    // store B's files; reopen must succeed and show no publication.
    let b = Store::open(&dir_b, &SPACE).expect("reopen b");
    assert!(!b.is_published(&prepared.batch).expect("read b"));
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
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
