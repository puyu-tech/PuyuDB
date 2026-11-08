// SPDX-License-Identifier: Apache-2.0

//! I08 Phase 2 (P3) integration tests: named snapshots with CAS on name
//! take-over (D4, §14), and bounded hash-checked physical checkpoints under
//! `cache/` (§6.2/§6.6, D3) — auto-generated past the configurable
//! replay-cost threshold, disposable (`rm -rf cache/` re-derives identical
//! reads), and never carrying post-cut changes.

use std::path::PathBuf;

use puyudb_core::api::{DEFAULT_REPLAY_COST_THRESHOLD, Database, ReadBudget};
use puyudb_core::identity::{
    ArtifactId, BatchId, BranchId, ChunkId, EpochId, FileId, GenerationNonce, RepositoryId,
    SpaceId, VersionId, WriterEpoch,
};
use puyudb_core::profile::{
    self, ArtifactManifest, BatchBody, EpochDescriptor, EpochKind, MetadataArtifact, ModeTag,
    NativeArtifact, Operation, Path,
};
use puyudb_core::publication::Publication;
use puyudb_crdt::dt_text::TextDoc;

const SPACE: SpaceId = SpaceId::from_array([1; 16]);
const BRANCH: BranchId = BranchId::from_array([2; 16]);
const KEY_SEED: [u8; 32] = [3; 32];
const WRITER_EPOCH: WriterEpoch = WriterEpoch::from_array([4; 16]);
const FILE: FileId = FileId::from_array([1; 16]);

fn temp() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "puyu-i08-p3-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp");
    path
}

fn frame_payload(db: &Database, kind: &str, payload: &[u8]) -> (ArtifactManifest, Vec<ChunkId>) {
    let manifest = ArtifactManifest::for_payload(kind, payload).expect("manifest");
    let chunks = payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(|c| {
            let id = profile::chunk_id(c);
            db.stage_chunk(id, c).expect("stage");
            id
        })
        .collect();
    (manifest, chunks)
}

fn ops_payload(ops: Vec<Operation>, salt: u8) -> Vec<u8> {
    MetadataArtifact {
        operations: ops,
        context_assertions: vec![vec![salt]],
    }
    .encode()
    .expect("metadata artifact")
}

struct Meta {
    sequence: u64,
    previous: Option<BatchId>,
    parents: Vec<BatchId>,
    lamport: u64,
}

fn meta(sequence: u64, previous: Option<BatchId>, parents: Vec<BatchId>, lamport: u64) -> Meta {
    Meta {
        sequence,
        previous,
        parents,
        lamport,
    }
}

fn body(m: Meta, metadata: ArtifactId) -> BatchBody {
    BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: m.sequence,
        previous: m.previous,
        parents: m.parents,
        lamport: m.lamport,
        native_artifacts: Vec::new(),
        metadata,
    }
}

fn publish(
    db: &Database,
    b: &BatchBody,
    artifacts: &[ArtifactManifest],
    chunks: &[ChunkId],
    key: u8,
) -> BatchId {
    let signature = profile::sign_batch(&KEY_SEED, &b.space, &b.id().expect("id"));
    let expected = db.branch_frontier(&BRANCH).expect("frontier");
    let receipt = db
        .transact(
            b"i08-p3",
            &[key],
            &expected,
            &Publication {
                body: b,
                signature_key: &b.public_key,
                signature: &signature,
                artifacts,
                chunks,
                peer: 1,
            },
        )
        .expect("publish");
    receipt.batch_id
}

fn publish_meta(db: &Database, m: Meta, ops: Vec<Operation>, salt: u8, key: u8) -> BatchId {
    let (manifest, chunks) = frame_payload(db, "metadata-v1", &ops_payload(ops, salt));
    let metadata = manifest.id().expect("id");
    let b = body(m, metadata);
    let id = b.id().expect("id");
    publish(db, &b, std::slice::from_ref(&manifest), &chunks, key);
    id
}

fn publish_native(
    db: &Database,
    m: Meta,
    epoch: EpochId,
    change: &[u8],
    salt: u8,
    key: u8,
) -> BatchId {
    let (change_manifest, change_chunks) = frame_payload(
        db,
        "native-change-v1",
        &NativeArtifact {
            epoch,
            profile: profile::TEXT_PROFILE.to_owned(),
            native: change.to_vec(),
        }
        .encode()
        .expect("native artifact"),
    );
    let (meta_manifest, meta_chunks) =
        frame_payload(db, "metadata-v1", &ops_payload(Vec::new(), salt));
    let metadata = meta_manifest.id().expect("id");
    let native_id = change_manifest.id().expect("id");
    let mut b = body(m, metadata);
    b.native_artifacts = vec![native_id];
    let id = b.id().expect("id");
    let chunks: Vec<ChunkId> = [change_chunks, meta_chunks].concat();
    publish(db, &b, &[change_manifest, meta_manifest], &chunks, key);
    id
}

fn create_op(epoch: EpochDescriptor) -> Operation {
    Operation::Create {
        repository: RepositoryId::from_array([9; 16]),
        file: FILE,
        path: Path::new("a.txt").expect("path"),
        mode: ModeTag::Regular,
        epoch,
    }
}

/// A space whose root batch creates `FILE` with a text epoch over
/// `baseline_text`, seed registered.
struct TextWorld {
    db: Database,
    epoch: EpochId,
    seed_native: Vec<u8>,
    root: BatchId,
}

fn text_world_in(dir: &std::path::Path, baseline_text: &str) -> TextWorld {
    let db = Database::create_space(dir, &SPACE).expect("create");
    let (baseline_manifest, baseline_chunks) =
        frame_payload(&db, "text-baseline-v1", baseline_text.as_bytes());
    db.stage_artifact(&baseline_manifest, &baseline_chunks)
        .expect("stage baseline");
    let epoch_desc = EpochDescriptor {
        file: FILE,
        nonce: GenerationNonce::from_array([1; 16]),
        kind: EpochKind::Text,
        baseline: baseline_manifest.id().expect("id"),
        seed_profile: Some(profile::TEXT_PROFILE.to_owned()),
        git_oid: None,
    };
    let epoch = epoch_desc.id().expect("epoch id");
    let root = publish_meta(
        &db,
        meta(1, None, Vec::new(), 1),
        vec![create_op(epoch_desc)],
        1,
        1,
    );
    let seed_name = profile::seed_actor_name(&epoch);
    let mut doc = TextDoc::empty();
    let agent = doc.agent_id(&seed_name);
    doc.add_insert(agent, &[], 0, baseline_text);
    let seed_bytes = doc.encode();
    let (seed_manifest, seed_chunks) = frame_payload(
        &db,
        "native-change-v1",
        &NativeArtifact {
            epoch,
            profile: profile::TEXT_PROFILE.to_owned(),
            native: seed_bytes.clone(),
        }
        .encode()
        .expect("seed artifact"),
    );
    db.register_seed(&epoch, &seed_manifest, &seed_chunks)
        .expect("register seed");
    TextWorld {
        db,
        epoch,
        seed_native: seed_bytes,
        root,
    }
}

impl TextWorld {
    /// A native change inserting `text` at char `pos` over the seed plus the
    /// given chain of earlier changes (a dependent chain: each new op's base
    /// frontier includes all previous changes).
    fn change_after(&self, chain: &[Vec<u8>], pos: usize, text: &str) -> Vec<u8> {
        let mut full = TextDoc::load(&self.seed_native).expect("load seed");
        for change in chain {
            full.merge(change).expect("merge base change");
        }
        let frontier = full.local_frontier();
        let writer_name = profile::writer_actor_name(
            &profile::public_key(&KEY_SEED),
            &BRANCH,
            &WRITER_EPOCH,
            &self.epoch,
        );
        let agent = full.agent_id(&writer_name);
        full.add_insert(agent, &frontier, pos, text);
        full.encode_from(&frontier)
    }
}

fn generous_budget() -> ReadBudget {
    ReadBudget {
        max_steps: 1_000_000,
        max_output_bytes: 16 * 1024 * 1024,
        deadline: None,
    }
}

#[test]
fn snapshot_create_read_list_and_cas_negatives() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let root = publish_meta(&db, meta(1, None, Vec::new(), 1), Vec::new(), 1, 1);
    let v1 = db.version_of(&[root]).unwrap();
    let c2 = publish_meta(&db, meta(2, Some(root), vec![root], 2), Vec::new(), 2, 2);
    let v2 = db.version_of(&[c2]).unwrap();

    // Create + read + list.
    assert!(db.create_snapshot("rel-1", &v1).unwrap());
    assert_eq!(db.snapshot_version("rel-1").unwrap(), Some(v1));
    // Idempotent for the same (name, version).
    assert!(!db.create_snapshot("rel-1", &v1).unwrap());
    let pins = db.list_snapshots().unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].name, "rel-1");
    assert_eq!(pins[0].version, v1);

    // Spec-negative: CAS on name take-over with a different version.
    let err = db
        .create_snapshot("rel-1", &v2)
        .expect_err("take-over must fail");
    assert_eq!(err.code(), "precondition_failed");
    // The pin was not re-pointed.
    assert_eq!(db.snapshot_version("rel-1").unwrap(), Some(v1));

    // Spec-negative: unresolvable version is content_unavailable.
    let unknown = VersionId::from_array([0xee; 32]);
    let err = db
        .create_snapshot("ghost", &unknown)
        .expect_err("unknown version");
    assert_eq!(err.code(), "content_unavailable");
    assert_eq!(db.snapshot_version("ghost").unwrap(), None);

    // Spec-negative: empty name is invalid_encoding.
    let err = db.create_snapshot("", &v1).expect_err("empty name");
    assert_eq!(err.code(), "invalid_encoding");

    // Empty-space edge: the genesis initial (empty) frontier pins fine.
    let initial = db.branch_initial_version(&BRANCH).unwrap().unwrap();
    assert!(db.create_snapshot("genesis", &initial).unwrap());
    assert_eq!(db.snapshot_version("genesis").unwrap(), Some(initial));
    assert!(db.resolve_version(&initial).unwrap().heads.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn auto_checkpoint_at_threshold_and_cache_discard_invariant() {
    let dir = temp();
    let w = text_world_in(&dir, "hello");
    let c2_change = w.change_after(&[], 5, " world");
    let c2 = publish_native(
        &w.db,
        meta(2, Some(w.root), vec![w.root], 2),
        w.epoch,
        &c2_change,
        2,
        2,
    );
    let v2 = w.db.version_of(&[c2]).unwrap();

    // Q1 default is pinned and below this tiny history: no auto-checkpoint.
    assert_eq!(db_threshold(&w.db), DEFAULT_REPLAY_COST_THRESHOLD);
    assert!(!w.db.checkpoint_cached(&v2).unwrap());

    // Lower the local threshold past the closure size (2): the next commit
    // auto-generates a hash-checked checkpoint for the branch frontier.
    w.db.set_checkpoint_replay_threshold(2).unwrap();
    let c3_change = w.change_after(std::slice::from_ref(&c2_change), 11, "!");
    let c3 = publish_native(
        &w.db,
        meta(3, Some(c2), vec![c2], 3),
        w.epoch,
        &c3_change,
        3,
        3,
    );
    let v3 = w.db.version_of(&[c3]).unwrap();
    assert!(
        w.db.checkpoint_cached(&v3).unwrap(),
        "checkpoint generated at threshold"
    );

    // The checkpointed read matches full replay byte-for-byte.
    let via_cache =
        w.db.text_at_version(&v3, &w.epoch, Some(&generous_budget()))
            .unwrap();
    assert_eq!(via_cache, "hello world!");
    assert_eq!(
        via_cache,
        w.db.text_at_version(&v3, &w.epoch, None).unwrap()
    );

    // Cache-discard invariant (D3, test-pinned): rm -rf cache/ then identical
    // reads — canonical history alone answers.
    std::fs::remove_dir_all(dir.join("cache")).expect("rm cache");
    assert!(!w.db.checkpoint_cached(&v3).unwrap());
    assert_eq!(
        w.db.text_at_version(&v3, &w.epoch, Some(&generous_budget()))
            .unwrap(),
        via_cache
    );

    // Corrupt-cache fallback: regenerate, then garbage the file; the read
    // discards it and replays identically.
    let c4_change = w.change_after(&[c2_change.clone(), c3_change.clone()], 12, "?");
    let c4 = publish_native(
        &w.db,
        meta(4, Some(c3), vec![c3], 4),
        w.epoch,
        &c4_change,
        4,
        4,
    );
    let v4 = w.db.version_of(&[c4]).unwrap();
    assert!(w.db.checkpoint_cached(&v4).unwrap());
    let ckpt_dir = dir.join("cache").join("checkpoints");
    let files: Vec<_> = std::fs::read_dir(&ckpt_dir).unwrap().flatten().collect();
    for f in files {
        std::fs::write(f.path(), b"garbage-not-a-checkpoint").unwrap();
    }
    assert!(
        !w.db.checkpoint_cached(&v4).unwrap(),
        "corrupt cache discarded"
    );
    assert_eq!(
        w.db.text_at_version(&v4, &w.epoch, None).unwrap(),
        "hello world!?"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

fn db_threshold(db: &Database) -> u64 {
    db.checkpoint_replay_threshold()
}

#[test]
fn checkpoint_never_carries_post_cut_changes() {
    let dir = temp();
    let w = text_world_in(&dir, "hello");
    w.db.set_checkpoint_replay_threshold(2).unwrap();
    let c2_change = w.change_after(&[], 5, " world");
    let c2 = publish_native(
        &w.db,
        meta(2, Some(w.root), vec![w.root], 2),
        w.epoch,
        &c2_change,
        2,
        2,
    );
    let v2 = w.db.version_of(&[c2]).unwrap();
    assert!(w.db.checkpoint_cached(&v2).unwrap(), "v2 checkpointed");

    // Post-cut batch lands; the v2 view must not change.
    let c3_change = w.change_after(std::slice::from_ref(&c2_change), 11, "!");
    let c3 = publish_native(
        &w.db,
        meta(3, Some(c2), vec![c2], 3),
        w.epoch,
        &c3_change,
        3,
        3,
    );
    let v3 = w.db.version_of(&[c3]).unwrap();

    assert_eq!(
        w.db.text_at_version(&v2, &w.epoch, None).unwrap(),
        "hello world"
    );
    assert_eq!(
        w.db.text_at_version(&v3, &w.epoch, None).unwrap(),
        "hello world!"
    );

    // A snapshot pin at the old version reads the same after the pin.
    assert!(w.db.create_snapshot("cut", &v2).unwrap());
    assert_eq!(
        w.db.text_at_version(
            &w.db.snapshot_version("cut").unwrap().unwrap(),
            &w.epoch,
            None
        )
        .unwrap(),
        "hello world"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn single_batch_edge_checkpoint_and_threshold_persists() {
    let dir = temp();
    let w = text_world_in(&dir, "hi");
    let v1 = w.db.version_of(&[w.root]).unwrap();

    // Threshold 1: the single root batch is already at cost 1 — but the
    // checkpoint is generated on the NEXT commit after the threshold was
    // set; set it before publishing any further batch.
    w.db.set_checkpoint_replay_threshold(1).unwrap();
    let c2 = publish_meta(
        &w.db,
        meta(2, Some(w.root), vec![w.root], 2),
        Vec::new(),
        2,
        2,
    );
    let v2 = w.db.version_of(&[c2]).unwrap();
    assert!(
        w.db.checkpoint_cached(&v2).unwrap(),
        "single-batch-plus-one checkpointed at cost 2 >= 1"
    );
    assert_eq!(w.db.text_at_version(&v2, &w.epoch, None).unwrap(), "hi");
    // v1 (cost 1) also qualifies retroactively only via reads; its checkpoint
    // is generated when pinned:
    assert!(w.db.create_snapshot("v1", &v1).unwrap());
    assert!(
        w.db.checkpoint_cached(&v1).unwrap(),
        "snapshot create checkpoints the pin"
    );
    assert_eq!(w.db.text_at_version(&v1, &w.epoch, None).unwrap(), "hi");

    // Local-configurable: the threshold persists across reopen.
    drop(w.db);
    let db = Database::open_space(&dir, &SPACE).expect("reopen");
    assert_eq!(db.checkpoint_replay_threshold(), 1);
    assert_eq!(db.snapshot_version("v1").unwrap(), Some(v1));

    let _ = std::fs::remove_dir_all(&dir);
}
