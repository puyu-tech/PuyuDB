//! Process-kill check for the publication ledger.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use puyudb_core::identity::{BatchId, WriterEpoch};
use puyudb_core::profile::chunk_id;
use puyudb_core::publication::{Publication, Store};

fn child_dir() -> Option<PathBuf> {
    std::env::var_os("PUYU_PUBLICATION_CRASH").map(PathBuf::from)
}

#[test]
fn kill_after_staged_chunk_leaves_no_publication() {
    if let Some(dir) = child_dir() {
        let store = Store::open(&dir).expect("child open");
        let bytes = b"staged-before-kill";
        let id = chunk_id(bytes);
        store.stage_chunk(id, bytes).expect("stage");
        std::fs::write(dir.join("ready"), b"1").expect("ready");
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    let dir = std::env::temp_dir().join(format!("puyu-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mut child = Command::new(std::env::current_exe().expect("exe"))
        .arg("kill_after_staged_chunk_leaves_no_publication")
        .arg("--exact")
        .env("PUYU_PUBLICATION_CRASH", &dir)
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
                "child never staged: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();

    let store = Store::open(&dir).expect("reopen");
    let id = chunk_id(b"staged-before-kill");
    let batch = BatchId::from_array([1; 32]);
    assert!(!store.is_published(&batch).expect("read"));
    assert_eq!(store.staged_without_publication().expect("staged"), 1);
    let publication = Publication {
        batch_id: batch,
        writer_epoch: WriterEpoch::from_array([7; 16]),
        sequence: 1,
        body: b"not-yet",
        chunks: &[id],
        peer: 1,
    };
    store.publish(&publication).expect("publish after crash");
    let _ = std::fs::remove_dir_all(&dir);
}
