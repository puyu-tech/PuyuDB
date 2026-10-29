//! Process-kill check for the publication ledger.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use puyudb_core::identity::{ArtifactId, BranchId, SpaceId, WriterEpoch};
use puyudb_core::profile::{self, BatchBody, chunk_id};
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
    assert_eq!(store.staged_without_publication().expect("staged"), 1);
    let body = BatchBody {
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
    };
    let batch = body.id().expect("id");
    assert!(!store.is_published(&batch).expect("read"));
    let signature = profile::sign_batch(&[3; 32], &body.space, &batch);
    store
        .publish(&Publication {
            body: &body,
            signature_key: &body.public_key,
            signature: &signature,
            chunks: &[id],
            peer: 1,
        })
        .expect("publish after crash");
    let _ = std::fs::remove_dir_all(&dir);
}
