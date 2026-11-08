// SPDX-License-Identifier: Apache-2.0

//! I08 Phase 3 (P4) — T02 history-fidelity battery (blueprint §17.1):
//! every retained version survives reload/checkpoint/cache-discard/snapshot
//! pin byte-identically, and no post-cut batch alters ANY old view
//! (hash/content/conflict comparison over all small-history frontiers).
//! Plus the spec-negative frontier battery, broad-DAG and cancellation
//! tests, and the gc-relevant fixture (a checkpoint never replaces the
//! immutable changes an older retained version needs).

mod history_common;

use std::time::{Duration, Instant};

use history_common::*;
use puyudb_core::api::{Database, ReadBudget};
use puyudb_core::identity::{BatchId, VersionId, WriterEpoch};
use puyudb_core::profile::{self, Frontier, LiveTag, Operation, Path};
use puyudb_core::projection::Field;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fingerprint of one version's full view: semantic-manifest hash, text,
/// paths, conflicts, file count, native-head count.
fn fingerprint(db: &Database, v: &VersionId, epoch: &puyudb_core::identity::EpochId) -> String {
    let manifest = db.semantic_manifest_at_version(v).unwrap();
    let manifest_id = hex(&manifest.id().unwrap());
    let text = db
        .text_at_version(v, epoch, None)
        .unwrap_or_else(|e| panic!("text read failed at {v}: {e}"));
    let mut paths: Vec<String> = db
        .paths_at_version(v)
        .unwrap()
        .iter()
        .map(|(p, files)| format!("{}=>{}", p.as_str(), files.len()))
        .collect();
    paths.sort();
    let conflicts = db.conflicts_at_version(v).unwrap().len();
    let files = db.files_at_version(v).unwrap().len();
    let native_heads = db.native_heads(v).unwrap().len();
    format!(
        "manifest={manifest_id} text={text:?} paths={paths:?} conflicts={conflicts} files={files} native_heads={native_heads}"
    )
}

/// A small but non-trivial history: create, rename, text edit, two
/// concurrent renames (conflict), join, delete, second edit. Returns the
/// world plus every branch-frontier version in order.
fn build_history(dir: &std::path::Path) -> (TextWorld, Vec<VersionId>) {
    let w = text_world_in(dir, "hello world");
    let mut versions = vec![w.db.version_of(&[w.root]).unwrap()];

    let c2 = publish_meta(
        &w.db,
        meta(2, Some(w.root), vec![w.root], 2),
        vec![Operation::Path {
            file: FILE,
            path: Path::new("b.txt").unwrap(),
        }],
        2,
        &[2],
    );
    versions.push(w.db.version_of(&[c2]).unwrap());

    let c3_change = w.change_after(&[], 11, "!");
    let c3 = publish_native(
        &w.db,
        meta(3, Some(c2), vec![c2], 3),
        w.epoch,
        &c3_change,
        3,
        &[3],
    );
    versions.push(w.db.version_of(&[c3]).unwrap());

    // Two concurrent renames over the same base cut → register conflict.
    // (Fork parents include c3 so the main stream's `previous` stays in the
    // join's parent closure, §13.1.)
    let fork_a = WriterEpoch::from_array([0xa1; 16]);
    let fork_b = WriterEpoch::from_array([0xb2; 16]);
    let c4 = publish_meta_on(
        &fork_a,
        &w.db,
        meta(1, None, vec![c3], 4),
        vec![Operation::Path {
            file: FILE,
            path: Path::new("c.txt").unwrap(),
        }],
        4,
        &[4],
    );
    let c5 = publish_meta_on(
        &fork_b,
        &w.db,
        meta(1, None, vec![c3], 4),
        vec![Operation::Path {
            file: FILE,
            path: Path::new("d.txt").unwrap(),
        }],
        5,
        &[5],
    );
    let mut join_parents = vec![c4, c5];
    join_parents.sort_unstable();
    let c6 = publish_meta(
        &w.db,
        meta(4, Some(c3), join_parents, 5),
        Vec::new(),
        6,
        &[6],
    );
    versions.push(w.db.version_of(&[c6]).unwrap());

    let c7 = publish_meta(
        &w.db,
        meta(5, Some(c6), vec![c6], 6),
        vec![Operation::Live {
            file: FILE,
            live: LiveTag::Deleted,
        }],
        7,
        &[7],
    );
    versions.push(w.db.version_of(&[c7]).unwrap());

    let c8_change = w.change_after(std::slice::from_ref(&c3_change), 12, "?");
    let c8 = publish_native(
        &w.db,
        meta(6, Some(c7), vec![c7], 7),
        w.epoch,
        &c8_change,
        8,
        &[8],
    );
    versions.push(w.db.version_of(&[c8]).unwrap());

    (w, versions)
}

// publish_meta_on is a helper local to checkpoints.rs; re-declare here via the
// common module path (history_common re-exports publish_meta only), so use a
// small local wrapper with an explicit writer epoch.
fn publish_meta_on(
    writer_epoch: &WriterEpoch,
    db: &Database,
    m: Meta,
    ops: Vec<Operation>,
    salt: u8,
    key: &[u8],
) -> BatchId {
    let (manifest, chunks) = frame_payload(db, "metadata-v1", &ops_payload(ops, salt));
    let metadata = manifest.id().unwrap();
    let mut b = body(m, metadata);
    b.writer_epoch = *writer_epoch;
    let id = b.id().unwrap();
    publish(db, &b, std::slice::from_ref(&manifest), &chunks, key);
    id
}

#[test]
fn t02_every_frontier_survives_reload_checkpoints_cache_discard_and_pins() {
    let dir = temp();
    let (w, versions) = build_history(&dir);
    let epoch = w.epoch;

    // Baseline fingerprints straight after publication.
    let base: Vec<String> = versions
        .iter()
        .map(|v| fingerprint(&w.db, v, &epoch))
        .collect();

    // 1. Reload: every retained version resolves and reads identically.
    drop(w.db);
    let db = Database::open_space(&dir, &SPACE).unwrap();
    for (v, want) in versions.iter().zip(&base) {
        assert_eq!(
            &fingerprint(&db, v, &epoch),
            want,
            "reload changed view of {v}"
        );
    }

    // 2. Checkpoints present: pin every version (threshold 1 → each pin
    //    checkpoints its version), then reads are identical.
    db.set_checkpoint_replay_threshold(1).unwrap();
    for (i, v) in versions.iter().enumerate() {
        assert!(db.create_snapshot(&format!("v{i}"), v).unwrap());
        assert!(db.checkpoint_cached(v).unwrap(), "pin {i} checkpointed");
    }
    for (v, want) in versions.iter().zip(&base) {
        assert_eq!(
            &fingerprint(&db, v, &epoch),
            want,
            "checkpointed read changed {v}"
        );
        // Read through the snapshot pin too.
        let pinned = db
            .snapshot_version(&format!(
                "v{}",
                versions.iter().position(|x| x == v).unwrap()
            ))
            .unwrap()
            .unwrap();
        assert_eq!(&pinned, v);
    }

    // 3. Cache discard (D3, test-pinned): rm -rf cache/ → identical reads.
    std::fs::remove_dir_all(dir.join("cache")).unwrap();
    for (v, want) in versions.iter().zip(&base) {
        assert!(!db.checkpoint_cached(v).unwrap());
        assert_eq!(
            &fingerprint(&db, v, &epoch),
            want,
            "cache-discard changed {v}"
        );
    }

    // 4. Snapshot pins survive the cache discard and still resolve+read.
    for (i, v) in versions.iter().enumerate() {
        assert_eq!(db.snapshot_version(&format!("v{i}")).unwrap(), Some(*v));
        assert_eq!(&fingerprint(&db, v, &epoch), &base[i]);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn future_leak_battery_no_post_cut_batch_alters_any_old_view() {
    let dir = temp();
    let (w, versions) = build_history(&dir);
    let epoch = w.epoch;
    let base: Vec<String> = versions
        .iter()
        .map(|v| fingerprint(&w.db, v, &epoch))
        .collect();

    // Post-cut batches: text edit, rename, delete, new file creation.
    let head = *versions.last().unwrap();
    let head_batch = w.db.resolve_version(&head).unwrap().heads[0];
    let c3c = w.change_after(&[], 11, "!");
    let c8c = w.change_after(std::slice::from_ref(&c3c), 12, "?");
    let e1_change = w.change_after(&[c3c, c8c], 13, "!!");
    let c9 = publish_native(
        &w.db,
        meta(7, Some(head_batch), vec![head_batch], 8),
        w.epoch,
        &e1_change,
        9,
        &[9],
    );
    let c10 = publish_meta(
        &w.db,
        meta(8, Some(c9), vec![c9], 9),
        vec![Operation::Path {
            file: FILE,
            path: Path::new("renamed.txt").unwrap(),
        }],
        10,
        &[10],
    );
    let c11 = publish_meta(
        &w.db,
        meta(9, Some(c10), vec![c10], 10),
        vec![Operation::Live {
            file: FILE,
            live: LiveTag::Alive,
        }],
        11,
        &[11],
    );
    let _ = c11;

    for (v, want) in versions.iter().zip(&base) {
        assert_eq!(
            &fingerprint(&w.db, v, &epoch),
            want,
            "future batch leaked into old view {v}"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleted_content_queryable_at_old_versions() {
    let dir = temp();
    let (w, versions) = build_history(&dir);
    // versions[3] is the join (alive, conflicted paths), versions[4] deleted.
    let v_alive = versions[3];
    let v_deleted = versions[4];
    let files_alive = w.db.files_at_version(&v_alive).unwrap();
    assert_eq!(files_alive[0].live, vec![LiveTag::Alive]);
    let files_deleted = w.db.files_at_version(&v_deleted).unwrap();
    assert_eq!(files_deleted[0].live, vec![LiveTag::Deleted]);
    // Deleted content stays queryable: text and register at the old version.
    assert_eq!(
        w.db.text_at_version(&v_deleted, &w.epoch, None).unwrap(),
        "hello world!"
    );
    let register = w.db.register_at_version(&v_deleted).unwrap();
    assert!(register.iter().any(|r| r.field == Field::Live));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn invalid_encoding_negative_battery() {
    let dir = temp();
    let (w, versions) = build_history(&dir);
    let root = w.root;
    let c2 = w.db.resolve_version(&versions[1]).unwrap().heads[0];

    // Unsorted / duplicate head lists fail at decode (D2, spec §5).
    let craft = |heads: &[&[u8; 32]]| {
        let mut out = Vec::new();
        puyudb_core::canon::write_array_header(&mut out, 3).unwrap();
        puyudb_core::canon::write_uint(&mut out, profile::PROFILE_VERSION).unwrap();
        puyudb_core::canon::write_bytes(&mut out, SPACE.as_bytes()).unwrap();
        puyudb_core::canon::write_array_header(&mut out, heads.len()).unwrap();
        for h in heads {
            puyudb_core::canon::write_bytes(&mut out, *h).unwrap();
        }
        out
    };
    let (hi, lo) = if root > c2 { (root, c2) } else { (c2, root) };
    assert!(
        Frontier::decode(&craft(&[hi.as_bytes(), lo.as_bytes()])).is_err(),
        "unsorted"
    );
    assert!(
        Frontier::decode(&craft(&[root.as_bytes(), root.as_bytes()])).is_err(),
        "duplicate"
    );

    // Redundant head (ancestor of another head) fails consumer validation.
    let mut heads = vec![root, c2];
    heads.sort_unstable();
    let err =
        w.db.validate_frontier(&Frontier {
            space: SPACE,
            heads,
        })
        .expect_err("redundant");
    assert_eq!(err.code(), "invalid_encoding");

    // Over-cap frontier (MAX_FRONTIER_HEADS + 1) is rejected before allocation.
    let cap = profile::MAX_FRONTIER_HEADS + 1;
    let mut out = Vec::new();
    puyudb_core::canon::write_array_header(&mut out, 3).unwrap();
    puyudb_core::canon::write_uint(&mut out, profile::PROFILE_VERSION).unwrap();
    puyudb_core::canon::write_bytes(&mut out, SPACE.as_bytes()).unwrap();
    puyudb_core::canon::write_array_header(&mut out, cap).unwrap();
    for i in 0..cap {
        let mut h = [0u8; 32];
        h[..8].copy_from_slice(&(i as u64).to_be_bytes());
        puyudb_core::canon::write_bytes(&mut out, &h).unwrap();
    }
    assert!(
        Frontier::decode(&out).is_err(),
        "over-cap frontier must be invalid_encoding"
    );

    // Unknown cut / unknown version.
    let ghost = BatchId::from_array([0x77; 32]);
    assert_eq!(
        w.db.version_of(&[ghost]).unwrap_err().code(),
        "invalid_encoding"
    );
    let unknown = VersionId::from_array([0xee; 32]);
    assert_eq!(
        w.db.resolve_version(&unknown).unwrap_err().code(),
        "content_unavailable"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn broad_dag_and_budget_cancellation() {
    let dir = temp();
    let w = text_world_in(&dir, "base text for the broad dag");
    let root = w.root;

    // 32 concurrent children of root, each on its own writer epoch.
    const N: u64 = 32;
    let mut heads = Vec::new();
    for i in 0..N {
        // Offset avoids the main WRITER_EPOCH ([4; 16]).
        let epoch = WriterEpoch::from_array([(i as u8) + 100; 16]);
        let child = publish_meta_on(
            &epoch,
            &w.db,
            meta(1, None, vec![root], 2),
            vec![Operation::Path {
                file: FILE,
                path: Path::new(&format!("f{i}.txt")).unwrap(),
            }],
            (i as u8) + 10,
            &[(i as u8) + 10],
        );
        heads.push(child);
    }
    heads.sort_unstable();
    let fork_version = w.db.version_of(&heads).unwrap();
    assert_eq!(w.db.closure(&fork_version).unwrap().len(), 33);
    for h in &heads {
        assert!(w.db.is_ancestor_of(&root, h).unwrap());
        assert!(!w.db.is_ancestor_of(h, &root).unwrap());
    }
    // The join over all heads is one more version; closure grows by 1.
    let join = publish_meta(
        &w.db,
        meta(2, Some(root), heads.clone(), 3),
        Vec::new(),
        99,
        &[99],
    );
    let v_join = w.db.version_of(&[join]).unwrap();
    assert_eq!(w.db.closure(&v_join).unwrap().len(), 34);

    // Budget cancellation on a read: zero steps → precondition_failed.
    let zero = ReadBudget {
        max_steps: 0,
        max_output_bytes: u64::MAX,
        deadline: None,
    };
    let err =
        w.db.text_at_version(&v_join, &w.epoch, Some(&zero))
            .expect_err("cancelled");
    assert_eq!(err.code(), "precondition_failed");
    // Deadline in the past cancels on the first step (checked every step now).
    let past = ReadBudget {
        max_steps: u64::MAX,
        max_output_bytes: u64::MAX,
        deadline: Some(Instant::now() - Duration::from_secs(1)),
    };
    let err =
        w.db.text_at_version(&v_join, &w.epoch, Some(&past))
            .expect_err("deadline");
    assert_eq!(err.code(), "precondition_failed");
    // Unbounded read still fine.
    assert_eq!(
        w.db.text_at_version(&v_join, &w.epoch, None).unwrap(),
        "base text for the broad dag"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gc_relevant_fixture_checkpoint_never_replaces_immutable_changes() {
    let dir = temp();
    let w = text_world_in(&dir, "immutable history");
    let c2_change = w.change_after(&[], 17, "!");
    let c2 = publish_native(
        &w.db,
        meta(2, Some(w.root), vec![w.root], 2),
        w.epoch,
        &c2_change,
        2,
        &[2],
    );
    let v2 = w.db.version_of(&[c2]).unwrap();
    w.db.set_checkpoint_replay_threshold(2).unwrap();
    let c3_change = w.change_after(std::slice::from_ref(&c2_change), 18, "?");
    let c3 = publish_native(
        &w.db,
        meta(3, Some(c2), vec![c2], 3),
        w.epoch,
        &c3_change,
        3,
        &[3],
    );
    let v3 = w.db.version_of(&[c3]).unwrap();
    assert!(w.db.checkpoint_cached(&v3).unwrap());
    assert!(w.db.create_snapshot("pin", &v2).unwrap());

    // The checkpoint caches a view; the immutable changes stay in the ledger.
    let heads_before = w.db.native_heads(&v2).unwrap();
    assert_eq!(heads_before.len(), 2, "seed + one change");

    std::fs::remove_dir_all(dir.join("cache")).unwrap();

    // After cache discard the pin still resolves and the immutable native
    // changes are all still there — the checkpoint never replaced them.
    assert_eq!(w.db.snapshot_version("pin").unwrap(), Some(v2));
    assert_eq!(w.db.native_heads(&v2).unwrap(), heads_before);
    assert_eq!(
        w.db.text_at_version(&v2, &w.epoch, None).unwrap(),
        "immutable history!"
    );
    assert_eq!(
        w.db.text_at_version(&v3, &w.epoch, None).unwrap(),
        "immutable history!?"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
