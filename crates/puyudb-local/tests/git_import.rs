// SPDX-License-Identifier: Apache-2.0

//! I07 Phase 3 (P4): T05/T08 evidence — real-system-git fixture matrix and
//! hostile fixtures, driving only the public surface of `puyudb-local`
//! (`git::discover`/`capture`/`preview`, `import::import`) and
//! `puyudb_core::api::Database` reads. Every test skips silently when no
//! `git` binary is present (CI without git stays green; the matrix runs
//! wherever git exists, e.g. the maintainer workstation and Linux CI).
//!
//! Fixture matrix (blueprint §17 T05/T08, plan P4 row): unborn HEAD, sha256
//! repo, worktree/gitfile, skip-worktree (sparse), gitlink/submodule,
//! non-UTF-8 path, unmerged mixed index, EOL-CRLF attributes, config-driven
//! core.autocrlf, LFS pointer, user filter + hooks never executed, unsafe
//! path names (no shell interpolation), inert symlink escaping the worktree,
//! source-unchanged invariant (R04), gc retention (§5.4), crash-mid-import
//! retry (D4), changed-repo retry CAS failure, pre-existing `.puyu`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use puyudb_core::api::{Availability, Database};
use puyudb_core::profile::{ModeTag, Path as PuyuPath};
use puyudb_local::git::{
    self, Capture, ContentClass, ConversionProfile, Exclusion, GitRepo, Source,
};
use puyudb_local::import::{self, ImportError, ImportResult};

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

struct Fx {
    dir: PathBuf,
}

static NEXT: AtomicU64 = AtomicU64::new(0);

impl Fx {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "puyudb-i07-p4-{tag}-{}-{}",
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

    fn new_sha256(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "puyudb-i07-p4-{tag}-256-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fx = Self { dir };
        fx.git(&["init", "-q", "--object-format=sha256", "-b", "main", "."]);
        fx.git(&["config", "user.email", "t@t"]);
        fx.git(&["config", "user.name", "t"]);
        fx
    }

    fn git_out(&self, args: &[&str]) -> Output {
        Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .output()
            .unwrap()
    }

    fn git(&self, args: &[&str]) {
        let out = self.git_out(args);
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Run git, returning whether it succeeded (for prune/eviction probes).
    fn git_succeeds(&self, args: &[&str]) -> bool {
        self.git_out(args).status.success()
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn write_raw_name(&self, name: &std::ffi::OsStr, bytes: &[u8]) {
        use std::os::unix::ffi::OsStrExt;
        std::fs::write(self.dir.join(name), bytes).unwrap();
        let _ = name.as_bytes();
    }

    fn commit(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", msg]);
    }

    fn discover(&self) -> GitRepo {
        git::discover(&self.dir).expect("discover")
    }

    fn capture(&self, repo: &GitRepo) -> Capture {
        git::capture(repo).expect("capture")
    }

    fn import(&self, repo: &GitRepo, capture: &Capture) -> ImportResult {
        import::import(repo, capture).expect("import")
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn preview_entry(repo: &GitRepo, capture: &Capture, path: &str) -> git::PreviewEntry {
    let pv = git::preview(repo, capture).expect("preview");
    pv.entries
        .iter()
        .find(|e| e.path == path)
        .unwrap_or_else(|| panic!("no preview entry for {path}"))
        .clone()
}

// --------------------------------------------------------------------------
// T05 fixture matrix
// --------------------------------------------------------------------------

#[test]
fn t05_unborn_head_and_committed_import() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("unborn");
    fx.write("a.txt", b"hello\n");
    fx.git(&["add", "a.txt"]);
    let repo = fx.discover();
    assert!(matches!(repo.head, git::Head::Unborn(ref b) if b == "main"));
    let cap = fx.capture(&repo);
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 1);
    assert!(!result.empty_cap);
    drop(result);
    // After a commit the same repo imports with a commit HEAD.
    fx.commit("c1");
    let repo = fx.discover();
    assert!(matches!(repo.head, git::Head::Commit(_)));
    let cap = fx.capture(&repo);
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 1);
}

#[test]
fn t05_sha256_repo_import() {
    if !git_available() {
        return;
    }
    let fx = Fx::new_sha256("sha256");
    fx.write("f.txt", b"hi\n");
    fx.commit("c1");
    let repo = fx.discover();
    assert_eq!(repo.object_format, git::ObjectFormat::Sha256);
    let cap = fx.capture(&repo);
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 1);
    let db = Database::open_space(&result.space_dir, &result.space).expect("open");
    let files = db.files_at(&[result.import_root]).expect("files_at");
    let epochs = db
        .epochs_at(&[result.import_root], files[0].file)
        .expect("epochs");
    match epochs[0].1.git_oid {
        Some(puyudb_core::identity::GitOid::Sha256(d)) => assert_eq!(d.len(), 32),
        other => panic!("expected sha256 git_oid, got {other:?}"),
    }
}

#[test]
fn t05_worktree_gitfile_import() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("wt");
    fx.write("a.txt", b"hello\n");
    fx.commit("c1");
    let wt = fx.dir.parent().unwrap().join(format!(
        "puyudb-i07-p4-wt-target-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&wt);
    let out = Command::new("git")
        .args(["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "wtb"])
        .current_dir(&fx.dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(wt.join("b.txt"), b"world\n").unwrap();
    let repo = git::discover(&wt).expect("discover worktree");
    assert!(repo.gitfile, "worktree .git is a gitfile");
    assert_eq!(repo.toplevel, wt.canonicalize().unwrap());
    assert_ne!(repo.git_dir, repo.common_dir);
    let cap = git::capture(&repo).expect("capture");
    let result = import::import(&repo, &cap).expect("import");
    assert_eq!(result.space_dir, repo.toplevel.join(".puyu"));
    assert_eq!(result.files, 2); // a.txt (tracked) + b.txt (untracked)
    let _ = std::fs::remove_dir_all(&wt);
}

#[test]
fn t05_sparse_skip_worktree_and_gitlink_submodule() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("sparse");
    fx.write("keep.txt", b"k\n");
    fx.write("sparse.txt", b"s\n");
    fx.commit("c1");
    // Sparse omission: skip-worktree flag on a tracked file.
    fx.git(&["update-index", "--skip-worktree", "sparse.txt"]);
    // Gitlink/submodule entry without any network: fabricate mode 160000.
    let inner = Fx::new("inner");
    inner.write("x", b"x\n");
    inner.commit("inner-c1");
    let sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&inner.dir)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let sha = sha.trim().to_owned();
    fx.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{sha},submod"),
    ]);
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let sparse = preview_entry(&repo, &cap, "sparse.txt");
    assert!(sparse.skip_worktree);
    assert!(
        sparse.excluded.is_none(),
        "sparse omission is not a deletion"
    );
    let sub = preview_entry(&repo, &cap, "submod");
    assert_eq!(sub.excluded, Some(Exclusion::Submodule));
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 2); // keep.txt + sparse.txt (submodule excluded)
    assert!(
        result
            .limitations
            .iter()
            .any(|l| l.contains("sparse omission"))
    );
    assert!(result.limitations.iter().any(|l| l.contains("gitlink")));
}

#[test]
fn t05_unmerged_mixed_index_import() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("conflict");
    fx.write("ok.txt", b"ok\n");
    fx.write("c.txt", b"base\n");
    fx.commit("base");
    fx.git(&["checkout", "-qb", "side"]);
    fx.write("c.txt", b"side\n");
    fx.git(&["commit", "-qam", "side"]);
    fx.git(&["checkout", "-q", "main"]);
    fx.write("c.txt", b"main\n");
    fx.git(&["commit", "-qam", "main"]);
    let out = fx.git_out(&["merge", "side"]);
    assert!(!out.status.success(), "expected conflict");
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let c = preview_entry(&repo, &cap, "c.txt");
    assert_eq!(c.excluded, Some(Exclusion::Unmerged));
    assert_eq!(c.unmerged_stages.len(), 3);
    let result = fx.import(&repo, &cap);
    assert_eq!(
        result.files, 1,
        "only ok.txt is imported; conflict is metadata"
    );
    assert!(
        result
            .limitations
            .iter()
            .any(|l| l.contains("conflict metadata"))
    );
}

#[test]
fn t05_non_utf8_path_excluded() {
    if !git_available() {
        return;
    }
    // APFS (macOS) rejects non-UTF-8 filenames at creation, so this fixture
    // only exists where the filesystem allows raw bytes (Linux).
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    return;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::ffi::OsStrExt;
        let fx = Fx::new("nonutf8");
        fx.write("good.txt", b"g\n");
        let bad = std::ffi::OsStr::from_bytes(b"bad\xff.txt");
        fx.write_raw_name(bad, b"b\n");
        // `git add -A` picks up the non-UTF-8 name without putting it in argv
        // (macOS execve requires UTF-8 arguments).
        fx.git(&["add", "-A"]);
        let repo = fx.discover();
        let cap = fx.capture(&repo);
        let pv = git::preview(&repo, &cap).expect("preview");
        let bad_entry = pv
            .entries
            .iter()
            .find(|e| e.excluded == Some(Exclusion::NonUtf8Path))
            .expect("non-UTF-8 path excluded");
        assert!(
            bad_entry.path.is_empty(),
            "unrepresentable path has no text"
        );
        let result = fx.import(&repo, &cap);
        assert_eq!(result.files, 1, "only the UTF-8 file imports");
    }
}
// --------------------------------------------------------------------------
// T05 conversion profiles (Gate B item 3: documented-limitation decision)
// --------------------------------------------------------------------------

/// EOL-CRLF via `.gitattributes` and config-driven `core.autocrlf`: the
/// baseline is the Git blob (LF), captured verbatim; the conversion profile
/// is reported in the preview. Decision (frozen §8.2 has no conversion
/// field): NO per-file op-5 context assertion — the limitation is reported
/// in the preview/import result instead. Recorded in the phase log and
/// `docs/validation/I07-git-import.md`.
#[test]
fn t05_eol_crlf_attributes_and_config_autocrlf() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("crlf");
    fx.write(".gitattributes", b"*.txt text eol=crlf\n");
    fx.write("doc.txt", b"line1\r\nline2\r\n");
    fx.commit("c1"); // git's own add normalizes the blob to LF
    fx.git(&["config", "core.autocrlf", "true"]);
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let doc = preview_entry(&repo, &cap, "doc.txt");
    assert_eq!(doc.conversion, ConversionProfile::BuiltIn);
    // The captured baseline is the blob (LF), verified through the plumbing.
    let oid = doc.git_oid.expect("tracked");
    let mut reader = git::BlobReader::open(&repo).unwrap();
    let blob = reader
        .read_blob(oid, puyudb_core::profile::MAX_ARTIFACT_RAW_BYTES)
        .unwrap();
    assert_eq!(blob, b"line1\nline2\n", "blob is LF-normalized");
    let pv = git::preview(&repo, &cap).unwrap();
    assert!(
        pv.limitations.iter().any(|l| l.contains("core.autocrlf")),
        "config conversion reported"
    );
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 2); // .gitattributes + doc.txt
    assert!(
        result
            .limitations
            .iter()
            .any(|l| l.contains("core.autocrlf"))
    );
    let db = Database::open_space(&result.space_dir, &result.space).unwrap();
    let files = db.files_at(&[result.import_root]).unwrap();
    let doc_file = files
        .iter()
        .find(|f| f.paths.iter().any(|p| p.as_str() == "doc.txt"))
        .unwrap();
    let epochs = db.epochs_at(&[result.import_root], doc_file.file).unwrap();
    assert!(epochs[0].1.git_oid.is_some());
}

// --------------------------------------------------------------------------
// T08 hostile fixtures: filters/hooks never executed, no shell, inert links
// --------------------------------------------------------------------------

#[test]
fn t08_user_filter_and_hooks_never_executed_and_lfs_pointer_opaque() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("lfs");
    // LFS pointer file, added BEFORE the filter is configured so fixture
    // setup itself does not run it.
    fx.write(
        "big.bin",
        b"version https://git-lfs.github.com/spec/v1\noid sha256:0123\nsize 7\n",
    );
    fx.write("note.txt", b"n\n");
    fx.write(".gitattributes", b"*.bin filter=lfs\n*.txt filter=pwn\n");
    fx.commit("c1");
    // Arm the hostile conversions: any execution would create marker files.
    let pwn_clean = fx.dir.join("pwned-clean");
    let pwn_smudge = fx.dir.join("pwned-smudge");
    let pwn_hook = fx.dir.join("pwned-hook");
    fx.git(&[
        "config",
        "filter.lfs.clean",
        &format!("touch {}", pwn_clean.display()),
    ]);
    fx.git(&[
        "config",
        "filter.lfs.smudge",
        &format!("touch {}", pwn_smudge.display()),
    ]);
    fx.git(&[
        "config",
        "filter.pwn.process",
        &format!("touch {}; cat", pwn_smudge.display()),
    ]);
    fx.git(&[
        "config",
        "filter.pwn.clean",
        &format!("touch {}", pwn_clean.display()),
    ]);
    let hooks = fx.dir.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(
        hooks.join("pre-commit"),
        format!("#!/bin/sh\ntouch {}\n", pwn_hook.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            hooks.join("pre-commit"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    fx.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let bin = preview_entry(&repo, &cap, "big.bin");
    assert_eq!(bin.conversion, ConversionProfile::Unsupported);
    assert_eq!(
        bin.class,
        Some(ContentClass::Opaque),
        "LFS pointer stays opaque"
    );
    let note = preview_entry(&repo, &cap, "note.txt");
    assert_eq!(note.conversion, ConversionProfile::Unsupported);
    let result = fx.import(&repo, &cap);
    assert!(
        result
            .limitations
            .iter()
            .any(|l| l.contains("unsupported conversion"))
    );
    // The pointer blob is captured verbatim, never hydrated.
    let oid = bin.git_oid.unwrap();
    let mut reader = git::BlobReader::open(&repo).unwrap();
    let blob = reader
        .read_blob(oid, puyudb_core::profile::MAX_ARTIFACT_RAW_BYTES)
        .unwrap();
    assert!(blob.starts_with(b"version https://git-lfs.github.com/spec/v1"));
    // No filter, smudge, or hook ever ran.
    for marker in [&pwn_clean, &pwn_smudge, &pwn_hook] {
        assert!(
            !marker.exists(),
            "{} was created — a filter/hook ran!",
            marker.display()
        );
    }
}

#[test]
fn t08_unsafe_path_names_no_shell_interpolation() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("shell");
    // Valid §11 paths that a shell would interpret; passed as literal argv.
    let names = [
        "a;touch pwned1.txt",
        "$(touch pwned2.txt)",
        "`touch pwned3.txt`",
        "x|touch pwned4.txt",
        "b$(id > pwned5.txt)",
    ];
    for name in names {
        fx.write(name, b"x\n");
    }
    fx.git(&["add", "-A"]);
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, names.len());
    let db = Database::open_space(&result.space_dir, &result.space).unwrap();
    let paths = db.paths_at(&[result.import_root]).unwrap();
    for name in names {
        assert!(
            paths.iter().any(|(p, _)| p.as_str() == name),
            "literal path {name:?} imported"
        );
    }
    drop(db);
    for i in 1..=5 {
        let marker = fx.dir.join(format!("pwned{i}.txt"));
        assert!(
            !marker.exists(),
            "shell interpolation happened: {}",
            marker.display()
        );
    }
}

#[test]
fn t08_inert_symlink_escaping_worktree() {
    if !git_available() {
        return;
    }
    #[cfg(not(unix))]
    {
        return;
    }
    #[cfg(unix)]
    {
        let fx = Fx::new("symlink");
        // A secret OUTSIDE the worktree that a following reader would leak.
        let secret = fx
            .dir
            .parent()
            .unwrap()
            .join(format!("puyudb-i07-secret-{}.txt", std::process::id()));
        std::fs::write(&secret, b"SECRET-BYTES").unwrap();
        std::os::unix::fs::symlink("../puyudb-i07-secret-PLACEHOLDER", fx.dir.join("escape")).ok();
        let _ = std::fs::remove_file(fx.dir.join("escape"));
        let target = format!("../{}", secret.file_name().unwrap().to_string_lossy());
        std::os::unix::fs::symlink(&target, fx.dir.join("escape")).unwrap();
        fx.write("plain.txt", b"p\n");
        fx.git(&["add", "-A"]);
        let repo = fx.discover();
        let cap = fx.capture(&repo);
        let esc = preview_entry(&repo, &cap, "escape");
        assert_eq!(esc.mode, ModeTag::Symlink);
        assert_eq!(esc.class, Some(ContentClass::SymlinkTarget));
        // The link target bytes are the inert content; the link is never followed.
        assert_eq!(
            git::read_symlink_target(&repo, b"escape", 4096).unwrap(),
            target.as_bytes()
        );
        assert!(
            git::read_worktree_file(&repo, b"escape", 4096)
                .unwrap()
                .is_none()
        );
        let result = fx.import(&repo, &cap);
        assert_eq!(result.files, 2); // escape (symlink target) + plain.txt
        let _ = std::fs::remove_file(&secret);
    }
}

// --------------------------------------------------------------------------
// R04 source-unchanged invariant + §5.4 gc retention + D4 crash/retry
// --------------------------------------------------------------------------

/// Byte-compare the repository's Git state and worktree around an import.
fn git_state_snapshot(fx: &Fx) -> Vec<u8> {
    let mut snap = Vec::new();
    fn add(snap: &mut Vec<u8>, label: &str, bytes: &[u8]) {
        snap.extend_from_slice(label.as_bytes());
        snap.push(0);
        snap.extend_from_slice(bytes);
        snap.push(0);
    }
    add(
        &mut snap,
        "head",
        &fx.git_out(&["rev-parse", "HEAD"]).stdout,
    );
    add(&mut snap, "refs", &fx.git_out(&["for-each-ref"]).stdout);
    add(
        &mut snap,
        "index-list",
        &fx.git_out(&["ls-files", "-s", "-z"]).stdout,
    );
    // status raw, minus our own .puyu control dir (untracked noise we create)
    let status = fx
        .git_out(&[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-z",
            "-uall",
            "--ignored=matching",
        ])
        .stdout;
    for field in status.split(|b| *b == 0) {
        if field.starts_with(b"? .puyu") || field.starts_with(b"! .puyu") {
            continue;
        }
        snap.extend_from_slice(field);
        snap.push(0);
    }
    // The physical index file bytes.
    let index_path = fx.dir.join(".git").join("index");
    add(
        &mut snap,
        "index-file",
        &std::fs::read(&index_path).unwrap(),
    );
    // Worktree file hashes (excluding .puyu).
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files(&fx.dir, &mut files);
    files.sort();
    for f in files {
        if f.components().any(|c| c.as_os_str() == ".puyu") {
            continue;
        }
        add(
            &mut snap,
            &format!("file:{}", f.strip_prefix(&fx.dir).unwrap().display()),
            &std::fs::read(&f).unwrap(),
        );
    }
    snap
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        // Do not descend into .git (Git state is compared via refs/index/HEAD).
        if path.file_name().map(|n| n == ".git").unwrap_or(false) {
            continue;
        }
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn t05_source_unchanged_by_import() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("unchanged");
    fx.write("a.txt", b"hello\n");
    fx.write("sub/b.bin", &[0u8, 1, 2, 3]);
    fx.commit("c1");
    fx.write("untracked.txt", b"u\n");
    let before = git_state_snapshot(&fx);
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 3);
    let after = git_state_snapshot(&fx);
    assert_eq!(
        before, after,
        "import mutated the user's Git state or worktree (R04)"
    );
}

#[test]
fn t05_gc_cannot_break_history() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("gc");
    fx.write("big.txt", b"important baseline bytes\n");
    fx.commit("c1");
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let result = fx.import(&repo, &cap);
    let db = Database::open_space(&result.space_dir, &result.space).unwrap();
    let files = db.files_at(&[result.import_root]).unwrap();
    let epochs = db.epochs_at(&[result.import_root], files[0].file).unwrap();
    let baseline = epochs[0].1.baseline;
    let git_oid = epochs[0].1.git_oid.expect("tracked blob oid");
    assert_eq!(
        db.artifact_availability(&baseline).unwrap(),
        Some(Availability::Resident),
        "baseline is independently retained at import"
    );
    drop(db);
    // Make the blob unreferenced (index + ref + reflogs) and prune it.
    fx.git(&["rm", "-q", "--cached", "big.txt"]);
    fx.git(&["update-ref", "-d", "refs/heads/main"]);
    fx.git(&["reflog", "expire", "--expire=now", "--all"]);
    fx.git(&["gc", "--prune=now", "-q"]);
    let oid_hex = repo.object_format.to_hex(&git_oid).unwrap();
    assert!(
        !fx.git_succeeds(&["cat-file", "-e", std::str::from_utf8(&oid_hex).unwrap()]),
        "fixture sanity: git gc pruned the blob"
    );
    // §5.4: the PuyuDB baseline survives the prune.
    let db = Database::open_space(&result.space_dir, &result.space).unwrap();
    assert_eq!(
        db.artifact_availability(&baseline).unwrap(),
        Some(Availability::Resident),
        "baseline still resident after git gc --prune=now"
    );
    let files = db.files_at(&[result.import_root]).unwrap();
    let epochs = db.epochs_at(&[result.import_root], files[0].file).unwrap();
    assert_eq!(epochs[0].1.baseline, baseline);
    assert_eq!(epochs[0].1.git_oid, Some(git_oid), "typed oid survives gc");
}

#[test]
fn t08_crash_mid_import_then_retry_per_d4() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("crash");
    // 600 files -> 3 bounded batches (Q2 constant 256).
    for i in 0..600 {
        fx.write(
            &format!("src/f{i:03}.txt"),
            format!("content {i}\n").as_bytes(),
        );
    }
    fx.git(&["add", "-A"]);
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    // Crash simulation: stop after the first file batch, before the root.
    let root = import::import_partial(&repo, &cap, 1).expect("partial import");
    assert_eq!(root, None, "no import root after a mid-import crash");
    // Retry per D4: the full import reuses committed batches (idempotent
    // keys) and publishes the root.
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 600);
    assert_eq!(result.file_batches, 3);
    {
        let db = Database::open_space(&result.space_dir, &result.space).unwrap();
        assert_eq!(
            db.branch_frontier(&result.branch).unwrap(),
            vec![result.import_root]
        );
        assert!(db.is_published(&result.import_root).unwrap());
    } // handle dropped: the owner lock is free for the next import
    // A second full retry is a pure idempotent no-op with the same root.
    let again = fx.import(&repo, &cap);
    assert_eq!(again.import_root, result.import_root);
}

#[test]
fn t08_changed_repo_retry_fails_cas_recovery_is_remove_puyu() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("changed");
    fx.write("a.txt", b"v1\n");
    fx.git(&["add", "a.txt"]);
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    let first = fx.import(&repo, &cap);
    assert_eq!(first.files, 1);
    // The repository changes after the import.
    fx.write("a.txt", b"v2\n");
    let cap2 = fx.capture(&repo);
    assert_ne!(cap.fingerprint, cap2.fingerprint);
    // Re-importing with the NEW capture fails the CAS against the old
    // import's frontier (documented recovery: remove .puyu, future I11/I14).
    match import::import(&repo, &cap2) {
        Err(ImportError::Api(err)) => assert_eq!(err.code(), "precondition_failed"),
        other => panic!("expected CAS precondition failure, got {other:?}"),
    }
    // The old import is intact and still readable.
    let db = Database::open_space(&first.space_dir, &first.space).unwrap();
    assert_eq!(
        db.branch_frontier(&first.branch).unwrap(),
        vec![first.import_root]
    );
}

#[test]
fn t08_preexisting_puyu_dir_rejected_not_clobbered() {
    if !git_available() {
        return;
    }
    let fx = Fx::new("preexist");
    fx.write("a.txt", b"v1\n");
    fx.git(&["add", "a.txt"]);
    // A user directory at the reserved control segment, not ours.
    let puyu = fx.dir.join(".puyu");
    std::fs::create_dir_all(&puyu).unwrap();
    std::fs::write(puyu.join("sentinel.txt"), b"user data").unwrap();
    let repo = fx.discover();
    let cap = fx.capture(&repo);
    match import::import(&repo, &cap) {
        Err(ImportError::PreExistingPuyuDir(dir)) => {
            assert_eq!(dir, puyu.canonicalize().unwrap())
        }
        other => panic!("expected PreExistingPuyuDir, got {other:?}"),
    }
    assert_eq!(
        std::fs::read(puyu.join("sentinel.txt")).unwrap(),
        b"user data",
        "pre-existing .puyu content untouched"
    );
    // An EMPTY pre-existing .puyu is fine: the space is created inside it.
    std::fs::remove_file(puyu.join("sentinel.txt")).unwrap();
    let result = fx.import(&repo, &cap);
    assert_eq!(result.files, 1);
}

// Keep the PuyuPath import used on every cfg.
#[test]
fn path_validation_surface_is_exercised() {
    assert!(PuyuPath::new("a//b").is_err());
    assert!(PuyuPath::new(".puyu/x").is_err());
    assert!(PuyuPath::new("a.txt").is_ok());
    let _ = Source::Tracked;
}
