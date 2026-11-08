// SPDX-License-Identifier: Apache-2.0

//! I08 Phase 3 (P4) — W07 harness (blueprint §16.2): historical queries over
//! 1k/100k/1m-batch spaces with cold/warm checkpoint sets and random retained
//! versions. Metrics: replay cost, read p95 (cold vs warm), cache size.
//!
//! The 1k/100k/1m runs are `#[ignore]`d (they are evidence runs, not CI
//! gates): `cargo test -p puyudb-core --test w07_harness -- --ignored --nocapture`.
//! A small 120-batch variant runs in the normal gate and asserts the
//! correctness invariants (warm reads == cold reads, byte-identical).

mod history_common;

use std::path::Path;
use std::time::Instant;

use history_common::*;
use puyudb_core::api::Database;
use puyudb_core::identity::VersionId;

/// Deterministic LCG (no new dependency).
fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state >> 33
}

fn p95(latencies_us: &mut [u128]) -> u128 {
    latencies_us.sort_unstable();
    let idx = (latencies_us.len() * 95 / 100).min(latencies_us.len().saturating_sub(1));
    latencies_us.get(idx).copied().unwrap_or(0)
}

fn cache_bytes(dir: &Path) -> u64 {
    fn dir_bytes(path: &Path) -> u64 {
        let mut total = 0;
        if let Ok(entries) = std::fs::read_dir(path) {
            for e in entries.flatten() {
                if let Ok(meta) = e.metadata() {
                    total += meta.len();
                }
                if e.path().is_dir() {
                    total += dir_bytes(&e.path());
                }
            }
        }
        total
    }
    dir_bytes(&dir.join("cache"))
}

struct Metrics {
    n: u64,
    publish_ms: u128,
    warm_p95_us: u128,
    cold_p95_us: u128,
    replay_cost_mid: usize,
    cache_bytes: u64,
    pinned: usize,
}

fn run_w07(dir: &Path, n: u64, pin_count: usize, auto_threshold: u64) -> Metrics {
    let w = text_world_in(dir, "seed");
    // Auto-checkpoints from closure size `auto_threshold` on; pinning uses
    // threshold 1 so every retained pinned version gets a warm checkpoint.
    w.db.set_checkpoint_replay_threshold(auto_threshold)
        .unwrap();

    // Keep raw batch ids during the hot loop; VersionIds are computed only
    // for the pinned picks, so harness overhead stays O(n), not O(n^2).
    let mut batches: Vec<puyudb_core::identity::BatchId> = Vec::with_capacity(n as usize + 1);
    batches.push(w.root);
    let mut prev = w.root;
    let mut full = puyudb_crdt::dt_text::TextDoc::load(&w.seed_native).unwrap();
    let t0 = Instant::now();
    for i in 0..n {
        let pos = 4 + i as usize; // "seed" is 4 chars; append one char per batch
        let seq = i + 2;
        let lamport = i + 2;
        let change = w.next_change(&mut full, pos, "x");
        let batch = publish_native(
            &w.db,
            meta(seq, Some(prev), vec![prev], lamport),
            w.epoch,
            &change,
            (i % 251) as u8,
            &i.to_be_bytes(),
        );
        batches.push(batch);
        prev = batch;
    }
    let publish_ms = t0.elapsed().as_millis();

    // Pin K random retained versions (deterministic LCG); each pin generates
    // a hash-checked checkpoint (threshold lowered to 1 for the pins).
    w.db.set_checkpoint_replay_threshold(1).unwrap();
    let mut rng = 0x1234_5678_9abc_def0u64;
    let mut pinned: Vec<VersionId> = Vec::new();
    for _ in 0..pin_count {
        let idx = (lcg(&mut rng) as usize) % batches.len();
        let v = w.db.version_of(&[batches[idx]]).unwrap();
        let name = format!("w07-{}", hex_of(&v));
        if w.db.create_snapshot(&name, &v).unwrap() {
            pinned.push(v);
        }
    }
    assert!(pinned.iter().all(|v| w.db.checkpoint_cached(v).unwrap()));

    let read = |db: &Database, v: &VersionId| -> (u128, String) {
        let t = Instant::now();
        let text = db.text_at_version(v, &w.epoch, None).unwrap();
        (t.elapsed().as_micros(), text)
    };

    // Warm reads (checkpoints cached).
    let mut warm = Vec::new();
    let mut warm_texts = Vec::new();
    for v in &pinned {
        let (us, text) = read(&w.db, v);
        warm.push(us);
        warm_texts.push(text);
    }
    let warm_p95 = p95(&mut warm);
    let cb = cache_bytes(dir);

    // Cold reads: discard the whole cache, then identical reads via replay.
    std::fs::remove_dir_all(dir.join("cache")).unwrap();
    let mut cold = Vec::new();
    for (v, want) in pinned.iter().zip(&warm_texts) {
        assert!(!w.db.checkpoint_cached(v).unwrap());
        let (us, text) = read(&w.db, v);
        assert_eq!(&text, want, "cold replay differs from warm checkpoint read");
        cold.push(us);
    }
    let cold_p95 = p95(&mut cold);

    let mid = w.db.version_of(&[batches[batches.len() / 2]]).unwrap();
    let replay_cost_mid = w.db.closure(&mid).unwrap().len();

    Metrics {
        n,
        publish_ms,
        warm_p95_us: warm_p95,
        cold_p95_us: cold_p95,
        replay_cost_mid,
        cache_bytes: cb,
        pinned: pinned.len(),
    }
}

fn hex_of(v: &VersionId) -> String {
    v.as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .chars()
        .take(12)
        .collect()
}

fn report(m: &Metrics) {
    println!(
        "W07 n={} publish_ms={} warm_p95_us={} cold_p95_us={} replay_cost_mid={} cache_bytes={} pinned={}",
        m.n, m.publish_ms, m.warm_p95_us, m.cold_p95_us, m.replay_cost_mid, m.cache_bytes, m.pinned
    );
}

/// Small W07 variant that runs in the normal test gate: correctness
/// invariants only (warm == cold, byte-identical), metrics printed.
#[test]
fn w07_small_warm_cold_correctness() {
    let dir = temp();
    let m = run_w07(&dir, 24, 4, 8);
    report(&m);
    assert_eq!(m.pinned, 4);
    assert!(
        m.replay_cost_mid > 12,
        "mid-history replay cost is the closure size"
    );
    assert!(
        m.cache_bytes > 0,
        "warm checkpoints are on disk before discard"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "W07 evidence run: cargo test -p puyudb-core --test w07_harness -- --ignored w07_300 --nocapture"]
fn w07_300_batches() {
    let dir = temp();
    report(&run_w07(&dir, 300, 10, 1024));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "W07 evidence run: cargo test -p puyudb-core --test w07_harness -- --ignored w07_1k --nocapture. NOTE: a linear chain publishes in O(n^3) today (branch_heads computes O(n^2) reach pairs per publish) — recorded as a WATCH item for I18; do not run to completion casually"]
fn w07_1k_batches() {
    let dir = temp();
    report(&run_w07(&dir, 1_000, 10, 1024));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "W07 evidence run (long): cargo test -p puyudb-core --test w07_harness -- --ignored w07_100k --nocapture"]
fn w07_100k_batches() {
    let dir = temp();
    report(&run_w07(&dir, 100_000, 10, 1024));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "W07 evidence run (long): cargo test -p puyudb-core --test w07_harness -- --ignored w07_1m --nocapture"]
fn w07_1m_batches() {
    let dir = temp();
    report(&run_w07(&dir, 1_000_000, 10, 1024));
    let _ = std::fs::remove_dir_all(&dir);
}
