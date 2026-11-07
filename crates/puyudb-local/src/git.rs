// SPDX-License-Identifier: Apache-2.0

//! I07 Phase 1 (P1+P2): read-only Git discovery, capture, and import preview.
//!
//! Blueprint §7.1–§7.3, spec §8.1/§11, plan decisions D1–D5:
//!
//! - **D1:** all Git access is read-only system-git plumbing via `std::process`
//!   (`rev-parse`, `status --porcelain=v2 -z`, `ls-files -s -t -z`,
//!   `cat-file --batch(-check) -z`, `check-ignore -z --stdin`,
//!   `check-attr -z --stdin`). NUL-delimited output only, `--end-of-options`
//!   where the command accepts it (paths are never passed as CLI arguments;
//!   stdin-fed paths are NUL-delimited, which is the end-of-options
//!   protection for them), typed output verification, no shell interpolation,
//!   size caps on every read. A missing `git` binary is an environmental
//!   failure ([`Error::Environment`]), never a §14 code. No new crate
//!   dependency: only `puyudb-core`'s public modules.
//! - **D5:** no Git filter, hook, textconv, smudge/clean, or process filter is
//!   ever executed; unsupported conversion profiles are classified
//!   [`ConversionProfile::Unsupported`] and captured opaque with a reported
//!   limitation.
//!
//! This module never writes to the repository (R04): every git invocation is a
//! read-only query. P3 (import execution through `puyudb_core::api::Database`)
//! is a later phase and lives in `import.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path as FsPath, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};

use puyudb_core::identity::GitOid;
use puyudb_core::profile::{self, ModeTag, Path};

/// Cap for one non-streaming plumbing call's stdout (status, ls-files,
/// check-ignore, check-attr). Bounded untrusted input (D1/R06).
const MAX_PLUMBING_OUTPUT: u64 = 64 * 1024 * 1024;
/// Cap for captured stderr text.
const MAX_STDERR: u64 = 4096;
/// Cap for one `cat-file --batch` header line.
const MAX_HEADER: u64 = 256;

/// Failure of the Git discovery/capture layer.
///
/// [`Error::Environment`] and [`Error::Git`] are environmental (missing git
/// binary, io, plumbing failure): deliberately not §14 codes (D1).
/// [`Error::Parse`] is typed-output verification failure (D1).
#[derive(Debug)]
pub enum Error {
    /// Environmental failure: git binary missing, spawn/io error, or an
    /// unreadable repository path. Never a §14 code.
    Environment(String),
    /// The plumbing command itself failed; carries bounded stderr text.
    Git(String),
    /// Plumbing output failed typed verification (unexpected record shape,
    /// bad OID, oversize read, poisoned stream).
    Parse(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment(msg) => write!(f, "git environment: {msg}"),
            Self::Git(msg) => write!(f, "git plumbing failed: {msg}"),
            Self::Parse(msg) => write!(f, "git output verification failed: {msg}"),
        }
    }
}

impl std::error::Error for Error {}

fn parse_err(msg: impl Into<String>) -> Error {
    Error::Parse(msg.into())
}

// --------------------------------------------------------------------------
// Plumbing runner (D1)
// --------------------------------------------------------------------------

struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Run one read-only git command with `dir` as the working directory.
///
/// `args` are compile-time literals (plus the filter neutralizers from
/// [`with_filter_neutralizers`]); no untrusted data is ever placed on the
/// command line (paths flow through NUL-delimited stdin or not at all), and
/// `--end-of-options` is passed by callers whose command accepts it. Stdout
/// is capped at `cap` bytes; stderr at [`MAX_STDERR`]. Stdin is written on a
/// helper thread so a large piped input cannot deadlock against a full
/// stdout pipe.
fn run_git(
    dir: &FsPath,
    args: &[String],
    stdin_data: Option<&[u8]>,
    cap: u64,
) -> Result<Output, Error> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(dir)
        .stdin(if stdin_data.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|err| Error::Environment(format!("cannot spawn system git: {err}")))?;
    let mut child_stdin = child.stdin.take();
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut child_stderr = child.stderr.take();
    let (out, status, err) = std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            if let (Some(mut pipe), Some(data)) = (child_stdin.take(), stdin_data) {
                let _ = pipe.write_all(data);
            }
        });
        // Drain stderr to EOF on a helper thread (keeping only MAX_STDERR
        // bytes): a child writing more than the pipe buffer to stderr must
        // not block inside wait().
        let drainer = scope.spawn(|| {
            let mut kept = Vec::new();
            let mut buf = [0u8; 8192];
            if let Some(mut stderr) = child_stderr.take() {
                loop {
                    match stderr.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let room = MAX_STDERR as usize - kept.len().min(MAX_STDERR as usize);
                            if room > 0 {
                                kept.extend_from_slice(&buf[..n.min(room)]);
                            }
                        }
                    }
                }
            }
            kept
        });
        let mut out = Vec::new();
        let read = stdout
            .by_ref()
            .take(cap + 1)
            .read_to_end(&mut out)
            .map_err(|err| Error::Environment(format!("reading git stdout: {err}")));
        let status = child
            .wait()
            .map_err(|err| Error::Environment(format!("waiting for git: {err}")));
        let _ = writer.join();
        let err = drainer.join().unwrap_or_default();
        match (read, status) {
            (Ok(_), Ok(status)) => Ok((out, status, err)),
            (Err(e), _) | (_, Err(e)) => Err(e),
        }
    })?;
    if out.len() as u64 > cap {
        return Err(parse_err(format!(
            "git {} output exceeds the {cap} byte cap",
            args.first().map_or("?", String::as_str)
        )));
    }
    Ok(Output {
        status,
        stdout: out,
        stderr: err,
    })
}

/// Every configured `filter.<driver>.*` is overridden with an EMPTY command
/// for our read-only invocations. In git's `apply_filter`, `cmd && *cmd` is
/// required before anything is spawned, so an empty command means zero
/// filter processes ever run — D5 (import never executes user filters, even
/// when `git status` rehashes racy worktree entries). Enumerated via
/// read-only `git config --get-regexp` (exit 1 = none configured).
fn filter_neutralizers(dir: &FsPath) -> Vec<String> {
    let out = run_git(
        dir,
        &[
            "config".to_owned(),
            "--get-regexp".to_owned(),
            "^filter\\.".to_owned(),
        ],
        None,
        MAX_PLUMBING_OUTPUT,
    );
    let Ok(out) = out else { return Vec::new() };
    if !out.status.success() {
        return Vec::new(); // exit 1: no filter.* config at all
    }
    let mut drivers: Vec<String> = Vec::new();
    for line in out.stdout.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        // "filter.<driver>.<capability> <value>"
        let Some(key) = line.split(|b| *b == b' ').next() else {
            continue;
        };
        let text = String::from_utf8_lossy(key);
        let mut parts = text.split('.');
        if parts.next() != Some("filter") {
            continue;
        }
        if let Some(driver) = parts.next()
            && !driver.is_empty()
            && !drivers.iter().any(|d| d == driver)
        {
            drivers.push(driver.to_owned());
        }
    }
    let mut args = Vec::new();
    for driver in drivers {
        for cap in ["clean", "smudge", "process"] {
            args.push("-c".to_owned());
            args.push(format!("filter.{driver}.{cap}="));
        }
    }
    args
}

/// Base args + filter neutralizers, as owned strings for `run_git`.
fn with_filter_neutralizers(dir: &FsPath, base: &[&str]) -> Vec<String> {
    let mut args = filter_neutralizers(dir);
    args.extend(base.iter().map(|s| (*s).to_owned()));
    args
}

/// Run git expecting success; non-zero exit is [`Error::Git`] with bounded
/// stderr (never a §14 code). `ok_codes` adds documented non-error exits
/// (e.g. `check-ignore` exits 1 when no path is ignored).
fn git_ok(
    dir: &FsPath,
    args: &[&str],
    stdin_data: Option<&[u8]>,
    cap: u64,
) -> Result<Vec<u8>, Error> {
    git_ok_codes(dir, args, stdin_data, cap, &[])
}

fn git_ok_codes(
    dir: &FsPath,
    args: &[&str],
    stdin_data: Option<&[u8]>,
    cap: u64,
    ok_codes: &[i32],
) -> Result<Vec<u8>, Error> {
    let args = with_filter_neutralizers(dir, args);
    let out = run_git(dir, &args, stdin_data, cap)?;
    if !out.status.success() && !ok_codes.contains(&out.status.code().unwrap_or(-1)) {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(Error::Git(format!("git {}: {msg}", args.join(" "))));
    }
    Ok(out.stdout)
}

/// Build NUL-delimited stdin for a path/oid list, capped.
fn nul_input(items: impl IntoIterator<Item = Vec<u8>>) -> Result<Vec<u8>, Error> {
    let mut input = Vec::new();
    for item in items {
        if input.len() as u64 + item.len() as u64 + 1 > MAX_PLUMBING_OUTPUT {
            return Err(parse_err("stdin request list exceeds the cap"));
        }
        input.extend_from_slice(&item);
        input.push(0);
    }
    Ok(input)
}

fn unhex(s: &[u8]) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in s.as_chunks::<2>().0 {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

fn hex(bytes: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = Vec::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize]);
        out.push(HEX[(b & 0x0f) as usize]);
    }
    out
}

// --------------------------------------------------------------------------
// Discovery (P1, §7.1)
// --------------------------------------------------------------------------

/// Git object format, typed per spec §8.1 (1 = SHA-1/20B, 2 = SHA-256/32B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectFormat {
    /// SHA-1, 40 hex chars, 20 raw bytes.
    Sha1,
    /// SHA-256, 64 hex chars, 32 raw bytes.
    Sha256,
}

impl ObjectFormat {
    fn from_name(name: &[u8]) -> Result<Self, Error> {
        match name {
            b"sha1" => Ok(Self::Sha1),
            b"sha256" => Ok(Self::Sha256),
            other => Err(parse_err(format!(
                "unsupported object format {}",
                String::from_utf8_lossy(other)
            ))),
        }
    }

    fn oid_hex_len(self) -> usize {
        match self {
            Self::Sha1 => 40,
            Self::Sha256 => 64,
        }
    }

    fn zero_oid_hex(self) -> Vec<u8> {
        vec![b'0'; self.oid_hex_len()]
    }

    /// The well-known empty-blob OID for this algorithm (git special-cases
    /// it; intent-to-add entries point at it).
    fn empty_blob(self) -> GitOid {
        match self {
            Self::Sha1 => GitOid::Sha1([
                0xe6, 0x9d, 0xe2, 0x9b, 0xb2, 0xd1, 0xd6, 0x43, 0x4b, 0x8b, 0x29, 0xae, 0x77, 0x5a,
                0xd8, 0xc2, 0xe4, 0x8c, 0x53, 0x91,
            ]),
            Self::Sha256 => GitOid::Sha256([
                0x47, 0x3a, 0x0f, 0x4c, 0x3b, 0xe8, 0xa9, 0x36, 0x81, 0xa2, 0x67, 0xe3, 0xb1, 0xe9,
                0xa7, 0xdc, 0xda, 0x11, 0x85, 0x43, 0x6f, 0xe1, 0x41, 0xf7, 0x74, 0x91, 0x20, 0xa3,
                0x03, 0x72, 0x18, 0x13,
            ]),
        }
    }

    /// Parse and type-verify a hex OID against this format (spec §8.1).
    pub fn parse_oid(self, text: &[u8]) -> Result<GitOid, Error> {
        if text.len() != self.oid_hex_len() {
            return Err(parse_err(format!(
                "oid has {} hex chars, expected {}",
                text.len(),
                self.oid_hex_len()
            )));
        }
        let raw = unhex(text).ok_or_else(|| parse_err("oid is not hex"))?;
        match self {
            Self::Sha1 => Ok(GitOid::Sha1(raw.try_into().expect("20 bytes"))),
            Self::Sha256 => Ok(GitOid::Sha256(raw.try_into().expect("32 bytes"))),
        }
    }

    /// Hex text of an OID, verified to match this format.
    pub fn to_hex(self, oid: &GitOid) -> Result<Vec<u8>, Error> {
        if oid.digest().len() * 2 != self.oid_hex_len() {
            return Err(parse_err(
                "oid digest length does not match the object format",
            ));
        }
        Ok(hex(oid.digest()))
    }
}

/// HEAD state: a commit OID, or an unborn branch name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// HEAD resolves to a commit.
    Commit(GitOid),
    /// Unborn HEAD on this branch (short name, e.g. `main`).
    Unborn(String),
}

/// A discovered Git repository/worktree (P1, §7.1: gitfile and common-dir
/// aware, never assumed to be `.git/HEAD` + `.git/index`).
#[derive(Debug, Clone)]
pub struct GitRepo {
    /// Canonical working-tree root (`--show-toplevel`).
    pub toplevel: PathBuf,
    /// Canonical worktree-specific git dir (`--git-dir`).
    pub git_dir: PathBuf,
    /// Canonical common git dir (`--git-common-dir`).
    pub common_dir: PathBuf,
    /// Whether `<toplevel>/.git` is a gitfile (linked worktree / submodule).
    pub gitfile: bool,
    /// Object format (`--show-object-format`).
    pub object_format: ObjectFormat,
    /// HEAD state.
    pub head: Head,
}

fn resolve_under(base: &FsPath, raw: &[u8]) -> Result<PathBuf, Error> {
    let text = std::str::from_utf8(raw).map_err(|_| parse_err("git-returned path is not UTF-8"))?;
    let path = FsPath::new(text);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    abs.canonicalize()
        .map_err(|err| Error::Environment(format!("cannot canonicalize {}: {err}", abs.display())))
}

fn discover_head(dir: &FsPath, format: ObjectFormat) -> Result<Head, Error> {
    // rev-parse never touches worktree content, so no filter neutralizers.
    let out = run_git(
        dir,
        &[
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            "-q".to_owned(),
            "HEAD".to_owned(),
        ],
        None,
        MAX_HEADER,
    )?;
    if out.status.success() {
        let line = out.stdout.trim_ascii_end();
        return Ok(Head::Commit(format.parse_oid(line)?));
    }
    // Unborn HEAD (exit 1, empty output): the branch name comes from
    // symbolic-ref. Any other failure propagates as a plumbing error.
    let sym = git_ok(dir, &["symbolic-ref", "-q", "HEAD"], None, MAX_HEADER)?;
    let text = std::str::from_utf8(sym.trim_ascii_end())
        .map_err(|_| parse_err("symbolic-ref output is not UTF-8"))?;
    let branch = text.strip_prefix("refs/heads/").unwrap_or(text).to_owned();
    Ok(Head::Unborn(branch))
}

/// Discover the repository containing `start` (read-only, §7.1).
///
/// # Errors
/// [`Error::Environment`] when `start` is not readable or git is missing;
/// [`Error::Git`]/[`Error::Parse`] when discovery output fails verification.
pub fn discover(start: impl AsRef<FsPath>) -> Result<GitRepo, Error> {
    let start = start.as_ref().canonicalize().map_err(|err| {
        Error::Environment(format!("cannot read {}: {err}", start.as_ref().display()))
    })?;
    let out = git_ok(
        &start,
        &[
            "rev-parse",
            "--git-dir",
            "--git-common-dir",
            "--show-object-format",
            "--show-toplevel",
        ],
        None,
        MAX_PLUMBING_OUTPUT,
    )?;
    let lines: Vec<&[u8]> = out
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    // Exactly four facts; a path containing a newline yields extra lines and
    // fails safe instead of being misparsed.
    if lines.len() != 4 {
        return Err(parse_err(format!(
            "rev-parse returned {} lines, expected 4",
            lines.len()
        )));
    }
    let object_format = ObjectFormat::from_name(lines[2])?;
    let toplevel = resolve_under(&start, lines[3])?;
    let git_dir = resolve_under(&start, lines[0])?;
    let common_dir = resolve_under(&start, lines[1])?;
    let gitfile = toplevel.join(".git").is_file();
    let head = discover_head(&start, object_format)?;
    Ok(GitRepo {
        toplevel,
        git_dir,
        common_dir,
        gitfile,
        object_format,
        head,
    })
}

// --------------------------------------------------------------------------
// Capture (P1, §7.2/1–4)
// --------------------------------------------------------------------------

/// One index entry from `ls-files -s -t -z`: staged content, stage number
/// (0 = merged, 1–3 = unmerged conflict stages), and the `-t` tag
/// (`H` cached, `S` skip-worktree, …).
#[derive(Debug, Clone)]
pub struct IndexEntry {
    /// Raw path bytes (not guaranteed UTF-8, §7.1).
    pub path: Vec<u8>,
    /// Raw git mode: 0o100644, 0o100755, 0o120000 (symlink), 0o160000 (gitlink).
    pub mode: u32,
    /// Blob/commit OID of this stage.
    pub oid: GitOid,
    /// Stage number: 0 merged, 1 base, 2 ours, 3 theirs.
    pub stage: u8,
    /// The `ls-files -t` tag byte.
    pub tag: u8,
}

impl IndexEntry {
    /// Skip-worktree (sparse omission) flag — not a deletion (§7.3).
    pub fn skip_worktree(&self) -> bool {
        self.tag == b'S'
    }
}

/// One parsed `status --porcelain=v2 -z` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusRecord {
    /// `# …` header line.
    Header(Vec<u8>),
    /// `1 …`: ordinary changed entry, index-vs-HEAD / worktree-vs-index XY.
    Ordinary {
        /// XY status pair.
        xy: [u8; 2],
        /// Path bytes.
        path: Vec<u8>,
        /// Index hash `hI` (`None` when the zero OID: the intent-to-add
        /// marker — a genuinely staged empty file has `hI` = the empty-blob
        /// OID instead, spec §8.2 empty baselines are legitimate).
        index_oid: Option<GitOid>,
    },
    /// `2 …`: renamed/copied entry; carries the original path.
    Renamed {
        /// XY status pair.
        xy: [u8; 2],
        /// Path bytes.
        path: Vec<u8>,
        /// Original path bytes.
        orig: Vec<u8>,
    },
    /// `u …`: unmerged entry with the present conflict stages (1–3).
    Unmerged {
        /// XY status pair.
        xy: [u8; 2],
        /// Path bytes.
        path: Vec<u8>,
        /// Present conflict stages (stage number, OID).
        stages: Vec<(u8, GitOid)>,
    },
    /// `? …`: untracked path.
    Untracked(Vec<u8>),
    /// `! …`: ignored path (only with `--ignored`).
    Ignored(Vec<u8>),
}

fn parse_ls_files(out: &[u8], format: ObjectFormat) -> Result<Vec<IndexEntry>, Error> {
    let mut entries = Vec::new();
    for field in out.split(|b| *b == 0) {
        if field.is_empty() {
            continue;
        }
        let Some(tab) = field.iter().position(|b| *b == b'\t') else {
            return Err(parse_err("ls-files record without TAB"));
        };
        let (head, path) = (&field[..tab], &field[tab + 1..]);
        let parts: Vec<&[u8]> = head.split(|b| *b == b' ').collect();
        if parts.len() != 4 || parts[0].len() != 1 {
            return Err(parse_err(
                "ls-files record is not <tag> <mode> <oid> <stage>",
            ));
        }
        let mode_text = std::str::from_utf8(parts[1]).map_err(|_| parse_err("bad mode"))?;
        let mode = u32::from_str_radix(mode_text, 8).map_err(|_| parse_err("bad octal mode"))?;
        let oid = format.parse_oid(parts[2])?;
        let stage_text = std::str::from_utf8(parts[3]).map_err(|_| parse_err("bad stage"))?;
        let stage = stage_text
            .parse::<u8>()
            .map_err(|_| parse_err("bad stage number"))?;
        entries.push(IndexEntry {
            path: path.to_vec(),
            mode,
            oid,
            stage,
            tag: parts[0][0],
        });
    }
    Ok(entries)
}

fn parse_status(out: &[u8], format: ObjectFormat) -> Result<Vec<StatusRecord>, Error> {
    let mut fields = out.split(|b| *b == 0);
    let mut records = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        match field[0] {
            b'#' => {
                let body = &field[1..];
                records.push(StatusRecord::Header(
                    body.strip_prefix(b" ").unwrap_or(body).to_vec(),
                ));
            }
            b'?' => records.push(StatusRecord::Untracked(field[2..].to_vec())),
            b'!' => records.push(StatusRecord::Ignored(field[2..].to_vec())),
            b'1' | b'2' | b'u' => {
                let kind = field[0];
                // "1 XY sub mH mI mW hH hI path" (9 fields), "2 … R<score> path"
                // (10 fields + orig path in the next NUL field), "u XY sub m1
                // m2 m3 mW h1 h2 h3 path" (11 fields). The path is the last
                // space-separated field and may itself contain spaces.
                let max = match kind {
                    b'1' => 9,
                    b'2' => 10,
                    _ => 11,
                };
                let tokens: Vec<&[u8]> = field.splitn(max, |b| *b == b' ').collect();
                if tokens.len() != max || tokens[1].len() != 2 {
                    return Err(parse_err("malformed status record"));
                }
                let xy = [tokens[1][0], tokens[1][1]];
                let path = tokens[max - 1].to_vec();
                match kind {
                    b'1' => {
                        // hI is the 8th field; the zero OID means "no staged
                        // content" (intent-to-add), not an empty blob.
                        let index_oid = if tokens[7] == format.zero_oid_hex().as_slice() {
                            None
                        } else {
                            Some(format.parse_oid(tokens[7])?)
                        };
                        records.push(StatusRecord::Ordinary {
                            xy,
                            path,
                            index_oid,
                        });
                    }
                    b'2' => {
                        let orig = fields
                            .next()
                            .ok_or_else(|| parse_err("rename record without orig path"))?
                            .to_vec();
                        records.push(StatusRecord::Renamed { xy, path, orig });
                    }
                    _ => {
                        let mut stages = Vec::new();
                        for (idx, stage) in (1u8..=3).zip(7..10) {
                            if tokens[stage] != format.zero_oid_hex().as_slice() {
                                stages.push((idx, format.parse_oid(tokens[stage])?));
                            }
                        }
                        records.push(StatusRecord::Unmerged { xy, path, stages });
                    }
                }
            }
            other => {
                return Err(parse_err(format!(
                    "unknown status record tag {}",
                    other as char
                )));
            }
        }
    }
    Ok(records)
}

/// A captured scan of one repository: index and status kept separate
/// (§7.2/3: the index is not HEAD), plus the starting status fingerprint.
#[derive(Debug, Clone)]
pub struct Capture {
    /// The discovered repository this scan belongs to.
    pub repo: GitRepo,
    /// Full index, including unmerged stages and skip-worktree entries.
    pub index: Vec<IndexEntry>,
    /// Parsed porcelain v2 records.
    pub status: Vec<StatusRecord>,
    /// Starting status fingerprint (§7.2/3): binds the exact scan inputs;
    /// P3 derives idempotency keys from it (D4).
    pub fingerprint: [u8; 32],
}

/// Capture index + status + fingerprint of `repo`'s worktree (read-only).
///
/// # Errors
/// Environmental or verification failures; see [`Error`].
pub fn capture(repo: &GitRepo) -> Result<Capture, Error> {
    let ls = git_ok(
        &repo.toplevel,
        &["ls-files", "-s", "-t", "-z", "--end-of-options"],
        None,
        MAX_PLUMBING_OUTPUT,
    )?;
    let status_raw = git_ok(
        &repo.toplevel,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "-z",
            "-uall",
            "--ignored=matching",
            "--end-of-options",
        ],
        None,
        MAX_PLUMBING_OUTPUT,
    )?;
    let index = parse_ls_files(&ls, repo.object_format)?;
    let status = parse_status(&status_raw, repo.object_format)?;
    // `.puyu` is the reserved control segment (spec §11): PuyuDB's own space
    // directory inside the worktree is never user content, so untracked or
    // ignored `.puyu/…` records are dropped from the scan (and its
    // fingerprint) — creating the space must not change the captured state,
    // which the §7.2/6 recheck compares.
    let status: Vec<StatusRecord> = status
        .into_iter()
        .filter(|r| !is_puyu_control_record(r))
        .collect();
    let mut fp_input = Vec::new();
    fp_input.extend_from_slice(match repo.object_format {
        ObjectFormat::Sha1 => b"sha1",
        ObjectFormat::Sha256 => b"sha256",
    });
    fp_input.push(0);
    match &repo.head {
        Head::Commit(oid) => {
            fp_input.extend_from_slice(&repo.object_format.to_hex(oid)?);
        }
        Head::Unborn(branch) => {
            fp_input.extend_from_slice(b"unborn:");
            fp_input.extend_from_slice(branch.as_bytes());
        }
    }
    fp_input.push(0);
    fp_input.extend_from_slice(&ls);
    fp_input.push(0);
    fp_input.extend_from_slice(&encode_status_records(&status, repo.object_format));
    let fingerprint = profile::raw_digest("puyudb/git-capture-fingerprint/v1", &fp_input);
    Ok(Capture {
        repo: repo.clone(),
        index,
        status,
        fingerprint,
    })
}

/// Whether a status record's path lives under the reserved `.puyu` control
/// segment (first path component, byte-exact).
fn is_puyu_control_record(record: &StatusRecord) -> bool {
    let path: &[u8] = match record {
        StatusRecord::Ordinary { path, .. }
        | StatusRecord::Renamed { path, .. }
        | StatusRecord::Unmerged { path, .. }
        | StatusRecord::Untracked(path)
        | StatusRecord::Ignored(path) => path,
        StatusRecord::Header(_) => return false,
    };
    path.split(|b| *b == b'/').next() == Some(b".puyu")
}

/// Canonical re-encoding of parsed status records for the scan fingerprint
/// (deterministic, and unaffected by dropped `.puyu` control records).
fn encode_status_records(records: &[StatusRecord], format: ObjectFormat) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        match r {
            StatusRecord::Header(h) => {
                out.push(b'#');
                out.extend_from_slice(h);
                out.push(0);
            }
            StatusRecord::Ordinary {
                xy,
                path,
                index_oid,
            } => {
                out.push(b'1');
                out.extend_from_slice(xy);
                out.push(0);
                match index_oid {
                    Some(oid) => out.extend_from_slice(&format.to_hex(oid).unwrap_or_default()),
                    None => out.extend_from_slice(&format.zero_oid_hex()),
                }
                out.push(0);
                out.extend_from_slice(path);
                out.push(0);
            }
            StatusRecord::Renamed { xy, path, orig } => {
                out.push(b'2');
                out.extend_from_slice(xy);
                out.push(0);
                out.extend_from_slice(path);
                out.push(0);
                out.extend_from_slice(orig);
                out.push(0);
            }
            StatusRecord::Unmerged { xy, path, stages } => {
                out.push(b'u');
                out.extend_from_slice(xy);
                out.push(0);
                for (stage, oid) in stages {
                    out.push(*stage);
                    out.extend_from_slice(&format.to_hex(oid).unwrap_or_default());
                    out.push(0);
                }
                out.extend_from_slice(path);
                out.push(0);
            }
            StatusRecord::Untracked(path) => {
                out.push(b'?');
                out.extend_from_slice(path);
                out.push(0);
            }
            StatusRecord::Ignored(path) => {
                out.push(b'!');
                out.extend_from_slice(path);
                out.push(0);
            }
        }
    }
    out
}
// --------------------------------------------------------------------------
// Blob reads (P1, §7.2/4: `cat-file --batch`, size caps, typed verification)
// --------------------------------------------------------------------------

/// Streaming `git cat-file --batch -z` session.
///
/// Requests are NUL-delimited hex OIDs on stdin; responses are
/// `<oid> SP <type> SP <size> LF <contents> <delimiter>` (the delimiter byte
/// is consumed by framing on `size`, so binary content is unambiguous).
pub struct BlobReader {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    format: ObjectFormat,
    poisoned: bool,
}

impl BlobReader {
    /// Spawn a batch session in `repo`'s worktree.
    ///
    /// # Errors
    /// [`Error::Environment`] when git cannot be spawned.
    pub fn open(repo: &GitRepo) -> Result<Self, Error> {
        let mut child = Command::new("git")
            .args(with_filter_neutralizers(
                &repo.toplevel,
                &["cat-file", "--batch", "-z"],
            ))
            .current_dir(&repo.toplevel)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| Error::Environment(format!("cannot spawn system git: {err}")))?;
        Ok(Self {
            stdin: child.stdin.take().expect("stdin is piped"),
            stdout: child.stdout.take().expect("stdout is piped"),
            child,
            format: repo.object_format,
            poisoned: false,
        })
    }

    /// Read one blob. The header size is verified against `cap` (and the
    /// profile cap) *before* the payload is read (D1 size caps); a blob above
    /// the cap poisons the session because its payload is not drained.
    ///
    /// # Errors
    /// [`Error::Parse`] on verification failure or oversize; the session is
    /// poisoned after an oversize blob.
    pub fn read_blob(&mut self, oid: GitOid, cap: u64) -> Result<Vec<u8>, Error> {
        if self.poisoned {
            return Err(parse_err(
                "blob session poisoned by a previous oversize blob",
            ));
        }
        let mut request = self.format.to_hex(&oid)?;
        request.push(0);
        self.stdin
            .write_all(&request)
            .and_then(|()| self.stdin.flush())
            .map_err(|err| Error::Environment(format!("writing to cat-file: {err}")))?;
        // Read the header byte-wise up to LF (bounded): read_to_end would
        // block waiting for the 256-byte take to fill while cat-file waits
        // for the next request.
        let mut header = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            match self.stdout.read_exact(&mut byte) {
                Ok(()) => {
                    header.push(byte[0]);
                    if byte[0] == b'\n' || header.len() as u64 > MAX_HEADER {
                        break;
                    }
                }
                Err(err) => {
                    self.poisoned = true;
                    return Err(Error::Environment(format!(
                        "reading cat-file header: {err}"
                    )));
                }
            }
        }
        if header.last() != Some(&b'\n') {
            self.poisoned = true;
            return Err(parse_err("oversize cat-file header"));
        }
        let header = header.trim_ascii_end();
        let parts: Vec<&[u8]> = header.split(|b| *b == b' ').collect();
        // cat-file prints a 2-token "<oid> SP missing" line for absent objects.
        if parts.len() == 2 && parts[1] == b"missing" {
            return Err(parse_err(format!("object {oid} is missing")));
        }
        if parts.len() != 3 {
            self.poisoned = true;
            return Err(parse_err(format!(
                "unexpected cat-file header {}",
                String::from_utf8_lossy(header)
            )));
        }
        // Typed verification: the echoed OID must equal the request.
        let echoed = self.format.parse_oid(parts[0])?;
        if echoed != oid {
            self.poisoned = true;
            return Err(parse_err("cat-file echoed a different oid than requested"));
        }
        if parts[1] != b"blob" {
            self.poisoned = true;
            return Err(parse_err(format!(
                "object {oid} is a {}, expected blob",
                String::from_utf8_lossy(parts[1])
            )));
        }
        let size: u64 = std::str::from_utf8(parts[2])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| parse_err("bad blob size in header"))?;
        let cap = cap.min(profile::MAX_ARTIFACT_RAW_BYTES);
        if size > cap {
            self.poisoned = true;
            return Err(parse_err(format!(
                "blob {oid} is {size} bytes, above the {cap} byte cap"
            )));
        }
        let mut payload = vec![0u8; size as usize];
        self.stdout
            .read_exact(&mut payload)
            .map_err(|err| Error::Environment(format!("reading blob payload: {err}")))?;
        let mut delimiter = [0u8; 1];
        self.stdout
            .read_exact(&mut delimiter)
            .map_err(|err| Error::Environment(format!("reading blob delimiter: {err}")))?;
        Ok(payload)
    }
}

impl Drop for BlobReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sizes of many blobs in one `cat-file --batch-check -z` session, without
/// transferring payloads. Missing objects are an error (typed verification).
///
/// # Errors
/// See [`Error`].
pub fn blob_sizes(repo: &GitRepo, oids: &[GitOid]) -> Result<BTreeMap<GitOid, u64>, Error> {
    let input = nul_input(oids.iter().map(|oid| {
        repo.object_format
            .to_hex(oid)
            .expect("oid matches the repo format")
    }))?;
    let out = git_ok(
        &repo.toplevel,
        &["cat-file", "--batch-check", "-z"],
        Some(&input),
        MAX_PLUMBING_OUTPUT,
    )?;
    let mut sizes = BTreeMap::new();
    for line in out.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let parts: Vec<&[u8]> = line.split(|b| *b == b' ').collect();
        let oid = repo.object_format.parse_oid(parts[0])?;
        // cat-file prints a 2-token "<oid> SP missing" line for absent objects.
        if parts.len() == 2 && parts[1] == b"missing" {
            return Err(parse_err(format!("object {oid} is missing")));
        }
        if parts.len() != 3 {
            return Err(parse_err("unexpected batch-check line"));
        }
        if parts[1] != b"blob" {
            return Err(parse_err(format!(
                "object {oid} is a {}, expected blob",
                String::from_utf8_lossy(parts[1])
            )));
        }
        let size: u64 = std::str::from_utf8(parts[2])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| parse_err("bad size in batch-check line"))?;
        sizes.insert(oid, size);
    }
    Ok(sizes)
}

/// Read one worktree file's bytes, capped (§7.2/4: selected actual worktree
/// bytes). Never follows symlinks: `symlink_metadata` decides, a symlink (or
/// any non-regular file) returns `None` — link targets are read separately
/// via [`read_symlink_target`] (R06 safe paths, §7.3). Returns `None` when
/// the file is absent (e.g. sparse omission).
///
/// # Errors
/// Oversize reads and io failures are [`Error::Environment`]/[`Error::Parse`].
pub fn read_worktree_file(
    repo: &GitRepo,
    rel_path: &[u8],
    cap: u64,
) -> Result<Option<Vec<u8>>, Error> {
    let rel = std::str::from_utf8(rel_path).map_err(|_| parse_err("worktree path is not UTF-8"))?;
    // Hardened precondition: the relative path is spec-§11-validated BEFORE
    // joining, so callers cannot escape the toplevel via `..` or a reserved
    // segment even if they skip the earlier policy gate.
    if let Err(err) = Path::new(rel) {
        return Err(parse_err(format!(
            "worktree path {rel:?} fails spec §11: {err}"
        )));
    }
    let full = repo.toplevel.join(FsPath::new(rel));
    let meta = match std::fs::symlink_metadata(&full) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::Environment(format!("stat {rel}: {err}"))),
    };
    if !meta.is_file() {
        return Ok(None); // symlink, directory, socket, … — never followed/read
    }
    let cap = cap.min(profile::MAX_ARTIFACT_RAW_BYTES);
    if meta.len() > cap {
        return Err(parse_err(format!(
            "worktree file {rel} is {} bytes, above the {cap} byte cap",
            meta.len()
        )));
    }
    std::fs::read(&full)
        .map_err(|err| Error::Environment(format!("reading {rel}: {err}")))
        .map(Some)
}

/// Read a symlink's target bytes (inert link-target content, §7.3), capped.
/// `read_link` never follows the link.
///
/// # Errors
/// Oversize targets and io failures; see [`Error`].
pub fn read_symlink_target(repo: &GitRepo, rel_path: &[u8], cap: u64) -> Result<Vec<u8>, Error> {
    let rel = std::str::from_utf8(rel_path).map_err(|_| parse_err("worktree path is not UTF-8"))?;
    // Same hardening as read_worktree_file: §11-validate before joining.
    if let Err(err) = Path::new(rel) {
        return Err(parse_err(format!(
            "worktree path {rel:?} fails spec §11: {err}"
        )));
    }
    let full = repo.toplevel.join(FsPath::new(rel));
    let target = std::fs::read_link(&full)
        .map_err(|err| Error::Environment(format!("read_link {rel}: {err}")))?;
    #[cfg(unix)]
    let bytes: &[u8] = {
        use std::os::unix::ffi::OsStrExt;
        target.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let bytes: &[u8] = target.to_string_lossy().as_bytes();
    let cap = cap.min(profile::MAX_ARTIFACT_RAW_BYTES);
    if bytes.len() as u64 > cap {
        return Err(parse_err(format!(
            "symlink target of {rel} is {} bytes, above the {cap} byte cap",
            bytes.len()
        )));
    }
    Ok(bytes.to_vec())
}

/// Stat one untracked candidate without following links (R06, §7.3): fills
/// the mode (exec bit / symlink) and `size_hint`. The path is validated
/// against spec §11 BEFORE joining, so a hostile path cannot escape the
/// toplevel via `..`.
fn stat_untracked(repo: &GitRepo, cand: &mut Candidate) -> Result<(), Error> {
    let Ok(rel) = std::str::from_utf8(&cand.path) else {
        return Ok(()); // decide() excludes non-UTF-8 paths
    };
    if Path::new(rel).is_err() {
        return Ok(()); // decide() excludes invalid paths before any read
    }
    let full = repo.toplevel.join(FsPath::new(rel));
    let meta = match std::fs::symlink_metadata(&full) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Environment(format!("stat {rel}: {err}"))),
    };
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        cand.mode = 0o120000;
        cand.size_hint = Some(meta.len());
    } else if file_type.is_file() {
        cand.size_hint = Some(meta.len());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            cand.mode = if meta.permissions().mode() & 0o111 != 0 {
                0o100755
            } else {
                0o100644
            };
        }
        #[cfg(not(unix))]
        {
            cand.mode = 0o100644;
        }
    }
    Ok(())
}

// --------------------------------------------------------------------------
// Conversion-profile classification (P1/P2, §7.3, D5)
// --------------------------------------------------------------------------

/// Conversion profile of one path, from `check-attr` (never executed, D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionProfile {
    /// No conversion attributes: blob bytes are the worktree bytes.
    None,
    /// Git built-in conversion only (`text`/`eol`/`ident`); reproduced in P3
    /// solely through the explicitly tested adapter — captured bytes stay
    /// verbatim here.
    BuiltIn,
    /// User-defined filter or `working-tree-encoding`: never executed (D5);
    /// bytes are captured opaque and the limitation is reported.
    Unsupported,
}

/// Classify the conversion profile of candidate paths via
/// `git check-attr -z --stdin` (read-only plumbing; paths via NUL-delimited
/// stdin, attrs are fixed literals).
///
/// # Errors
/// See [`Error`].
pub fn conversion_profiles(
    repo: &GitRepo,
    paths: &[Vec<u8>],
) -> Result<BTreeMap<Vec<u8>, ConversionProfile>, Error> {
    let input = nul_input(paths.iter().cloned())?;
    let out = git_ok(
        &repo.toplevel,
        &[
            "check-attr",
            "-z",
            "--stdin",
            "filter",
            "text",
            "eol",
            "working-tree-encoding",
            "ident",
            "--end-of-options",
        ],
        Some(&input),
        MAX_PLUMBING_OUTPUT,
    )?;
    // Records are <path>\0<attr>\0<value>\0 triplets.
    let fields: Vec<&[u8]> = out.split(|b| *b == 0).filter(|f| !f.is_empty()).collect();
    let mut attrs: BTreeMap<Vec<u8>, BTreeMap<Vec<u8>, Vec<u8>>> = BTreeMap::new();
    if !fields.len().is_multiple_of(3) {
        return Err(parse_err(
            "check-attr output is not path/attr/value triplets",
        ));
    }
    for triplet in fields.as_chunks::<3>().0 {
        attrs
            .entry(triplet[0].to_vec())
            .or_default()
            .insert(triplet[1].to_vec(), triplet[2].to_vec());
    }
    Ok(attrs
        .into_iter()
        .map(|(path, values)| (path, classify_attrs(&values)))
        .collect())
}

/// Whether the repository config (`core.autocrlf`, `core.eol`) asks git to
/// apply a built-in conversion, so blob and worktree bytes can differ even
/// when no path attribute is set (§7.3: do not label them the same object).
/// Read-only `git config --get`; exit 1 ("unset") is not an error.
///
/// This is a report-level over-approximation (it upgrades every attribute-
/// clean path to BuiltIn), NOT a per-file fact: P3 must not treat it as one.
///
/// # Errors
/// See [`Error`].
pub fn repo_builtin_conversion(repo: &GitRepo) -> Result<bool, Error> {
    let get = |key: &str| -> Result<Option<String>, Error> {
        let out = git_ok_codes(
            &repo.toplevel,
            &["config", "--get", key],
            None,
            MAX_HEADER,
            &[1],
        )?;
        let text = std::str::from_utf8(out.trim_ascii_end())
            .map_err(|_| parse_err("config value is not UTF-8"))?;
        Ok((!text.is_empty()).then(|| text.to_owned()))
    };
    let autocrlf = get("core.autocrlf")?;
    let eol = get("core.eol")?;
    Ok(matches!(autocrlf.as_deref(), Some("true") | Some("input"))
        || matches!(eol.as_deref(), Some("crlf") | Some("native")))
}

fn classify_attrs(values: &BTreeMap<Vec<u8>, Vec<u8>>) -> ConversionProfile {
    let get = |attr: &[u8]| values.get(attr).map(Vec::as_slice);
    let unspecified =
        |v: Option<&[u8]>| v == Some(b"unspecified") || v == Some(b"unset") || v.is_none();
    // A named filter (incl. lfs) or a working-tree-encoding is a user-defined
    // conversion: never executed (D5) — capture opaque, report the limitation.
    if !unspecified(get(b"filter")) || !unspecified(get(b"working-tree-encoding")) {
        return ConversionProfile::Unsupported;
    }
    // Built-in conversions only.
    let text = get(b"text");
    let eol = get(b"eol");
    let ident = get(b"ident");
    if matches!(text, Some(b"set") | Some(b"auto"))
        || matches!(eol, Some(b"crlf") | Some(b"lf") | Some(b"native"))
        || ident == Some(b"set")
    {
        return ConversionProfile::BuiltIn;
    }
    ConversionProfile::None
}

/// Which candidate paths match Git ignore rules. `--no-index` so tracked
/// files are reported too (tracked-even-if-ignored, §7.2/2).
///
/// # Errors
/// See [`Error`]. Note: `check-ignore` takes its paths from NUL-delimited
/// stdin only; `--end-of-options` is not passed because this git version
/// mishandles it after `--stdin` — stdin framing is the protection here.
pub fn ignored_paths(repo: &GitRepo, paths: &[Vec<u8>]) -> Result<BTreeSet<Vec<u8>>, Error> {
    let input = nul_input(paths.iter().cloned())?;
    // Exit 1 is documented as "none of the paths are ignored", not an error.
    let out = git_ok_codes(
        &repo.toplevel,
        &["check-ignore", "-z", "--no-index", "--stdin"],
        Some(&input),
        MAX_PLUMBING_OUTPUT,
        &[1],
    )?;
    Ok(out
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .map(<[u8]>::to_vec)
        .collect())
}

// --------------------------------------------------------------------------
// Inclusion/exclusion policy + preview report (P2, §7.2/2–4, R06, spec §11)
// --------------------------------------------------------------------------

/// Where a candidate path came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// In the index (tracked) — a candidate even when ignored (§7.2/2).
    Tracked,
    /// Untracked worktree file (from `status` `?` records).
    Untracked,
}

/// Content classification for an included entry (spec §8.2 kind tags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentClass {
    /// UTF-8 text, kind 0 (seed profile `dt-text-v1`; seeds stay lazy, D3).
    Text,
    /// Opaque bytes, kind 1.
    Opaque,
    /// Symlink target bytes, kind 2 (inert).
    SymlinkTarget,
}

/// Why a candidate is excluded. Exclusions are always reported — never
/// treated as deletions (§7.2/2, §7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// Untracked and matched by Git ignore sources.
    IgnoredByGit,
    /// Puyu sensitive deny rule (§7.2/2, R06): excluded even when tracked;
    /// sets `ImportPreview::incomplete_capture`.
    SensitiveDeny,
    /// Gitlink/submodule (mode 0o160000): typed entry, no nested import (§7.3).
    Submodule,
    /// Intent-to-add: no staged content (empty-blob index entry).
    IntentToAdd,
    /// Path bytes are not valid UTF-8 — unrepresentable under spec §11.
    NonUtf8Path,
    /// Fails spec §11 validation (reserved `.git`/`.puyu` segment, dot
    /// components, caps, …).
    InvalidPath,
    /// Unmerged index stages 1–3: kept as Git conflict metadata, never one
    /// invented clean baseline (§7.2/3).
    Unmerged,
    /// Above the profile artifact cap.
    Oversize,
}

/// One candidate in the import preview.
#[derive(Debug, Clone)]
pub struct PreviewEntry {
    /// Validated spec §11 path text.
    pub path: String,
    /// Tracked (index) or untracked (worktree).
    pub source: Source,
    /// Mode tag (spec §7.1 mode tags).
    pub mode: ModeTag,
    /// Index blob OID (stage 0), if any.
    pub git_oid: Option<GitOid>,
    /// Classification when included.
    pub class: Option<ContentClass>,
    /// Exclusion reason when not included.
    pub excluded: Option<Exclusion>,
    /// Matches a Git ignore rule (reported even for tracked files).
    pub ignored_by_git: bool,
    /// Skip-worktree / sparse omission flag (not a deletion, §7.3).
    pub skip_worktree: bool,
    /// Unmerged conflict stages (stage number, OID).
    pub unmerged_stages: Vec<(u8, GitOid)>,
    /// Worktree bytes differ from the index (status `1`/`2`/`u` Y ≠ `.`).
    pub worktree_dirty: bool,
    /// Conversion profile (never executed, D5).
    pub conversion: ConversionProfile,
    /// Blob/worktree size when known.
    pub size: Option<u64>,
}

/// The import preview report (P2): no publication, pure data (§7.2/5).
#[derive(Debug, Clone)]
pub struct ImportPreview {
    /// Discovered repository.
    pub repo: GitRepo,
    /// Starting status fingerprint of the captured scan (D4 key material).
    pub fingerprint: [u8; 32],
    /// Entries sorted by raw path bytes.
    pub entries: Vec<PreviewEntry>,
    /// A sensitive deny rule excluded a path: capture is intentionally
    /// incomplete and must be reported, never read as deletion (§7.2/2).
    pub incomplete_capture: bool,
    /// No included files — an allowed empty-cap import (D3, pinned by test).
    pub empty_cap: bool,
    /// Reported limitations (unsupported conversions, submodules, sparse
    /// omissions, non-UTF-8/invalid paths, oversize, unmerged conflicts).
    pub limitations: Vec<String>,
}

/// Puyu-side sensitive deny rules (§7.2/2, R06 least disclosure). Byte-exact
/// and case-sensitive per spec §11 (no case folding).
fn sensitive_deny(path: &str) -> bool {
    let mut components = path.split('/');
    let Some(first) = components.next() else {
        return false;
    };
    if matches!(first, ".ssh" | ".aws" | ".gnupg") {
        return true;
    }
    let file = path.rsplit('/').next().unwrap_or(path);
    matches!(
        file,
        ".env" | ".env.local" | "id_rsa" | "id_ed25519" | "id_dsa"
    ) || file.ends_with(".pem")
        || file.ends_with(".key")
        || file.ends_with(".p12")
}

/// Raw git mode → spec mode tag.
fn mode_tag(mode: u32) -> Result<ModeTag, Error> {
    match mode {
        0o100644 => Ok(ModeTag::Regular),
        0o100755 => Ok(ModeTag::Executable),
        0o120000 => Ok(ModeTag::Symlink),
        _ => Err(parse_err(format!("unsupported index mode {mode:o}"))),
    }
}

/// One candidate before policy, built purely from the capture (plus a
/// worktree stat for untracked entries — see [`stat_untracked`]).
struct Candidate {
    path: Vec<u8>,
    source: Source,
    mode: u32,
    oid: Option<GitOid>,
    /// Worktree size for untracked candidates (from `symlink_metadata`).
    size_hint: Option<u64>,
    skip_worktree: bool,
    intent_to_add: bool,
    /// Came from a status `u` record (a real conflict marker).
    from_unmerged: bool,
    unmerged_stages: Vec<(u8, GitOid)>,
    worktree_dirty: bool,
}

fn build_candidates(capture: &Capture) -> Vec<Candidate> {
    let format = capture.repo.object_format;
    // Intent-to-add: a stage-0 index entry pointing at the empty blob whose
    // status `1` record has the ZERO index hash (hI = 0, XY ".A"). A genuinely
    // staged empty file has hI = the empty-blob OID and stays included
    // (spec §8.2 empty baselines are legitimate).
    let ita: BTreeSet<&[u8]> = capture
        .status
        .iter()
        .filter_map(|r| match r {
            StatusRecord::Ordinary {
                path, index_oid, ..
            } if index_oid.is_none() => Some(path.as_slice()),
            _ => None,
        })
        .collect();
    let empty = format.empty_blob();
    // Worktree-dirty paths: any `1`/`2`/`u` record whose Y is not `.`.
    let dirty: BTreeSet<&[u8]> = capture
        .status
        .iter()
        .filter_map(|r| match r {
            StatusRecord::Ordinary { xy, path, .. }
            | StatusRecord::Renamed { xy, path, .. }
            | StatusRecord::Unmerged { xy, path, .. } => (xy[1] != b'.').then_some(path.as_slice()),
            _ => None,
        })
        .collect();
    // Unmerged stages per path, from the index (stage > 0).
    let mut unmerged: BTreeMap<&[u8], Vec<(u8, GitOid)>> = BTreeMap::new();
    for e in &capture.index {
        if e.stage > 0 {
            unmerged
                .entry(e.path.as_slice())
                .or_default()
                .push((e.stage, e.oid));
        }
    }
    let mut by_path: BTreeMap<Vec<u8>, Candidate> = BTreeMap::new();
    for e in &capture.index {
        if e.stage > 0 {
            continue; // conflict stages are metadata on the stage-0 path
        }
        let intent_to_add = e.oid == empty && ita.contains(e.path.as_slice());
        by_path.insert(
            e.path.clone(),
            Candidate {
                path: e.path.clone(),
                source: Source::Tracked,
                mode: e.mode,
                oid: Some(e.oid),
                size_hint: None,
                skip_worktree: e.skip_worktree(),
                intent_to_add,
                from_unmerged: false,
                unmerged_stages: unmerged.get(e.path.as_slice()).cloned().unwrap_or_default(),
                worktree_dirty: dirty.contains(e.path.as_slice()),
            },
        );
    }
    for r in &capture.status {
        match r {
            StatusRecord::Untracked(path) | StatusRecord::Ignored(path) => {
                by_path.entry(path.clone()).or_insert_with(|| Candidate {
                    path: path.clone(),
                    source: Source::Untracked,
                    mode: 0o100644,
                    oid: None,
                    size_hint: None,
                    skip_worktree: false,
                    intent_to_add: false,
                    from_unmerged: false,
                    unmerged_stages: Vec::new(),
                    worktree_dirty: false,
                });
            }
            // Gate A finding 8: a real conflict emits ls-files stages 1/2/3
            // with NO stage-0 entry, plus one `u` record. The conflicted path
            // must still become a candidate so it is reported as Unmerged
            // conflict metadata — never silently absent from the preview.
            // A degenerate `u` record (no parsed stages AND no index stages)
            // is still excluded as Unmerged — fail safe, never a clean file.
            StatusRecord::Unmerged { path, stages, .. } => {
                let stages = if stages.is_empty() {
                    unmerged.get(path.as_slice()).cloned().unwrap_or_default()
                } else {
                    stages.clone()
                };
                by_path.entry(path.clone()).or_insert_with(|| Candidate {
                    path: path.clone(),
                    source: Source::Tracked,
                    mode: 0o100644,
                    oid: None,
                    size_hint: None,
                    skip_worktree: false,
                    intent_to_add: false,
                    from_unmerged: true,
                    unmerged_stages: stages,
                    worktree_dirty: dirty.contains(path.as_slice()),
                });
            }
            _ => {}
        }
    }
    by_path.into_values().collect()
}

/// Policy checks that run BEFORE any content is read (§11 representability,
/// sensitive deny, untracked-ignored, unmerged, gitlink, intent-to-add).
/// Returns the exclusion and its report note. Pure: no content is touched.
fn early_exclusion(cand: &Candidate, ignored: &BTreeSet<Vec<u8>>) -> Option<(Exclusion, String)> {
    // §11: path representability first — non-UTF-8 and invalid paths are
    // explicit exclusions, never lossy substitutions (§5.6/§11).
    let Ok(text) = std::str::from_utf8(&cand.path) else {
        return Some((
            Exclusion::NonUtf8Path,
            "non-UTF-8 path excluded (spec §11)".into(),
        ));
    };
    if let Err(err) = Path::new(text) {
        return Some((
            Exclusion::InvalidPath,
            format!("path {text:?} excluded: {err}"),
        ));
    }
    if sensitive_deny(text) {
        return Some((
            Exclusion::SensitiveDeny,
            format!("sensitive path {text} denied by Puyu policy; capture is incomplete"),
        ));
    }
    if cand.source == Source::Untracked && ignored.contains(&cand.path) {
        return Some((
            Exclusion::IgnoredByGit,
            format!("untracked path {text} excluded by Git ignore rules"),
        ));
    }
    if cand.from_unmerged || !cand.unmerged_stages.is_empty() {
        return Some((
            Exclusion::Unmerged,
            format!(
                "path {text} has unmerged stages {:?}; kept as conflict metadata",
                cand.unmerged_stages
                    .iter()
                    .map(|(s, _)| *s)
                    .collect::<Vec<_>>()
            ),
        ));
    }
    if cand.mode == 0o160000 {
        return Some((
            Exclusion::Submodule,
            format!("gitlink/submodule {text} is a typed entry; no nested import"),
        ));
    }
    if cand.intent_to_add {
        return Some((
            Exclusion::IntentToAdd,
            format!("intent-to-add path {text} has no staged content"),
        ));
    }
    None
}

/// Pure policy decision for one candidate. `content` is the already-read,
/// capped baseline bytes (blob for tracked, worktree file or symlink target
/// for untracked) when classification needed them.
fn decide(
    cand: &Candidate,
    ignored: &BTreeSet<Vec<u8>>,
    profiles: &BTreeMap<Vec<u8>, ConversionProfile>,
    sizes: &BTreeMap<GitOid, u64>,
    content: Option<&[u8]>,
    limitations: &mut Vec<String>,
) -> PreviewEntry {
    let conversion = profiles
        .get(&cand.path)
        .copied()
        .unwrap_or(ConversionProfile::None);
    let ignored_by_git = ignored.contains(&cand.path);
    let mut entry = PreviewEntry {
        path: String::new(),
        source: cand.source,
        mode: ModeTag::Regular,
        git_oid: cand.oid,
        class: None,
        excluded: None,
        ignored_by_git,
        skip_worktree: cand.skip_worktree,
        unmerged_stages: cand.unmerged_stages.clone(),
        worktree_dirty: cand.worktree_dirty,
        conversion,
        size: cand
            .oid
            .and_then(|o| sizes.get(&o).copied())
            .or(cand.size_hint),
    };
    if let Some((excl, note)) = early_exclusion(cand, ignored) {
        // Keep the path text when it is representable, for the report.
        if let Ok(text) = std::str::from_utf8(&cand.path)
            && Path::new(text).is_ok()
        {
            entry.path = text.to_owned();
        }
        limitations.push(note);
        entry.excluded = Some(excl);
        return entry;
    }
    let text = std::str::from_utf8(&cand.path).expect("early_exclusion passed UTF-8");
    entry.path = text.to_owned();
    let mut fail = |entry: &mut PreviewEntry, excl: Exclusion, note: String| {
        limitations.push(note);
        entry.excluded = Some(excl);
    };
    let mode = match mode_tag(cand.mode) {
        Ok(m) => m,
        Err(err) => {
            fail(&mut entry, Exclusion::InvalidPath, format!("{err}"));
            return entry;
        }
    };
    entry.mode = mode;
    if let Some(size) = entry.size
        && size > profile::MAX_ARTIFACT_RAW_BYTES
    {
        fail(
            &mut entry,
            Exclusion::Oversize,
            format!("path {text} is {size} bytes, above the profile cap"),
        );
        return entry;
    }
    // Classification (spec §8.2 kinds): symlink target, else conversion and
    // size/UTF-8 decide text vs opaque. Unsupported conversions capture
    // opaque and report the limitation (D5).
    entry.class = Some(match mode {
        ModeTag::Symlink => ContentClass::SymlinkTarget,
        _ => {
            if conversion == ConversionProfile::Unsupported {
                limitations.push(format!(
                    "path {text} has an unsupported conversion profile; captured opaque (D5)"
                ));
                ContentClass::Opaque
            } else if entry
                .size
                .is_some_and(|s| s > profile::MAX_TEXT_EPOCH_BYTES as u64)
            {
                limitations.push(format!(
                    "path {text} exceeds MAX_TEXT_EPOCH_BYTES; classified opaque"
                ));
                ContentClass::Opaque
            } else {
                match content {
                    Some(bytes) if std::str::from_utf8(bytes).is_ok() => ContentClass::Text,
                    _ => ContentClass::Opaque,
                }
            }
        }
    });
    if cand.skip_worktree {
        limitations.push(format!(
            "path {text} is a sparse omission (skip-worktree); not a deletion"
        ));
    }
    entry
}

/// Build the import preview for a captured scan (P2). Runs read-only plumbing
/// for ignore rules, conversion attributes, and blob sizes; reads baseline
/// bytes only for candidates that need UTF-8 classification, capped at
/// `MAX_TEXT_EPOCH_BYTES`. Performs no publication.
///
/// # Errors
/// See [`Error`].
pub fn preview(repo: &GitRepo, capture: &Capture) -> Result<ImportPreview, Error> {
    let mut candidates = build_candidates(capture);
    // Untracked entries: stat without following links (exec bit, symlink,
    // size) before any read (R06, §7.3).
    for cand in &mut candidates {
        if cand.source == Source::Untracked {
            stat_untracked(repo, cand)?;
        }
    }
    let paths: Vec<Vec<u8>> = candidates.iter().map(|c| c.path.clone()).collect();
    let ignored = ignored_paths(repo, &paths)?;
    let mut profiles = conversion_profiles(repo, &paths)?;
    let repo_config_conversion = repo_builtin_conversion(repo)?;
    if repo_config_conversion {
        // core.autocrlf/core.eol apply a built-in conversion even without
        // path attributes; bytes are captured verbatim and the difference is
        // reported (§7.3: blob and worktree bytes are not the same object).
        for profile in profiles.values_mut() {
            if *profile == ConversionProfile::None {
                *profile = ConversionProfile::BuiltIn;
            }
        }
    }
    // Gitlinks (mode 0o160000) point at commit OIDs that need not exist in
    // this repository's object store; they are excluded as Submodule before
    // classification, so they never enter the size query.
    let oids: Vec<GitOid> = candidates
        .iter()
        .filter(|c| c.mode != 0o160000)
        .filter_map(|c| c.oid)
        .collect();
    let sizes = blob_sizes(repo, &oids)?;
    let mut reader = BlobReader::open(repo)?;
    let mut limitations = Vec::new();
    if repo_config_conversion {
        limitations.push(
            "repo config core.autocrlf/core.eol requests a built-in conversion; \
             blob and worktree bytes may differ and are captured verbatim (§7.3)"
                .into(),
        );
    }
    let mut entries = Vec::new();
    let mut incomplete_capture = false;
    let read_cap = profile::MAX_TEXT_EPOCH_BYTES as u64;
    for cand in &candidates {
        // Policy first: never read content for an excluded entry (finding: no
        // bytes fetched for sensitive-denied or otherwise excluded paths).
        let early = early_exclusion(cand, &ignored);
        let size = cand
            .oid
            .and_then(|o| sizes.get(&o).copied())
            .or(cand.size_hint);
        // Read baseline bytes only when classification needs them (tracked:
        // blob; untracked: worktree file or symlink target), and only within
        // the text-epoch cap — larger entries classify opaque by size alone,
        // never by aborting the preview.
        let content: Option<Vec<u8>> = if early.is_none() && size.is_none_or(|s| s <= read_cap) {
            match cand.oid {
                Some(oid) => Some(reader.read_blob(oid, read_cap)?),
                None if cand.mode == 0o120000 => {
                    Some(read_symlink_target(repo, &cand.path, read_cap)?)
                }
                None => {
                    let bytes = read_worktree_file(repo, &cand.path, read_cap)?;
                    if bytes.is_none() && cand.size_hint.is_some() {
                        // Scan race: the file vanished between status and
                        // stat/read. Report it instead of silently classifying
                        // opaque (§7.2/6 recheck reconciles later).
                        limitations.push(format!(
                            "path {} vanished during the scan (status/stat race); classified opaque",
                            String::from_utf8_lossy(&cand.path)
                        ));
                    }
                    bytes
                }
            }
        } else {
            None
        };
        let entry = decide(
            cand,
            &ignored,
            &profiles,
            &sizes,
            content.as_deref(),
            &mut limitations,
        );
        if entry.excluded == Some(Exclusion::SensitiveDeny) {
            incomplete_capture = true;
        }
        entries.push(entry);
    }
    entries.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let empty_cap = entries.iter().all(|e| e.excluded.is_some());
    Ok(ImportPreview {
        repo: repo.clone(),
        fingerprint: capture.fingerprint,
        entries,
        incomplete_capture,
        empty_cap,
        limitations,
    })
}
// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sha1_oid(byte: u8) -> GitOid {
        GitOid::Sha1([byte; 20])
    }

    #[test]
    fn oid_parsing_is_typed_and_verified() {
        let hex40 = hex(&[0xabu8; 20]);
        assert_eq!(
            ObjectFormat::Sha1.parse_oid(&hex40).unwrap(),
            GitOid::Sha1([0xab; 20])
        );
        let hex64 = hex(&[0xcd; 32]);
        assert_eq!(
            ObjectFormat::Sha256.parse_oid(&hex64).unwrap(),
            GitOid::Sha256([0xcd; 32])
        );
        // Wrong length for the format fails; a sha1 hex string is not a sha256 oid.
        assert!(ObjectFormat::Sha1.parse_oid(&hex64).is_err());
        assert!(ObjectFormat::Sha256.parse_oid(&hex40).is_err());
        assert!(ObjectFormat::Sha1.parse_oid(b"zz").is_err());
        assert_eq!(ObjectFormat::Sha1.oid_hex_len(), 40);
        assert_eq!(ObjectFormat::Sha256.oid_hex_len(), 64);
        // Empty-blob constants match git's well-known values.
        assert_eq!(
            ObjectFormat::Sha1
                .to_hex(&ObjectFormat::Sha1.empty_blob())
                .unwrap(),
            b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391".to_vec()
        );
        assert_eq!(
            ObjectFormat::Sha256
                .to_hex(&ObjectFormat::Sha256.empty_blob())
                .unwrap(),
            b"473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813".to_vec()
        );
    }

    #[test]
    fn parses_ls_files_records() {
        let oid = sha1_oid(1);
        let oid_hex = ObjectFormat::Sha1.to_hex(&oid).unwrap();
        let mut raw = Vec::new();
        raw.extend_from_slice(b"H 100644 ");
        raw.extend_from_slice(&oid_hex);
        raw.extend_from_slice(b" 0\ta.txt\0S 120000 ");
        raw.extend_from_slice(&oid_hex);
        raw.extend_from_slice(b" 0\tlink\0M 100644 ");
        raw.extend_from_slice(&oid_hex);
        raw.extend_from_slice(b" 2\tmerged.txt\0");
        let entries = parse_ls_files(&raw, ObjectFormat::Sha1).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, b"a.txt");
        assert_eq!(entries[0].mode, 0o100644);
        assert_eq!(entries[0].stage, 0);
        assert!(!entries[0].skip_worktree());
        assert!(entries[1].skip_worktree());
        assert_eq!(entries[1].mode, 0o120000);
        assert_eq!(entries[2].stage, 2);
        assert!(parse_ls_files(b"100644 no-tab\0", ObjectFormat::Sha1).is_err());
    }

    #[test]
    fn parses_status_records() {
        let oid = sha1_oid(2);
        let oid_hex = ObjectFormat::Sha1.to_hex(&oid).unwrap();
        let zero = ObjectFormat::Sha1.zero_oid_hex();
        let mut raw = Vec::new();
        raw.extend_from_slice(b"# branch.head main\0");
        raw.extend_from_slice(b"1 AM N... 000000 100644 100644 ");
        raw.extend_from_slice(&zero);
        raw.push(b' ');
        raw.extend_from_slice(&oid_hex);
        raw.extend_from_slice(b" a.txt\0");
        raw.extend_from_slice(b"2 RM N... 100644 100644 100644 ");
        raw.extend_from_slice(&oid_hex);
        raw.push(b' ');
        raw.extend_from_slice(&oid_hex);
        raw.extend_from_slice(b" R100 renamed.txt\0a.txt\0");
        raw.extend_from_slice(b"u AA N... 000000 100644 100644 100644 ");
        raw.extend_from_slice(&zero);
        raw.push(b' ');
        raw.extend_from_slice(&oid_hex);
        raw.push(b' ');
        raw.extend_from_slice(&oid_hex);
        raw.extend_from_slice(b" merged.txt\0? untracked.txt\0");
        let records = parse_status(&raw, ObjectFormat::Sha1).unwrap();
        assert_eq!(records.len(), 5);
        assert_eq!(
            records[0],
            StatusRecord::Header(b"branch.head main".to_vec())
        );
        assert_eq!(
            records[1],
            StatusRecord::Ordinary {
                xy: *b"AM",
                path: b"a.txt".to_vec(),
                index_oid: Some(oid)
            }
        );
        assert_eq!(
            records[2],
            StatusRecord::Renamed {
                xy: *b"RM",
                path: b"renamed.txt".to_vec(),
                orig: b"a.txt".to_vec()
            }
        );
        match &records[3] {
            StatusRecord::Unmerged { path, stages, .. } => {
                assert_eq!(path, b"merged.txt");
                assert_eq!(stages, &vec![(2, oid), (3, oid)]);
            }
            other => panic!("expected unmerged, got {other:?}"),
        }
        assert_eq!(
            records[4],
            StatusRecord::Untracked(b"untracked.txt".to_vec())
        );
    }

    #[test]
    fn classifies_conversion_profiles() {
        let mut values = BTreeMap::new();
        values.insert(b"filter".to_vec(), b"unspecified".to_vec());
        values.insert(b"text".to_vec(), b"unspecified".to_vec());
        values.insert(b"eol".to_vec(), b"unspecified".to_vec());
        values.insert(b"working-tree-encoding".to_vec(), b"unspecified".to_vec());
        values.insert(b"ident".to_vec(), b"unspecified".to_vec());
        assert_eq!(classify_attrs(&values), ConversionProfile::None);
        values.insert(b"eol".to_vec(), b"crlf".to_vec());
        assert_eq!(classify_attrs(&values), ConversionProfile::BuiltIn);
        values.insert(b"filter".to_vec(), b"lfs".to_vec());
        assert_eq!(classify_attrs(&values), ConversionProfile::Unsupported);
        let mut values = BTreeMap::new();
        values.insert(b"working-tree-encoding".to_vec(), b"UTF-16".to_vec());
        assert_eq!(classify_attrs(&values), ConversionProfile::Unsupported);
    }

    #[test]
    fn sensitive_deny_rules() {
        assert!(sensitive_deny(".env"));
        assert!(sensitive_deny("deploy/.env.local"));
        assert!(sensitive_deny(".ssh/id_rsa"));
        assert!(sensitive_deny("certs/server.pem"));
        assert!(sensitive_deny(".aws/credentials"));
        assert!(!sensitive_deny("src/main.rs"));
        assert!(!sensitive_deny("env.txt")); // exact names only, no folding (§11)
    }

    fn candidate(path: &str, source: Source) -> Candidate {
        Candidate {
            path: path.as_bytes().to_vec(),
            source,
            mode: 0o100644,
            oid: Some(sha1_oid(9)),
            size_hint: None,
            skip_worktree: false,
            intent_to_add: false,
            from_unmerged: false,
            unmerged_stages: Vec::new(),
            worktree_dirty: false,
        }
    }

    fn test_repo() -> GitRepo {
        GitRepo {
            toplevel: PathBuf::from("/tmp/x"),
            git_dir: PathBuf::from("/tmp/x/.git"),
            common_dir: PathBuf::from("/tmp/x/.git"),
            gitfile: false,
            object_format: ObjectFormat::Sha1,
            head: Head::Unborn("main".into()),
        }
    }

    /// Gate A finding 1: a genuinely staged EMPTY file (index OID = empty
    /// blob, status hI = empty-blob OID) must NOT be flagged intent-to-add;
    /// real intent-to-add (status hI = zero OID) must be.
    #[test]
    fn intent_to_add_vs_staged_empty_file() {
        let empty = ObjectFormat::Sha1.empty_blob();
        let empty_hex = ObjectFormat::Sha1.to_hex(&empty).unwrap();
        let zero = ObjectFormat::Sha1.zero_oid_hex();
        let mk_status = |staged_empty: bool| {
            let mut raw = Vec::new();
            if staged_empty {
                // `git add empty.txt`: hI = empty-blob OID.
                raw.extend_from_slice(b"1 A. N... 000000 100644 100644 ");
                raw.extend_from_slice(&zero);
                raw.push(b' ');
                raw.extend_from_slice(&empty_hex);
                raw.extend_from_slice(b" empty.txt\0");
            } else {
                // `git add -N ita.txt`: hI = zero OID.
                raw.extend_from_slice(b"1 .A N... 000000 000000 100644 ");
                raw.extend_from_slice(&zero);
                raw.push(b' ');
                raw.extend_from_slice(&zero);
                raw.extend_from_slice(b" ita.txt\0");
            }
            parse_status(&raw, ObjectFormat::Sha1).unwrap()
        };
        let mk_index = |name: &str| {
            let mut raw = Vec::new();
            raw.extend_from_slice(b"H 100644 ");
            raw.extend_from_slice(&empty_hex);
            raw.extend_from_slice(b" 0\t");
            raw.extend_from_slice(name.as_bytes());
            raw.push(0);
            parse_ls_files(&raw, ObjectFormat::Sha1).unwrap()
        };
        // Staged empty file: included, not ITA.
        let capture = Capture {
            repo: test_repo(),
            index: mk_index("empty.txt"),
            status: mk_status(true),
            fingerprint: [0; 32],
        };
        let cands = build_candidates(&capture);
        assert_eq!(cands.len(), 1);
        assert!(!cands[0].intent_to_add, "staged empty file must not be ITA");
        // Intent-to-add: flagged.
        let capture = Capture {
            repo: test_repo(),
            index: mk_index("ita.txt"),
            status: mk_status(false),
            fingerprint: [0; 32],
        };
        let cands = build_candidates(&capture);
        assert_eq!(cands.len(), 1);
        assert!(cands[0].intent_to_add, "zero-hI record must be ITA");
        let mut limitations = Vec::new();
        let e = decide(
            &cands[0],
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            &mut limitations,
        );
        assert_eq!(e.excluded, Some(Exclusion::IntentToAdd));
    }

    /// Gate A re-run finding 8: a real conflict has ls-files stages 1/2/3 and
    /// NO stage-0 entry, plus one status `u` record. The conflicted path must
    /// still appear in the preview as Unmerged conflict metadata.
    #[test]
    fn unmerged_conflict_is_visible_in_candidates() {
        let oid1 = sha1_oid(1);
        let oid2 = sha1_oid(2);
        let oid3 = sha1_oid(3);
        let hex = |o: GitOid| ObjectFormat::Sha1.to_hex(&o).unwrap();
        let zero = ObjectFormat::Sha1.zero_oid_hex();
        // Index: only stages 1,2,3 for merged.txt (no stage 0).
        let mut ls = Vec::new();
        for (stage, oid) in [(1u8, oid1), (2, oid2), (3, oid3)] {
            ls.extend_from_slice(b"M 100644 ");
            ls.extend_from_slice(&hex(oid));
            ls.push(b' ');
            ls.push(b'0' + stage);
            ls.extend_from_slice(b"\tmerged.txt\0");
        }
        // Status: one `u` record (11 fields).
        let mut st = Vec::new();
        st.extend_from_slice(b"u AA N... 000000 100644 100644 100644 ");
        st.extend_from_slice(&zero);
        st.push(b' ');
        st.extend_from_slice(&hex(oid2));
        st.push(b' ');
        st.extend_from_slice(&hex(oid3));
        st.extend_from_slice(b" merged.txt\0");
        let capture = Capture {
            repo: test_repo(),
            index: parse_ls_files(&ls, ObjectFormat::Sha1).unwrap(),
            status: parse_status(&st, ObjectFormat::Sha1).unwrap(),
            fingerprint: [0; 32],
        };
        let cands = build_candidates(&capture);
        assert_eq!(cands.len(), 1, "conflicted path must be a candidate");
        assert_eq!(cands[0].path, b"merged.txt");
        assert_eq!(
            cands[0].unmerged_stages,
            vec![(2, oid2), (3, oid3)],
            "stage 1 (absent) is not listed"
        );
        let mut limitations = Vec::new();
        let e = decide(
            &cands[0],
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            &mut limitations,
        );
        assert_eq!(e.excluded, Some(Exclusion::Unmerged));
        assert_eq!(e.unmerged_stages.len(), 2);
        assert!(limitations.iter().any(|l| l.contains("conflict metadata")));
    }

    /// Carry-forward: a degenerate `u` record (no stages anywhere) is still
    /// excluded as Unmerged — never silently treated as a clean file.
    #[test]
    fn degenerate_unmerged_record_is_excluded() {
        let zero = ObjectFormat::Sha1.zero_oid_hex();
        let mut st = Vec::new();
        st.extend_from_slice(b"u AA N... 000000 100644 100644 100644 ");
        st.extend_from_slice(&zero);
        st.push(b' ');
        st.extend_from_slice(&zero);
        st.push(b' ');
        st.extend_from_slice(&zero);
        st.extend_from_slice(b" gone.txt\0");
        let capture = Capture {
            repo: test_repo(),
            index: Vec::new(),
            status: parse_status(&st, ObjectFormat::Sha1).unwrap(),
            fingerprint: [0; 32],
        };
        let cands = build_candidates(&capture);
        assert_eq!(cands.len(), 1);
        assert!(cands[0].from_unmerged);
        let mut limitations = Vec::new();
        let e = decide(
            &cands[0],
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            &mut limitations,
        );
        assert_eq!(e.excluded, Some(Exclusion::Unmerged));
    }

    /// The reserved `.puyu` control segment is dropped from status records
    /// (PuyuDB's own space dir is never user content).
    #[test]
    fn puyu_control_segment_is_filtered_from_scan() {
        assert!(is_puyu_control_record(&StatusRecord::Untracked(
            b".puyu/manifest.toml".to_vec()
        )));
        assert!(is_puyu_control_record(&StatusRecord::Untracked(
            b".puyu".to_vec()
        )));
        assert!(!is_puyu_control_record(&StatusRecord::Untracked(
            b".puyux/a".to_vec()
        )));
        assert!(!is_puyu_control_record(&StatusRecord::Untracked(
            b"src/.puyu/a".to_vec()
        )));
        assert!(!is_puyu_control_record(&StatusRecord::Header(
            b"branch.head main".to_vec()
        )));
    }

    /// Gate A finding 2: an oversize untracked entry is excluded/classified by
    /// size alone — decide never needs its bytes and never errors.
    #[test]
    fn oversize_untracked_entry_is_reported_not_fatal() {
        let mut c = candidate("video.bin", Source::Untracked);
        c.oid = None;
        c.size_hint = Some(profile::MAX_TEXT_EPOCH_BYTES as u64 + 1);
        let mut limitations = Vec::new();
        let e = decide(
            &c,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None, // no content read
            &mut limitations,
        );
        assert!(e.excluded.is_none());
        assert_eq!(e.class, Some(ContentClass::Opaque));
        assert_eq!(e.size, Some(profile::MAX_TEXT_EPOCH_BYTES as u64 + 1));
        assert!(
            limitations
                .iter()
                .any(|l| l.contains("MAX_TEXT_EPOCH_BYTES"))
        );
        // Above the 1 GiB profile cap: excluded as oversize.
        c.size_hint = Some(profile::MAX_ARTIFACT_RAW_BYTES + 1);
        let e = decide(
            &c,
            &BTreeSet::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            &mut limitations,
        );
        assert_eq!(e.excluded, Some(Exclusion::Oversize));
    }

    #[test]
    fn policy_exclusions_and_classification() {
        let ignored = BTreeSet::new();
        let profiles = BTreeMap::new();
        let mut sizes = BTreeMap::new();
        sizes.insert(sha1_oid(9), 5u64);
        let mut limitations = Vec::new();
        // Text when content is UTF-8.
        let e = decide(
            &candidate("a.txt", Source::Tracked),
            &ignored,
            &profiles,
            &sizes,
            Some(b"hello"),
            &mut limitations,
        );
        assert_eq!(e.class, Some(ContentClass::Text));
        assert!(e.excluded.is_none());
        assert_eq!(e.path, "a.txt");
        // Opaque when content is not UTF-8.
        let e = decide(
            &candidate("b.bin", Source::Tracked),
            &ignored,
            &profiles,
            &sizes,
            Some(&[0u8, 159, 146, 150]),
            &mut limitations,
        );
        assert_eq!(e.class, Some(ContentClass::Opaque));
        // Sensitive deny is reported, not silent.
        let e = decide(
            &candidate(".env", Source::Tracked),
            &ignored,
            &profiles,
            &sizes,
            None,
            &mut limitations,
        );
        assert_eq!(e.excluded, Some(Exclusion::SensitiveDeny));
        // Non-UTF-8 path is an explicit exclusion.
        let mut c = candidate("a.txt", Source::Tracked);
        c.path = vec![0xff, 0xfe];
        let e = decide(&c, &ignored, &profiles, &sizes, None, &mut limitations);
        assert_eq!(e.excluded, Some(Exclusion::NonUtf8Path));
        // Reserved segment fails §11.
        let e = decide(
            &candidate("x/.git/config", Source::Tracked),
            &ignored,
            &profiles,
            &sizes,
            None,
            &mut limitations,
        );
        assert_eq!(e.excluded, Some(Exclusion::InvalidPath));
        // Unmerged stages become conflict metadata.
        let mut c = candidate("c.txt", Source::Tracked);
        c.unmerged_stages = vec![(1, sha1_oid(1)), (2, sha1_oid(2)), (3, sha1_oid(3))];
        let e = decide(&c, &ignored, &profiles, &sizes, None, &mut limitations);
        assert_eq!(e.excluded, Some(Exclusion::Unmerged));
        assert_eq!(e.unmerged_stages.len(), 3);
        // Gitlink is a submodule entry.
        let mut c = candidate("sub", Source::Tracked);
        c.mode = 0o160000;
        let e = decide(&c, &ignored, &profiles, &sizes, None, &mut limitations);
        assert_eq!(e.excluded, Some(Exclusion::Submodule));
        // Symlink mode classifies as symlink target.
        let mut c = candidate("link", Source::Tracked);
        c.mode = 0o120000;
        let e = decide(
            &c,
            &ignored,
            &profiles,
            &sizes,
            Some(b"a.txt"),
            &mut limitations,
        );
        assert_eq!(e.class, Some(ContentClass::SymlinkTarget));
        assert_eq!(e.mode, ModeTag::Symlink);
        // Unsupported conversion captures opaque with a limitation.
        let mut profiles = BTreeMap::new();
        profiles.insert(b"big.lfs".to_vec(), ConversionProfile::Unsupported);
        let e = decide(
            &candidate("big.lfs", Source::Tracked),
            &ignored,
            &profiles,
            &sizes,
            Some(b"version https://git-lfs.github.com/spec/v1"),
            &mut limitations,
        );
        assert_eq!(e.class, Some(ContentClass::Opaque));
        assert!(
            limitations
                .iter()
                .any(|l| l.contains("unsupported conversion"))
        );
        // Untracked + ignored is excluded.
        let mut ignored = BTreeSet::new();
        ignored.insert(b"ign.txt".to_vec());
        let e = decide(
            &candidate("ign.txt", Source::Untracked),
            &ignored,
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            &mut limitations,
        );
        assert_eq!(e.excluded, Some(Exclusion::IgnoredByGit));
    }

    #[test]
    fn fingerprint_binds_the_scan_inputs() {
        let mk = |ls: &[u8], st: &[u8]| {
            let mut fp = Vec::new();
            fp.extend_from_slice(b"sha1\0unborn:main\0");
            fp.extend_from_slice(ls);
            fp.push(0);
            fp.extend_from_slice(st);
            profile::raw_digest("puyudb/git-capture-fingerprint/v1", &fp)
        };
        assert_eq!(mk(b"ls", b"st"), mk(b"ls", b"st"));
        assert_ne!(mk(b"ls", b"st"), mk(b"ls2", b"st"));
        assert_ne!(mk(b"ls", b"st"), mk(b"ls", b"st2"));
    }

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// End-to-end smoke test against a real system-git repo created in a temp
    /// dir. Skipped when no git binary is present (P4 adds the full fixture
    /// matrix); the scratch repo is removed afterwards.
    #[test]
    fn smoke_discover_capture_preview_real_repo() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("puyudb-i07-git-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "-b", "main", "."]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        std::fs::write(dir.join("b.bin"), [0u8, 159, 146, 150]).unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/c.txt"), "x").unwrap();
        std::fs::write(dir.join(".env"), "secret=1").unwrap();
        std::fs::write(dir.join("ignored.txt"), "i").unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(dir.join(".gitkeep"), "").unwrap(); // staged EMPTY file
        git(&[
            "add",
            "a.txt",
            "b.bin",
            "sub/c.txt",
            ".gitignore",
            ".gitkeep",
        ]);
        std::fs::write(dir.join("untracked.txt"), "u\n").unwrap();
        std::fs::write(dir.join("run.sh"), "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("run.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            std::os::unix::fs::symlink("a.txt", dir.join("ulink")).unwrap();
        }
        let repo = discover(&dir).expect("discover");
        assert_eq!(repo.object_format, ObjectFormat::Sha1);
        assert!(!repo.gitfile);
        assert_eq!(repo.head, Head::Unborn("main".into()));
        assert_eq!(repo.toplevel, dir.canonicalize().unwrap());
        let cap = capture(&repo).expect("capture");
        assert_eq!(cap.index.len(), 5);
        let pv = preview(&repo, &cap).expect("preview");
        let find = |p: &str| pv.entries.iter().find(|e| e.path == p).unwrap();
        let a = find("a.txt");
        assert_eq!(a.source, Source::Tracked);
        assert_eq!(a.class, Some(ContentClass::Text));
        assert!(a.excluded.is_none());
        assert!(a.git_oid.is_some());
        assert_eq!(find("b.bin").class, Some(ContentClass::Opaque));
        assert_eq!(find("sub/c.txt").class, Some(ContentClass::Text));
        assert_eq!(find("untracked.txt").source, Source::Untracked);
        assert_eq!(find("untracked.txt").class, Some(ContentClass::Text));
        assert_eq!(find(".env").excluded, Some(Exclusion::SensitiveDeny));
        assert_eq!(find("ignored.txt").excluded, Some(Exclusion::IgnoredByGit));
        // Gate A finding 1: a staged empty file is included (empty baseline
        // is legitimate, spec §8.2), never flagged intent-to-add.
        let gk = find(".gitkeep");
        assert!(gk.excluded.is_none());
        assert_eq!(gk.class, Some(ContentClass::Text));
        // Gate A finding 3: untracked exec bit and symlink are metadata.
        let run = find("run.sh");
        assert_eq!(run.mode, ModeTag::Executable);
        assert_eq!(run.class, Some(ContentClass::Text));
        #[cfg(unix)]
        {
            let ulink = find("ulink");
            assert_eq!(ulink.mode, ModeTag::Symlink);
            assert_eq!(ulink.class, Some(ContentClass::SymlinkTarget));
            // The link target bytes are the inert content, not a.txt's bytes.
            let mut reader = BlobReader::open(&repo).unwrap();
            let _ = &mut reader;
            assert_eq!(
                read_symlink_target(&repo, b"ulink", 4096).unwrap(),
                b"a.txt".to_vec()
            );
        }
        assert!(pv.incomplete_capture);
        assert!(!pv.empty_cap);
        assert!(!pv.limitations.is_empty());
        // Fingerprint is stable across identical rescans.
        let cap2 = capture(&repo).unwrap();
        assert_eq!(cap.fingerprint, cap2.fingerprint);
        // Blob read round-trips through cat-file with typed verification.
        let oid = a.git_oid.unwrap();
        let mut reader = BlobReader::open(&repo).unwrap();
        let bytes = reader
            .read_blob(oid, profile::MAX_TEXT_EPOCH_BYTES as u64)
            .unwrap();
        assert_eq!(bytes, b"hello\n");
        // Empty-cap preview on a fresh repo with nothing staged or untracked.
        let empty_dir =
            std::env::temp_dir().join(format!("puyudb-i07-git-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&empty_dir);
        std::fs::create_dir_all(&empty_dir).unwrap();
        let out = Command::new("git")
            .args(["init", "-q", "-b", "main", "."])
            .current_dir(&empty_dir)
            .output()
            .unwrap();
        assert!(out.status.success());
        let erepo = discover(&empty_dir).unwrap();
        let ecap = capture(&erepo).unwrap();
        let epv = preview(&erepo, &ecap).unwrap();
        assert!(epv.empty_cap);
        assert!(epv.entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty_dir);
    }

    /// Gate A re-run finding 8, live: a real merge conflict (stages 1/2/3, no
    /// stage 0, one `u` record) must surface in the preview as Unmerged
    /// conflict metadata, never silently absent.
    #[test]
    fn smoke_merge_conflict_is_reported() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("puyudb-i07-git-conf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "-b", "main", "."]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("ok.txt"), "ok\n").unwrap();
        std::fs::write(dir.join("c.txt"), "base\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        git(&["checkout", "-qb", "side"]);
        std::fs::write(dir.join("c.txt"), "side\n").unwrap();
        git(&["add", "c.txt"]);
        git(&["commit", "-qm", "side"]);
        git(&["checkout", "-q", "main"]);
        std::fs::write(dir.join("c.txt"), "main\n").unwrap();
        git(&["add", "c.txt"]);
        git(&["commit", "-qm", "main"]);
        // Merge now conflicts; the repo is left mid-merge (read-only for us).
        let out = Command::new("git")
            .args(["merge", "side"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(!out.status.success(), "expected a merge conflict");
        let repo = discover(&dir).unwrap();
        match repo.head {
            Head::Commit(_) => {}
            other => panic!("expected commit HEAD mid-merge, got {other:?}"),
        }
        let cap = capture(&repo).unwrap();
        // Index has stages 1/2/3 for c.txt and a stage-0 entry for ok.txt.
        let c_stages: Vec<u8> = cap
            .index
            .iter()
            .filter(|e| e.path == b"c.txt")
            .map(|e| e.stage)
            .collect();
        assert_eq!(c_stages, vec![1, 2, 3]);
        assert!(
            cap.index
                .iter()
                .any(|e| e.path == b"ok.txt" && e.stage == 0)
        );
        let pv = preview(&repo, &cap).unwrap();
        let c = pv
            .entries
            .iter()
            .find(|e| e.path == "c.txt")
            .expect("conflicted path must be present in the preview");
        assert_eq!(c.excluded, Some(Exclusion::Unmerged));
        assert_eq!(c.unmerged_stages.len(), 3);
        assert!(
            pv.limitations
                .iter()
                .any(|l| l.contains("conflict metadata"))
        );
        // The unconflicted file is still included normally.
        let ok = pv.entries.iter().find(|e| e.path == "ok.txt").unwrap();
        assert!(ok.excluded.is_none());
        assert_eq!(ok.class, Some(ContentClass::Text));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SHA-256 repos produce typed 32-byte OIDs (spec §8.1 algorithm tag 2).
    #[test]
    fn smoke_sha256_repo() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("puyudb-i07-git-256-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "--object-format=sha256", "-b", "main", "."]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("f.txt"), "hi\n").unwrap();
        git(&["add", "f.txt"]);
        let repo = discover(&dir).unwrap();
        assert_eq!(repo.object_format, ObjectFormat::Sha256);
        let cap = capture(&repo).unwrap();
        assert_eq!(cap.index.len(), 1);
        match cap.index[0].oid {
            GitOid::Sha256(digest) => assert_eq!(digest.len(), 32),
            other => panic!("expected sha256 oid, got {other:?}"),
        }
        let pv = preview(&repo, &cap).unwrap();
        assert_eq!(pv.entries.len(), 1);
        assert_eq!(pv.entries[0].class, Some(ContentClass::Text));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
