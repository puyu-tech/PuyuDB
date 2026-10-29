//! Minimal I04 publication workload shared by the ADR-02 storage spike.
//!
//! An acknowledged publication is one whose id was appended to `acks.log` and
//! fsynced only after the engine's durable commit returned. The engine may
//! contain unacked publications. It must not contain a partial one.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub const BOUNDARY_ITERS: u64 = 24;
pub const RANDOM_ITERS: u64 = 24;
pub const RANDOM_LOOP: u64 = 200;
pub const MEASURE_WARMUP: u64 = 20;
pub const MEASURE_COUNT: u64 = 300;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chunks (
  key BLOB PRIMARY KEY,
  bytes BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS roots (
  root_id INTEGER PRIMARY KEY,
  stream INTEGER NOT NULL,
  seq INTEGER NOT NULL,
  chunk_key BLOB NOT NULL,
  UNIQUE(stream, seq),
  FOREIGN KEY(chunk_key) REFERENCES chunks(key)
);
CREATE TABLE IF NOT EXISTS events (
  cursor INTEGER PRIMARY KEY,
  root_id INTEGER NOT NULL UNIQUE,
  FOREIGN KEY(root_id) REFERENCES roots(root_id)
);
CREATE TABLE IF NOT EXISTS outbox (
  root_id INTEGER PRIMARY KEY,
  peer INTEGER NOT NULL,
  FOREIGN KEY(root_id) REFERENCES roots(root_id)
);
";

pub trait Engine: Sized {
    fn open(dir: &Path) -> Result<Self, String>;
    fn stage_chunk(&mut self, key: &[u8; 32], bytes: &[u8]) -> Result<(), String>;
    fn publish(&mut self, root: u64, stream: u64, seq: u64, key: &[u8; 32]) -> Result<(), String>;
    fn snapshot(&self) -> Result<Snapshot, String>;
    fn probe_native(dir: &Path) -> Result<String, String>;
    fn identify() -> String;
    fn settings(&self) -> String {
        "settings=n/a".into()
    }
}

#[derive(Clone, Debug)]
pub struct RootRow {
    pub root_id: u64,
    pub stream: u64,
    pub seq: u64,
    pub chunk_key: [u8; 32],
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub chunks: Vec<[u8; 32]>,
    pub roots: Vec<RootRow>,
    pub events: Vec<(u64, u64)>,
    pub outbox: Vec<(u64, u64)>,
}

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub roots: u64,
    pub events: u64,
    pub outbox: u64,
    pub staged_only: u64,
    pub missing_chunk: u64,
    pub incomplete_root: u64,
    pub orphan_event: u64,
    pub orphan_outbox: u64,
    pub dup_stream_seq: u64,
    pub dup_root: u64,
    pub acked_missing: u64,
    pub acked_incomplete: u64,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.missing_chunk == 0
            && self.incomplete_root == 0
            && self.orphan_event == 0
            && self.orphan_outbox == 0
            && self.dup_stream_seq == 0
            && self.dup_root == 0
            && self.acked_missing == 0
            && self.acked_incomplete == 0
            && self.roots == self.events
            && self.roots == self.outbox
    }

    pub fn line(&self) -> String {
        format!(
            "VERIFY ok={} roots={} events={} outbox={} staged_only={} missing_chunk={} incomplete_root={} orphan_event={} orphan_outbox={} dup_stream_seq={} dup_root={} acked_missing={} acked_incomplete={}",
            u8::from(self.is_ok()),
            self.roots,
            self.events,
            self.outbox,
            self.staged_only,
            self.missing_chunk,
            self.incomplete_root,
            self.orphan_event,
            self.orphan_outbox,
            self.dup_stream_seq,
            self.dup_root,
            self.acked_missing,
            self.acked_incomplete,
        )
    }
}

pub fn chunk_key(seed: u64, root: u64) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[..8].copy_from_slice(&seed.to_le_bytes());
    key[8..16].copy_from_slice(&root.to_le_bytes());
    key[16] = 0xA4;
    key
}

pub fn chunk_bytes(key: &[u8; 32]) -> [u8; 64] {
    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(key);
    bytes[32..].copy_from_slice(key);
    bytes
}

pub fn payload(key: &[u8; 32], len: u32) -> Vec<u8> {
    let len = len.max(32) as usize;
    let mut body = vec![0u8; len];
    if std::env::var_os("ADR02_INCOMPRESSIBLE").is_some() {
        let mut state = u64::from_le_bytes(key[..8].try_into().unwrap_or([0; 8]));
        for byte in &mut body {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *byte = state as u8;
        }
    } else {
        for (index, byte) in body.iter_mut().enumerate() {
            *byte = key[index % 32];
        }
    }
    body
}

pub fn pack_root(key: &[u8; 32], stream: u64, seq: u64) -> [u8; 48] {
    let mut out = [0u8; 48];
    out[..32].copy_from_slice(key);
    out[32..40].copy_from_slice(&stream.to_le_bytes());
    out[40..48].copy_from_slice(&seq.to_le_bytes());
    out
}

pub fn unpack_root(bytes: &[u8]) -> Option<([u8; 32], u64, u64)> {
    if bytes.len() != 48 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes[..32]);
    let stream = u64::from_le_bytes(bytes[32..40].try_into().ok()?);
    let seq = u64::from_le_bytes(bytes[40..48].try_into().ok()?);
    Some((key, stream, seq))
}

pub fn u64_key(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

pub fn stream_seq_key(stream: u64, seq: u64) -> [u8; 16] {
    let mut key = [0u8; 16];
    key[..8].copy_from_slice(&stream.to_le_bytes());
    key[8..].copy_from_slice(&seq.to_le_bytes());
    key
}

pub fn check(snapshot: &Snapshot, acked: &[u64]) -> Report {
    let mut report = Report {
        roots: snapshot.roots.len() as u64,
        events: snapshot.events.len() as u64,
        outbox: snapshot.outbox.len() as u64,
        ..Report::default()
    };
    let mut seen_root = std::collections::BTreeSet::new();
    let mut seen_ss = std::collections::BTreeSet::new();
    let mut rooted = std::collections::BTreeSet::new();
    for root in &snapshot.roots {
        if !seen_root.insert(root.root_id) {
            report.dup_root += 1;
        }
        if !seen_ss.insert((root.stream, root.seq)) {
            report.dup_stream_seq += 1;
        }
        rooted.insert(root.chunk_key);
        if !snapshot.chunks.contains(&root.chunk_key) {
            report.missing_chunk += 1;
        }
        let events = snapshot
            .events
            .iter()
            .filter(|(_, id)| *id == root.root_id)
            .count();
        let outbox = snapshot
            .outbox
            .iter()
            .filter(|(id, _)| *id == root.root_id)
            .count();
        if events != 1 || outbox != 1 {
            report.incomplete_root += 1;
        }
    }
    report.staged_only = snapshot
        .chunks
        .iter()
        .filter(|key| !rooted.contains(*key))
        .count() as u64;
    report.orphan_event = snapshot
        .events
        .iter()
        .filter(|(_, id)| !seen_root.contains(id))
        .count() as u64;
    report.orphan_outbox = snapshot
        .outbox
        .iter()
        .filter(|(id, _)| !seen_root.contains(id))
        .count() as u64;
    for id in acked {
        let Some(root) = snapshot.roots.iter().find(|root| root.root_id == *id) else {
            report.acked_missing += 1;
            continue;
        };
        let complete = snapshot.chunks.contains(&root.chunk_key)
            && snapshot.events.iter().filter(|(_, rid)| rid == id).count() == 1
            && snapshot.outbox.iter().filter(|(rid, _)| rid == id).count() == 1;
        if !complete {
            report.acked_incomplete += 1;
        }
    }
    report
}

pub fn main<E: Engine>() -> Result<(), String> {
    let args = Args::parse()?;
    fs::create_dir_all(&args.dir).map_err(|e| e.to_string())?;
    if !args.no_owner_lock {
        hold_owner_lock(&args.dir)?;
    }
    match args.cmd.as_str() {
        "version" => {
            println!("{}", E::identify());
            Ok(())
        }
        "hold-lock" => {
            println!("LOCKED");
            let _ = std::io::stdout().flush();
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        "stage-wait" => stage_wait::<E>(&args),
        "loop" => publish_loop::<E>(&args),
        "preflight" => preflight::<E>(&args.dir),
        "verify" => {
            let acked = read_acks(args.acks.as_deref().unwrap_or(Path::new("")))?;
            let engine = E::open(&args.dir)?;
            let report = check(&engine.snapshot()?, &acked);
            println!("{}", report.line());
            if report.is_ok() {
                Ok(())
            } else {
                Err(report.line())
            }
        }
        "probe-native" => {
            let (tx, rx) = mpsc::channel();
            let dir = args.dir.clone();
            thread::spawn(move || {
                let _ = tx.send(E::probe_native(&dir));
            });
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Ok(line)) => {
                    println!("PROBE {line}");
                    Ok(())
                }
                Ok(Err(err)) => Err(err),
                Err(_) => {
                    println!("PROBE timed_out_likely_blocked");
                    Ok(())
                }
            }
        }
        "measure" => measure::<E>(&args),
        "settings" => {
            let engine = E::open(&args.dir)?;
            println!("{}", engine.settings());
            Ok(())
        }
        "stage-many-wait" => stage_many_wait::<E>(&args),
        "create-wait" => create_wait::<E>(&args),
        other => Err(format!("unknown command {other}")),
    }
}

fn create_wait<E: Engine>(args: &Args) -> Result<(), String> {
    println!("OPENING");
    let _ = std::io::stdout().flush();
    let mut engine = E::open(&args.dir)?;
    println!("OPENED");
    let _ = std::io::stdout().flush();
    // Gap so a kill after OPENED lands before the first chunk commit.
    thread::sleep(Duration::from_millis(30));
    let key = chunk_key(args.seed, 1);
    engine.stage_chunk(&key, &payload(&key, 64))?;
    println!("CHUNK_DURABLE 1");
    let _ = std::io::stdout().flush();
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn stage_many_wait<E: Engine>(args: &Args) -> Result<(), String> {
    let mut engine = E::open(&args.dir)?;
    let count = args.count.max(1) as u64;
    for root in 1..=count {
        let key = chunk_key(args.seed, root);
        engine.stage_chunk(&key, &payload(&key, args.bytes))?;
    }
    println!("STAGED {count}");
    let _ = std::io::stdout().flush();
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn stage_wait<E: Engine>(args: &Args) -> Result<(), String> {
    let mut engine = E::open(&args.dir)?;
    let key = chunk_key(args.seed, args.root);
    engine.stage_chunk(&key, &payload(&key, args.bytes))?;
    println!("CHUNK_DURABLE {}", args.root);
    let _ = std::io::stdout().flush();
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn publish_loop<E: Engine>(args: &Args) -> Result<(), String> {
    let mut engine = E::open(&args.dir)?;
    let mut acks = OpenOptions::new()
        .create(true)
        .append(true)
        .open(args.dir.join("acks.log"))
        .map_err(|e| e.to_string())?;
    for root in 1..=args.count.max(1) as u64 {
        let key = chunk_key(args.seed, root);
        engine.stage_chunk(&key, &payload(&key, args.bytes))?;
        engine.publish(root, 1, root, &key)?;
        record_ack(&mut acks, &args.dir, root)?;
        println!("ACK {root}");
        let _ = std::io::stdout().flush();
    }
    Ok(())
}

fn preflight<E: Engine>(dir: &Path) -> Result<(), String> {
    let mut engine = E::open(dir)?;
    let key = chunk_key(1, 1);
    engine.stage_chunk(&key, &payload(&key, 64))?;
    engine.publish(1, 1, 1, &key)?;
    let dup = engine.publish(2, 1, 1, &key);
    if dup.is_ok() {
        return Err("duplicate stream/seq was accepted".into());
    }
    let missing = engine.publish(3, 1, 2, &chunk_key(1, 3));
    if missing.is_ok() {
        return Err("publish without staged chunk was accepted".into());
    }
    let report = check(&engine.snapshot()?, &[1]);
    println!("{}", report.line());
    if report.is_ok() && report.roots == 1 {
        println!("PREFLIGHT ok");
        Ok(())
    } else {
        Err(format!("preflight verify failed: {}", report.line()))
    }
}

fn measure<E: Engine>(args: &Args) -> Result<(), String> {
    let count = args.count.max(1) as u64;
    let mut engine = E::open(&args.dir)?;
    for root in 1..=MEASURE_WARMUP {
        let key = chunk_key(args.seed, root);
        engine.stage_chunk(&key, &payload(&key, args.bytes))?;
        engine.publish(root, 1, root, &key)?;
    }
    let mut publish_ns = Vec::with_capacity(count as usize);
    let started = Instant::now();
    for offset in 1..=count {
        let root = MEASURE_WARMUP + offset;
        let key = chunk_key(args.seed, root);
        engine.stage_chunk(&key, &payload(&key, args.bytes))?;
        let one = Instant::now();
        engine.publish(root, 1, root, &key)?;
        publish_ns.push(one.elapsed().as_nanos());
    }
    let elapsed = started.elapsed();
    drop(engine);
    let reopen_started = Instant::now();
    let engine = E::open(&args.dir)?;
    let acked: Vec<u64> = (MEASURE_WARMUP + 1..=MEASURE_WARMUP + count).collect();
    let report = check(&engine.snapshot()?, &acked);
    let reopen = reopen_started.elapsed();
    if !report.is_ok() {
        return Err(report.line());
    }
    publish_ns.sort_unstable();
    let bytes = dir_bytes(&args.dir)?;
    let sum: u128 = publish_ns.iter().sum();
    println!("engine={}", E::identify());
    println!("chunk_bytes={}", args.bytes);
    println!("samples={count}");
    println!("rank=nearest-rank ceil(p*n/100)-1");
    println!("publish_p50_ns={}", percentile(&publish_ns, 50));
    println!("publish_p99_ns={}", percentile(&publish_ns, 99));
    println!("publish_sum_ns={sum}");
    println!("workload_ns={}", elapsed.as_nanos());
    println!(
        "throughput_per_s={:.4}",
        count as f64 / elapsed.as_secs_f64()
    );
    println!("file_bytes={bytes}");
    list_files(&args.dir)?;
    println!("reopen_verify_ns={}", reopen.as_nanos());
    println!("{}", report.line());
    Ok(())
}

fn list_files(dir: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    fn walk(root: &Path, path: &Path, files: &mut Vec<(String, u64)>) -> Result<(), String> {
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            if meta.is_dir() {
                walk(root, &entry.path(), files)?;
            } else {
                let name = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .into_owned();
                files.push((name, meta.len()));
            }
        }
        Ok(())
    }
    walk(dir, dir, &mut files)?;
    files.sort();
    for (name, len) in files {
        println!("file {name} {len}");
    }
    Ok(())
}

fn percentile(sorted: &[u128], pct: u128) -> u128 {
    let n = sorted.len();
    let rank = (pct * n as u128).div_ceil(100);
    sorted[rank.saturating_sub(1) as usize]
}

fn dir_bytes(dir: &Path) -> Result<u64, String> {
    fn walk(path: &Path, total: &mut u64) -> Result<(), String> {
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            if meta.is_dir() {
                walk(&entry.path(), total)?;
            } else {
                *total += meta.len();
            }
        }
        Ok(())
    }
    let mut total = 0;
    walk(dir, &mut total)?;
    Ok(total)
}

pub fn engine_bin(name: &str) -> String {
    let exe = std::env::current_exe().expect("current_exe");
    exe.parent()
        .and_then(|path| path.parent())
        .expect("target profile dir")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

pub const PRE_ACK_DELAYS: u64 = 40;
pub const PRE_ACK_BOUNDARY: u64 = 12;

/// Kills before any durable publication ack, including database creation.
pub fn pre_ack_kills(bin: &str) {
    for iter in 0..PRE_ACK_DELAYS {
        kill_before_ack(bin, 0xA402_2000 + iter, KillAt::Delay(iter));
    }
    for iter in 0..PRE_ACK_BOUNDARY {
        kill_before_ack(bin, 0xA402_3000 + iter, KillAt::Line("OPENING"));
        kill_before_ack(bin, 0xA402_4000 + iter, KillAt::Line("OPENED"));
    }
}

enum KillAt {
    Delay(u64),
    Line(&'static str),
}

fn kill_before_ack(bin: &str, seed: u64, at: KillAt) {
    let dir = unique_dir(&format!("preack-{seed}"));
    let mut child = Command::new(bin)
        .args([
            "--cmd",
            "create-wait",
            "--dir",
            &dir.to_string_lossy(),
            "--seed",
            &seed.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn create-wait");
    match at {
        KillAt::Delay(ms) => thread::sleep(Duration::from_millis(ms)),
        KillAt::Line(prefix) => {
            let stdout = child.stdout.take().expect("stdout");
            let signaled = wait_line(stdout, prefix, Duration::from_secs(30));
            if signaled.is_err() {
                kill_child(&mut child);
                let _ = child.wait();
                panic!("no {prefix} for {bin} seed {seed}: {signaled:?}");
            }
        }
    }
    kill_child(&mut child);
    let status = child.wait().expect("wait");
    if let Some(code) = status.code().filter(|code| *code != 0) {
        panic!("create-wait exited {code} before kill for seed {seed}");
    }
    let output = Command::new(bin)
        .args([
            "--cmd",
            "verify",
            "--dir",
            &dir.to_string_lossy(),
            "--acks",
            &dir.join("missing-acks").to_string_lossy(),
        ])
        .output()
        .expect("verify pre-ack");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "reopen failed after pre-ack kill seed {seed} in {}: {stdout} {stderr}",
        dir.display()
    );
    let line = stdout
        .lines()
        .find(|line| line.starts_with("VERIFY"))
        .unwrap_or("");
    assert!(
        line.contains("ok=1") && line.contains("roots=0"),
        "pre-ack kill seed {seed} left a publication: {line}"
    );
    let _ = fs::remove_dir_all(&dir);
}

pub fn crash_matrix(bin: &str) {
    let dir = unique_dir("preflight");
    run_ok(
        bin,
        &["--cmd", "preflight", "--dir", &dir.to_string_lossy()],
    );
    let _ = fs::remove_dir_all(&dir);

    let owner = unique_dir("owner");
    let mut holder = Command::new(bin)
        .args(["--cmd", "hold-lock", "--dir", &owner.to_string_lossy()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn holder");
    let stdout = holder.stdout.take().expect("stdout");
    wait_line(stdout, "LOCKED", Duration::from_secs(30)).expect("owner lock signal");
    let second = Command::new(bin)
        .args(["--cmd", "preflight", "--dir", &owner.to_string_lossy()])
        .output()
        .expect("spawn second");
    kill_child(&mut holder);
    let _ = holder.wait();
    assert!(
        !second.status.success(),
        "second writer was accepted: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("owner lock busy"),
        "missing owner-lock error: {stderr}"
    );
    let _ = fs::remove_dir_all(&owner);

    let probe = Command::new(bin)
        .args([
            "--cmd",
            "probe-native",
            "--no-owner-lock",
            "--dir",
            &unique_dir("probe").to_string_lossy(),
        ])
        .output()
        .expect("probe");
    assert!(
        probe.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&probe.stderr)
    );

    for iter in 0..BOUNDARY_ITERS {
        let seed = 0xA402_0000 + iter;
        let dir = unique_dir(&format!("boundary-{seed}"));
        let mut child = Command::new(bin)
            .args([
                "--cmd",
                "stage-wait",
                "--dir",
                &dir.to_string_lossy(),
                "--seed",
                &seed.to_string(),
                "--root",
                "7",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn stage-wait");
        let stdout = child.stdout.take().expect("stdout");
        let signaled = wait_line(stdout, "CHUNK_DURABLE", Duration::from_secs(60));
        kill_child(&mut child);
        let status = child.wait().expect("wait");
        if signaled.is_err() {
            panic!("no CHUNK_DURABLE for {bin} seed {seed}: {signaled:?} status {status:?}");
        }
        verify_dir(bin, &dir, true);
        let _ = fs::remove_dir_all(&dir);
    }

    for iter in 0..RANDOM_ITERS {
        let seed = 0xA402_1000 + iter;
        let dir = unique_dir(&format!("random-{seed}"));
        let mut child = Command::new(bin)
            .args([
                "--cmd",
                "loop",
                "--dir",
                &dir.to_string_lossy(),
                "--seed",
                &seed.to_string(),
                "--count",
                &RANDOM_LOOP.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn loop");
        let stdout = child.stdout.take().expect("stdout");
        // Kill only after the first durable ack, so this iteration is a publication
        // crash rather than a kill during engine creation. The creation window is
        // reported separately; it is timing-dependent.
        let signaled = wait_line(stdout, "ACK", Duration::from_secs(60));
        if signaled.is_err() {
            kill_child(&mut child);
            let _ = child.wait();
            panic!("no ACK before random kill for {bin} seed {seed}: {signaled:?}");
        }
        let delay = Duration::from_millis(5 + (seed % 47));
        thread::sleep(delay);
        kill_child(&mut child);
        let status = child.wait().expect("wait");
        if let Some(code) = status.code().filter(|code| *code != 0) {
            panic!("loop exited {code} before kill for seed {seed}");
        }
        verify_dir(bin, &dir, false);
        let _ = fs::remove_dir_all(&dir);
    }
    stage_many_kill(bin);
}

fn stage_many_kill(bin: &str) {
    let dir = unique_dir("many");
    let mut child = Command::new(bin)
        .args([
            "--cmd",
            "stage-many-wait",
            "--dir",
            &dir.to_string_lossy(),
            "--seed",
            "99",
            "--count",
            "16",
            "--bytes",
            "4096",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn stage-many");
    let stdout = child.stdout.take().expect("stdout");
    let signaled = wait_line(stdout, "STAGED", Duration::from_secs(120));
    kill_child(&mut child);
    let _ = child.wait();
    assert!(signaled.is_ok(), "no STAGED from {bin}: {signaled:?}");
    let output = Command::new(bin)
        .args([
            "--cmd",
            "verify",
            "--dir",
            &dir.to_string_lossy(),
            "--acks",
            &dir.join("acks.log").to_string_lossy(),
        ])
        .output()
        .expect("verify many");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "many verify failed: {stdout} {stderr}"
    );
    let line = stdout
        .lines()
        .find(|line| line.starts_with("VERIFY"))
        .unwrap_or("");
    assert!(
        line.contains("roots=0"),
        "staged chunks became a publication: {line}"
    );
    assert!(
        line.contains("staged_only=16"),
        "staged chunks were not durable: {line}"
    );
    let _ = fs::remove_dir_all(&dir);
}

fn verify_dir(bin: &str, dir: &Path, expect_no_root: bool) {
    let acks = dir.join("acks.log");
    let output = Command::new(bin)
        .args([
            "--cmd",
            "verify",
            "--dir",
            &dir.to_string_lossy(),
            "--acks",
            &acks.to_string_lossy(),
        ])
        .output()
        .expect("verify");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "verify failed in {}: {stdout} {stderr}",
        dir.display()
    );
    let line = stdout
        .lines()
        .find(|line| line.starts_with("VERIFY"))
        .unwrap_or("");
    assert!(
        line.contains("ok=1"),
        "bad verify line in {}: {line}",
        dir.display()
    );
    if expect_no_root {
        assert!(
            line.contains("roots=0"),
            "partial publication visible: {line}"
        );
    }
}

fn run_ok(bin: &str, args: &[&str]) {
    let output = Command::new(bin).args(args).output().expect("spawn");
    assert!(
        output.status.success(),
        "command failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn unique_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "adr02-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn wait_line(
    stdout: std::process::ChildStdout,
    prefix: &str,
    timeout: Duration,
) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let prefix = prefix.to_string();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut sent = false;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    if !sent {
                        let _ = tx.send(Err("eof".into()));
                    }
                    break;
                }
                Ok(_) if !sent && line.starts_with(&prefix) => {
                    sent = true;
                    let _ = tx.send(Ok(line.clone()));
                }
                Ok(_) => {}
                Err(err) => {
                    if !sent {
                        let _ = tx.send(Err(err.to_string()));
                    }
                    break;
                }
            }
        }
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => Err("timeout".into()),
    }
}

fn kill_child(child: &mut Child) {
    // std kills with SIGKILL on Unix and TerminateProcess on Windows.
    let _ = child.kill();
}

fn hold_owner_lock(dir: &Path) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("owner.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock().map_err(|_| "owner lock busy".to_string())?;
    // The fd must stay open until process exit; a crash then releases the OS lock.
    std::mem::forget(file);
    Ok(())
}

fn record_ack(file: &mut File, dir: &Path, root: u64) -> Result<(), String> {
    writeln!(file, "{root}").map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn read_acks(path: &Path) -> Result<Vec<u64>, String> {
    if path.as_os_str().is_empty() || !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.parse::<u64>().map_err(|e| e.to_string()))
        .collect()
}

struct Args {
    cmd: String,
    dir: PathBuf,
    seed: u64,
    count: u32,
    root: u64,
    bytes: u32,
    acks: Option<PathBuf>,
    no_owner_lock: bool,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut args = Self {
            cmd: String::new(),
            dir: PathBuf::from("."),
            seed: 1,
            count: MEASURE_COUNT as u32,
            root: 1,
            bytes: 64,
            acks: None,
            no_owner_lock: false,
        };
        let mut raw = std::env::args().skip(1);
        while let Some(flag) = raw.next() {
            match flag.as_str() {
                "--cmd" => args.cmd = raw.next().ok_or("--cmd needs a value")?,
                "--dir" => args.dir = PathBuf::from(raw.next().ok_or("--dir needs a value")?),
                "--seed" => args.seed = parse_next(&mut raw, "--seed")?,
                "--count" => args.count = parse_next(&mut raw, "--count")?,
                "--root" => args.root = parse_next(&mut raw, "--root")?,
                "--bytes" => args.bytes = parse_next(&mut raw, "--bytes")?,
                "--acks" => {
                    args.acks = Some(PathBuf::from(raw.next().ok_or("--acks needs a value")?))
                }
                "--no-owner-lock" => args.no_owner_lock = true,
                other => return Err(format!("unknown arg {other}")),
            }
        }
        if args.cmd.is_empty() {
            return Err("missing --cmd".into());
        }
        Ok(args)
    }
}

fn parse_next<T: std::str::FromStr>(
    raw: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<T, String> {
    raw.next()
        .ok_or_else(|| format!("{flag} needs a value"))?
        .parse()
        .map_err(|_| format!("{flag} is not a number"))
}
