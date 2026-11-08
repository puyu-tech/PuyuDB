// SPDX-License-Identifier: Apache-2.0

//! I08 Phase 3 (P4) — T02 evidence on a REAL I07-imported space: reuse the
//! I07 git-import fixture as the history generator, then prove the imported
//! version survives reload, snapshot pin, and cache discard with identical
//! views through the public facade. Skips silently when no `git` binary is
//! present (same policy as git_import.rs).

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use puyudb_core::api::Database;
use puyudb_local::git::{self, Capture, GitRepo};
use puyudb_local::import::{self, ImportResult};

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fx {
    dir: PathBuf,
}

impl Fx {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "puyudb-i08-p4-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fx = Self { dir };
        fx.git(&["init", "-q", "-b", "main", "."]);
        fx.git(&["config", "user.email", "t@t"]);
        fx.git(&["config", "user.name", "t"]);
        fx
    }

    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        std::fs::write(self.dir.join(rel), bytes).unwrap();
    }

    fn commit(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", msg]);
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fingerprint(db: &Database, result: &ImportResult) -> (String, Vec<String>, usize) {
    let v = db
        .branch_current_version(&result.branch)
        .unwrap()
        .expect("imported branch has a current version");
    let manifest = db.semantic_manifest_at_version(&v).unwrap();
    let manifest_id: String = manifest
        .id()
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut paths: Vec<String> = db
        .paths_at_version(&v)
        .unwrap()
        .iter()
        .map(|(p, files)| format!("{}=>{}", p.as_str(), files.len()))
        .collect();
    paths.sort();
    let files = db.files_at_version(&v).unwrap().len();
    let _ = v;
    (manifest_id, paths, files)
}

#[test]
fn t02_imported_history_survives_reload_pin_and_cache_discard() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("import");
    fx.write("a.txt", b"hello\n");
    fx.commit("c1");
    fx.write("a.txt", b"hello world\n");
    fx.write("b.txt", b"second\n");
    fx.commit("c2");
    fx.write("b.txt", b"second v2\n");
    fx.commit("c3");

    let repo: GitRepo = git::discover(&fx.dir).expect("discover");
    let capture: Capture = git::capture(&repo).expect("capture");
    let result: ImportResult = import::import(&repo, &capture).expect("import");
    assert!(result.files >= 2);

    let db = Database::open_space(&result.space_dir, &result.space).expect("open space");
    let base = fingerprint(&db, &result);
    let head = db.branch_current_version(&result.branch).unwrap().unwrap();
    // The import-root version resolves and pins.
    let root_version = db.version_of(&[result.import_root]).unwrap();
    assert!(db.create_snapshot("import-head", &head).unwrap());
    assert!(db.create_snapshot("import-root", &root_version).unwrap());

    // Reload: identical view.
    drop(db);
    let db = Database::open_space(&result.space_dir, &result.space).expect("reopen");
    assert_eq!(
        fingerprint(&db, &result),
        base,
        "reload changed imported view"
    );

    // Cache discard: pins still resolve, views identical (D3). (The cache dir
    // may not exist at the default threshold; discarding it is a no-op.)
    let _ = std::fs::remove_dir_all(result.space_dir.join("cache"));
    assert_eq!(db.snapshot_version("import-head").unwrap(), Some(head));
    assert_eq!(
        db.snapshot_version("import-root").unwrap(),
        Some(root_version)
    );
    assert_eq!(
        fingerprint(&db, &result),
        base,
        "cache discard changed view"
    );

    // The root version's closure contains the import root batch.
    assert!(
        db.closure(&root_version)
            .unwrap()
            .contains(&result.import_root)
    );
}
