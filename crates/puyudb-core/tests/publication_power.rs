//! Power-loss model for the publication ledger.
//!
//! The shim VFS discards bytes that never reached a successful sync, or leaves
//! a torn write, then refuses further writes. Reopen uses the normal VFS.

use puyudb_core::identity::{BatchId, WriterEpoch};
use puyudb_core::profile::chunk_id;
use puyudb_core::publication::{Publication, Store};
use sqlite_fault::{self, install, reset};

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

fn publication<'a>(sequence: u64, chunks: &'a [puyudb_core::identity::ChunkId]) -> Publication<'a> {
    Publication {
        batch_id: BatchId::from_array([sequence as u8; 32]),
        writer_epoch: WriterEpoch::from_array([9; 16]),
        sequence,
        body: b"body",
        chunks,
        peer: 1,
    }
}

fn open_fault(path: &std::path::Path) -> Store {
    install();
    reset();
    Store::open_vfs(path, Some("puyu-fault")).expect("fault open")
}

#[test]
fn fault_vfs_can_publish_when_disarmed() {
    let path = dir("ok");
    let bytes = b"durable-chunk";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    store.stage_chunk(id, bytes).expect("stage");
    store.publish(&publication(1, &[id])).expect("publish");
    drop(store);
    reset();
    let store = Store::open(&path).expect("reopen");
    assert!(
        store
            .is_published(&publication(1, &[id]).batch_id)
            .expect("read")
    );
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn lost_sync_during_publish_leaves_no_publication() {
    let path = dir("sync");
    let bytes = b"staged-only";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    store.stage_chunk(id, bytes).expect("stage");
    sqlite_fault::fail_next_sync();
    let err = store
        .publish(&publication(1, &[id]))
        .expect_err("sync lost");
    drop(err);
    drop(store);
    reset();
    let store = Store::open(&path).expect("reopen");
    assert!(
        !store
            .is_published(&publication(1, &[id]).batch_id)
            .expect("read")
    );
    assert_eq!(store.staged_without_publication().expect("staged"), 1);
    store.publish(&publication(1, &[id])).expect("retry");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn torn_write_during_publish_leaves_no_publication() {
    let path = dir("tear");
    let bytes = b"tear-chunk";
    let id = chunk_id(bytes);
    let store = open_fault(&path);
    store.stage_chunk(id, bytes).expect("stage");
    sqlite_fault::tear_next_write();
    assert!(store.publish(&publication(1, &[id])).is_err());
    drop(store);
    reset();
    let store = Store::open(&path).expect("reopen");
    assert!(
        !store
            .is_published(&publication(1, &[id]).batch_id)
            .expect("read")
    );
    store.publish(&publication(1, &[id])).expect("retry");
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn lost_sync_during_stage_does_not_ack_the_chunk() {
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
