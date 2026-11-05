// SPDX-License-Identifier: Apache-2.0

//! T04 power-loss evidence through the public Database facade (I06 P5):
//! fault-VFS sync/write failures during a keyed transact return an
//! environmental (non-§14) error, leave nothing visible, and leave the
//! idempotency key reusable (cancel-before-publish); an acked transact
//! survives a later armed fault.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use puyudb_core::api::Database;
use puyudb_core::identity::{BranchId, ChunkId, SpaceId, WriterEpoch};
use puyudb_core::profile::{self, ArtifactManifest, BatchBody, MetadataArtifact, chunk_id};
use puyudb_core::publication::Publication;
use sqlite_fault::{self, install, reset};

const SPACE: SpaceId = SpaceId::from_array([1; 16]);
const BRANCH: BranchId = BranchId::from_array([2; 16]);
const KEY_SEED: [u8; 32] = [3; 32];
const WRITER_EPOCH: WriterEpoch = WriterEpoch::from_array([4; 16]);

/// The shim VFS keeps one process-wide fault switch and one snapshot per open
/// file, so two tests must never run at once.
static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|err| err.into_inner())
}

fn dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "puyu-i06-power-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("dir");
    path
}

fn metadata_payload(distinct: &[u8]) -> Vec<u8> {
    MetadataArtifact {
        operations: Vec::new(),
        context_assertions: vec![distinct.to_vec()],
    }
    .encode()
    .expect("metadata artifact")
}

struct Prepared {
    body: BatchBody,
    signature: [u8; 64],
    batch: puyudb_core::identity::BatchId,
    manifest: ArtifactManifest,
    payload: Vec<u8>,
}

fn prepared_with(db: &Database, payload: &[u8]) -> Prepared {
    let payload = metadata_payload(payload);
    let manifest = ArtifactManifest::for_payload("metadata-v1", &payload).expect("manifest");
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
        metadata: manifest.id().expect("artifact id"),
    };
    let batch = body.id().expect("id");
    let signature = profile::sign_batch(&KEY_SEED, &body.space, &batch);
    let _ = db; // staging happens in stage_prepared
    Prepared {
        body,
        signature,
        batch,
        manifest,
        payload,
    }
}

fn stage_prepared(db: &Database, p: &Prepared) -> Vec<ChunkId> {
    p.payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(|c| {
            let id = chunk_id(c);
            db.stage_chunk(id, c).expect("stage");
            id
        })
        .collect()
}

fn publication<'a>(p: &'a Prepared, ids: &'a [ChunkId]) -> Publication<'a> {
    Publication {
        body: &p.body,
        signature_key: &p.body.public_key,
        signature: &p.signature,
        artifacts: std::slice::from_ref(&p.manifest),
        chunks: ids,
        peer: 1,
    }
}

/// Create the space through the facade on the fault VFS.
fn fault_create(path: &std::path::Path) -> Database {
    install();
    reset();
    Database::create_space_vfs(path, Some("puyu-fault"), &SPACE).expect("fault create")
}

/// Reopen through the facade on the normal VFS.
fn reopen(path: &std::path::Path) -> Database {
    reset();
    Database::open_space(path, &SPACE).expect("reopen")
}

#[test]
fn lost_sync_during_transact_leaves_nothing_visible_and_key_reusable() {
    let _serial = serial();
    let path = dir("sync");
    let db = fault_create(&path);
    let prepared = prepared_with(&db, b"power-sync");
    let ids = stage_prepared(&db, &prepared);
    sqlite_fault::fail_next_sync();
    let err = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect_err("faulted transact");
    // Environmental fault: not a §14 code, never mislabeled as corruption.
    assert_eq!(err.code(), "internal");
    drop(db);

    let db = reopen(&path);
    // No ack implies nothing visible.
    assert!(!db.bootstrapped().expect("bootstrapped"));
    assert!(db.branch_frontier(&BRANCH).expect("frontier").is_empty());
    assert!(db.receipt_for(b"caller", b"k").expect("receipt").is_none());
    // Cancel-before-publish: the key is still reusable and now commits.
    let receipt = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect("retry transact");
    assert!(db.is_published(&receipt.batch_id).expect("visible"));
    assert_eq!(
        db.branch_frontier(&BRANCH).expect("frontier"),
        vec![receipt.batch_id]
    );
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn torn_write_during_transact_leaves_nothing_visible_and_key_reusable() {
    let _serial = serial();
    let path = dir("tear");
    let db = fault_create(&path);
    let prepared = prepared_with(&db, b"power-tear");
    let ids = stage_prepared(&db, &prepared);
    sqlite_fault::tear_next_write();
    let err = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect_err("faulted transact");
    assert_eq!(err.code(), "internal");
    drop(db);

    let db = reopen(&path);
    assert!(!db.is_published(&prepared.batch).expect("read"));
    assert!(db.receipt_for(b"caller", b"k").expect("receipt").is_none());
    let receipt = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect("retry transact");
    assert!(db.is_published(&receipt.batch_id).expect("visible"));
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn disk_full_during_transact_leaves_nothing_visible_and_key_reusable() {
    let _serial = serial();
    let path = dir("enospc");
    let db = fault_create(&path);
    let prepared = prepared_with(&db, b"power-enospc");
    let ids = stage_prepared(&db, &prepared);
    sqlite_fault::fail_next_write_enospc();
    let err = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect_err("faulted transact");
    assert_eq!(err.code(), "internal");
    drop(db);

    let db = reopen(&path);
    assert!(!db.is_published(&prepared.batch).expect("read"));
    assert!(db.receipt_for(b"caller", b"k").expect("receipt").is_none());
    let receipt = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect("retry transact");
    assert!(db.is_published(&receipt.batch_id).expect("visible"));
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn acked_transact_survives_a_later_armed_fault() {
    let _serial = serial();
    let path = dir("acked");
    let db = fault_create(&path);
    let prepared = prepared_with(&db, b"power-acked");
    let ids = stage_prepared(&db, &prepared);
    let receipt = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect("acked transact");
    // The ack already returned; a later fault must not undo it.
    sqlite_fault::fail_next_sync();
    assert!(db.stage_chunk(chunk_id(b"other"), b"other").is_err());
    drop(db);

    // Reopen through the doc-hidden open_space_vfs hook on the now-disarmed
    // fault VFS, so both facade VFS hooks are exercised by real tests.
    reset();
    let db = Database::open_space_vfs(&path, Some("puyu-fault"), &SPACE).expect("fault reopen");
    assert!(db.is_published(&receipt.batch_id).expect("visible"));
    assert_eq!(
        db.receipt_for(b"caller", b"k").expect("receipt"),
        Some(receipt)
    );
    assert_eq!(
        db.branch_frontier(&BRANCH).expect("frontier"),
        vec![receipt.batch_id]
    );
    // Retry after reopen returns the same receipt (§13.1).
    let retry = db
        .transact(b"caller", b"k", &[], &publication(&prepared, &ids))
        .expect("retry");
    assert_eq!(retry, receipt);
    let _ = std::fs::remove_dir_all(&path);
}
