// SPDX-License-Identifier: Apache-2.0

//! I08 Phase 1 (P1+P2) integration tests: VersionId resolution and frontier
//! validation (spec §5), DAG closure/ancestry queries, native-head mapping,
//! branch current/initial versions (§8.3), snapshot-pin read resolution, and
//! budgeted old reads (D1/D5) — all through the public `Database` facade.
//!
//! Spec-negative tests are mandatory here: unnormalized/redundant frontiers
//! are `invalid_encoding`, unknown versions are `content_unavailable`, and no
//! post-cut batch may leak into an old view.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use puyudb_core::api::{Database, ReadBudget};
use puyudb_core::identity::{
    ArtifactId, BatchId, BranchId, ChunkId, EpochId, FileId, GenerationNonce, RepositoryId,
    SpaceId, VersionId, WriterEpoch,
};
use puyudb_core::profile::{
    self, ArtifactManifest, BatchBody, EpochDescriptor, EpochKind, Frontier, LiveTag,
    MetadataArtifact, ModeTag, NativeArtifact, Operation, Path,
};
use puyudb_core::projection::Field;
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
        "puyu-i08-history-{}-{}",
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

fn body(
    sequence: u64,
    previous: Option<BatchId>,
    parents: Vec<BatchId>,
    lamport: u64,
    metadata: ArtifactId,
) -> BatchBody {
    body_on(
        &WRITER_EPOCH,
        sequence,
        previous,
        parents,
        lamport,
        metadata,
    )
}

fn body_on(
    writer_epoch: &WriterEpoch,
    sequence: u64,
    previous: Option<BatchId>,
    parents: Vec<BatchId>,
    lamport: u64,
    metadata: ArtifactId,
) -> BatchBody {
    BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: *writer_epoch,
        sequence,
        previous,
        parents,
        lamport,
        native_artifacts: Vec::new(),
        metadata,
    }
}

/// Publish `body` with the given staged artifacts through the facade's keyed
/// transaction path, CAS-ing on the branch's current frontier.
fn publish(
    db: &Database,
    body: &BatchBody,
    artifacts: &[ArtifactManifest],
    chunks: &[ChunkId],
    key: u8,
) -> BatchId {
    let signature = profile::sign_batch(&KEY_SEED, &body.space, &body.id().expect("id"));
    let expected = db.branch_frontier(&BRANCH).expect("frontier");
    let receipt = db
        .transact(
            b"i08-test",
            &[key],
            &expected,
            &Publication {
                body,
                signature_key: &body.public_key,
                signature: &signature,
                artifacts,
                chunks,
                peer: 1,
            },
        )
        .expect("publish");
    receipt.batch_id
}

/// Stream position of one batch to publish.
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

/// Publish a metadata-only batch on the single-parent chain.
fn publish_meta(db: &Database, m: Meta, ops: Vec<Operation>, salt: u8, key: u8) -> BatchId {
    publish_meta_on(&WRITER_EPOCH, db, m, ops, salt, key)
}

/// Same, on an explicit writer stream (§13.1: a forked child whose `previous`
/// is outside its parent cut allocates a fresh writer epoch).
fn publish_meta_on(
    writer_epoch: &WriterEpoch,
    db: &Database,
    m: Meta,
    ops: Vec<Operation>,
    salt: u8,
    key: u8,
) -> BatchId {
    let (manifest, chunks) = frame_payload(db, "metadata-v1", &ops_payload(ops, salt));
    let metadata = manifest.id().expect("id");
    let b = body_on(
        writer_epoch,
        m.sequence,
        m.previous,
        m.parents,
        m.lamport,
        metadata,
    );
    let id = b.id().expect("id");
    publish(db, &b, std::slice::from_ref(&manifest), &chunks, key);
    id
}

/// Publish a batch carrying one native change artifact plus metadata.
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
    let mut b = body(m.sequence, m.previous, m.parents, m.lamport, metadata);
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

fn text_epoch_descriptor(baseline: ArtifactId) -> EpochDescriptor {
    EpochDescriptor {
        file: FILE,
        nonce: GenerationNonce::from_array([1; 16]),
        kind: EpochKind::Text,
        baseline,
        seed_profile: Some(profile::TEXT_PROFILE.to_owned()),
        git_oid: None,
    }
}

/// A space whose root batch creates `FILE` with a text epoch over
/// `baseline_text`, with the epoch seed registered.
struct TextWorld {
    db: Database,
    epoch: EpochId,
    seed_native: Vec<u8>,
    root: BatchId,
}

fn text_world(baseline_text: &str) -> TextWorld {
    text_world_in(&temp(), baseline_text)
}

fn text_world_in(dir: &std::path::Path, baseline_text: &str) -> TextWorld {
    let db = Database::create_space(dir, &SPACE).expect("create");
    let (baseline_manifest, baseline_chunks) =
        frame_payload(&db, "text-baseline-v1", baseline_text.as_bytes());
    db.stage_artifact(&baseline_manifest, &baseline_chunks)
        .expect("stage baseline");
    let epoch_desc = text_epoch_descriptor(baseline_manifest.id().expect("id"));
    let epoch = epoch_desc.id().expect("epoch id");
    let root = publish_meta(
        &db,
        meta(1, None, Vec::new(), 1),
        vec![create_op(epoch_desc)],
        1,
        1,
    );
    // Seed: the seed actor inserts the baseline text as the first op.
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
    /// A native change inserting `text` at char `pos` over the seed frontier.
    fn change(&self, pos: usize, text: &str) -> Vec<u8> {
        self.change_after(&[], pos, text)
    }

    /// A native change inserting `text` at char `pos` over `base_change`
    /// (a dependent chain: the new op's base frontier includes the base
    /// change's operations).
    fn change_after(&self, base_change: &[u8], pos: usize, text: &str) -> Vec<u8> {
        let mut full = TextDoc::load(&self.seed_native).expect("load seed");
        if !base_change.is_empty() {
            full.merge(base_change).expect("merge base change");
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
        deadline: Some(Instant::now() + Duration::from_secs(60)),
    }
}

#[test]
fn version_roundtrip_and_unknown_version_is_content_unavailable() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let root = publish_meta(&db, meta(1, None, Vec::new(), 1), Vec::new(), 1, 1);

    let current = db.branch_current_version(&BRANCH).expect("current");
    assert_eq!(current, Some(db.version_of(&[root]).expect("version_of")));
    let frontier = db
        .resolve_version(&current.expect("some"))
        .expect("resolve");
    assert_eq!(frontier.heads, vec![root]);
    assert_eq!(frontier.space, SPACE);
    assert_eq!(db.closure(&current.unwrap()).expect("closure"), vec![root]);

    // The genesis batch's parent cut is the empty frontier: the branch's
    // initial version (§8.3), stored and resolvable.
    let initial = db.branch_initial_version(&BRANCH).expect("initial");
    assert_eq!(initial, Some(db.version_of(&[]).expect("empty version")));
    let initial_frontier = db
        .resolve_version(&initial.expect("some"))
        .expect("resolve initial");
    assert!(initial_frontier.heads.is_empty());

    // Spec §5: a missing object is VersionUnavailable → content_unavailable.
    let unknown = VersionId::from_array([0xee; 32]);
    let err = db.resolve_version(&unknown).expect_err("unknown version");
    assert_eq!(err.code(), "content_unavailable");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn version_of_rejects_non_visible_cut() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let ghost = BatchId::from_array([0x77; 32]);
    let err = db.version_of(&[ghost]).expect_err("ghost cut");
    assert_eq!(err.code(), "invalid_encoding");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn redundant_frontier_head_is_invalid_encoding_never_normalized() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let root = publish_meta(&db, meta(1, None, Vec::new(), 1), Vec::new(), 1, 1);
    let child = publish_meta(&db, meta(2, Some(root), vec![root], 2), Vec::new(), 2, 2);

    // [root, child] sorted by bytes: root is an ancestor of child, so the
    // list is redundant and a consumer must reject it (D2, spec §5).
    let mut heads = vec![root, child];
    heads.sort_unstable();
    let redundant = Frontier {
        space: SPACE,
        heads,
    };
    let err = db
        .validate_frontier(&redundant)
        .expect_err("redundant head");
    assert_eq!(err.code(), "invalid_encoding");

    // The minimal frontier validates.
    let minimal = Frontier {
        space: SPACE,
        heads: vec![child],
    };
    db.validate_frontier(&minimal).expect("minimal ok");

    // Unsorted/duplicate head lists fail at decode, before any DAG check:
    // write the two heads in descending byte order.
    let (hi, lo) = if root > child {
        (root, child)
    } else {
        (child, root)
    };
    let mut bytes = Vec::new();
    puyudb_core::canon::write_array_header(&mut bytes, 3).unwrap();
    puyudb_core::canon::write_uint(&mut bytes, profile::PROFILE_VERSION).unwrap();
    puyudb_core::canon::write_bytes(&mut bytes, SPACE.as_bytes()).unwrap();
    puyudb_core::canon::write_array_header(&mut bytes, 2).unwrap();
    puyudb_core::canon::write_bytes(&mut bytes, hi.as_bytes()).unwrap();
    puyudb_core::canon::write_bytes(&mut bytes, lo.as_bytes()).unwrap();
    assert!(
        Frontier::decode(&bytes).is_err(),
        "unsorted heads must fail"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closure_and_ancestry_over_a_fork() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let root = publish_meta(&db, meta(1, None, Vec::new(), 1), Vec::new(), 1, 1);
    let left = publish_meta(&db, meta(2, Some(root), vec![root], 2), Vec::new(), 2, 2);
    // A second child of `root` from the same stream would be a causal_fork
    // (§13.1): fresh writer epoch, sequence restarts at 1, parents may still
    // declare the captured base cut.
    const FORK_EPOCH: WriterEpoch = WriterEpoch::from_array([5; 16]);
    let right = publish_meta_on(
        &FORK_EPOCH,
        &db,
        meta(1, None, vec![root], 2),
        Vec::new(),
        3,
        3,
    );

    // Producer-side: version_of normalizes the raw cut [root, left, right]
    // down to the minimal frontier [left, right], which is the branch's
    // current version.
    let current = db
        .branch_current_version(&BRANCH)
        .expect("current")
        .unwrap();
    assert_eq!(db.version_of(&[root, left, right]).unwrap(), current);

    let closure = db.closure(&current).expect("closure");
    assert_eq!(closure.len(), 3);
    assert!(closure.contains(&root) && closure.contains(&left) && closure.contains(&right));

    assert!(db.is_ancestor_of(&root, &left).unwrap());
    assert!(db.is_ancestor_of(&root, &right).unwrap());
    assert!(!db.is_ancestor_of(&left, &right).unwrap());
    assert!(
        db.is_ancestor_of(&right, &right).unwrap(),
        "closure includes the head itself"
    );

    // An old version's closure does not grow when later batches land (D1).
    let v1 = db.version_of(&[root]).unwrap();
    assert_eq!(db.closure(&v1).unwrap(), vec![root]);
    let after = publish_meta(&db, meta(3, Some(left), vec![left], 3), Vec::new(), 4, 4);
    assert_eq!(
        db.closure(&v1).unwrap(),
        vec![root],
        "no future leak into closure"
    );
    assert!(db.is_ancestor_of(&after, &after).unwrap());
    assert!(!db.is_ancestor_of(&after, &v1_head(&db, &v1)).unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}

fn v1_head(db: &Database, version: &VersionId) -> BatchId {
    db.resolve_version(version).unwrap().heads[0]
}

#[test]
fn native_heads_map_closure_and_text_reads_do_not_leak_future() {
    let w = text_world("hello");
    let v1 = w.db.version_of(&[w.root]).unwrap();

    let c2_change = w.change(5, " world");
    let c2 = publish_native(
        &w.db,
        meta(2, Some(w.root), vec![w.root], 2),
        w.epoch,
        &c2_change,
        2,
        2,
    );
    let v2 = w.db.branch_current_version(&BRANCH).unwrap().unwrap();
    let c3_change = w.change_after(&c2_change, 11, "!");
    let c3 = publish_native(
        &w.db,
        meta(3, Some(c2), vec![c2], 3),
        w.epoch,
        &c3_change,
        3,
        3,
    );
    let v3 = w.db.branch_current_version(&BRANCH).unwrap().unwrap();

    // Native-head mapping per version's closure (§6.3 native_changes).
    let heads_v1 = w.db.native_heads(&v1).expect("heads v1");
    assert_eq!(heads_v1.len(), 1);
    assert_eq!(heads_v1[0].epoch, w.epoch);
    assert_eq!(heads_v1[0].producer, None, "seed row has no producer");
    let heads_v2 = w.db.native_heads(&v2).expect("heads v2");
    assert_eq!(heads_v2.len(), 2);
    assert!(heads_v2.iter().any(|h| h.producer == Some(c2)));
    assert!(
        heads_v2.iter().all(|h| h.producer != Some(c3)),
        "post-cut producer excluded"
    );

    // Future-leak guard: text at v1/v2 is byte-identical before and after c3.
    assert_eq!(
        w.db.text_at_version(&v1, &w.epoch, Some(&generous_budget()))
            .unwrap(),
        "hello"
    );
    assert_eq!(
        w.db.text_at_version(&v2, &w.epoch, Some(&generous_budget()))
            .unwrap(),
        "hello world"
    );
    assert_eq!(
        w.db.text_at_version(&v3, &w.epoch, Some(&generous_budget()))
            .unwrap(),
        "hello world!"
    );
    // The version read agrees with the raw-cut read at the same heads.
    let heads_v2 = w.db.resolve_version(&v2).unwrap().heads;
    assert_eq!(
        w.db.text_at_version(&v2, &w.epoch, None).unwrap(),
        w.db.text_at(&heads_v2, &w.epoch).unwrap()
    );
}

#[test]
fn historical_path_views_and_deletion_stay_queryable() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let epoch_desc = text_epoch_descriptor(ArtifactId::from_array([0xab; 32]));
    // The epoch descriptor's baseline need not be staged for metadata-only
    // reads; create the file without a seed.
    let root = publish_meta(
        &db,
        meta(1, None, Vec::new(), 1),
        vec![create_op(epoch_desc)],
        1,
        1,
    );
    let v1 = db.version_of(&[root]).unwrap();

    let renamed = publish_meta(
        &db,
        meta(2, Some(root), vec![root], 2),
        vec![Operation::Path {
            file: FILE,
            path: Path::new("b.txt").expect("path"),
        }],
        2,
        2,
    );
    let v2 = db.version_of(&[renamed]).unwrap();

    let deleted = publish_meta(
        &db,
        meta(3, Some(renamed), vec![renamed], 3),
        vec![Operation::Live {
            file: FILE,
            live: LiveTag::Deleted,
        }],
        3,
        3,
    );
    let v3 = db.version_of(&[deleted]).unwrap();

    // Historical path views obey the old path register (§11).
    let paths_v1 = db.paths_at_version(&v1).unwrap();
    assert_eq!(paths_v1.len(), 1);
    assert_eq!(paths_v1[0].0.as_str(), "a.txt");
    let paths_v2 = db.paths_at_version(&v2).unwrap();
    assert_eq!(paths_v2[0].0.as_str(), "b.txt");

    // Deleted content stays queryable at the deleting version (§5.7): the
    // file is present with a Deleted liveness alternative.
    let files_v3 = db.files_at_version(&v3).unwrap();
    assert_eq!(files_v3.len(), 1);
    assert_eq!(files_v3[0].file, FILE);
    assert_eq!(files_v3[0].live, vec![LiveTag::Deleted]);
    // ...and at the older version the file is alive at its old path.
    let files_v1 = db.files_at_version(&v1).unwrap();
    assert_eq!(files_v1[0].live, vec![LiveTag::Alive]);

    // Register evaluation at the base cut: alternatives in stable bytewise
    // write-identity order.
    let register = db.register_at_version(&v1).unwrap();
    let mut path_writes: Vec<_> = register.iter().filter(|w| w.field == Field::Path).collect();
    path_writes.sort_by_key(|w| w.write);
    assert_eq!(path_writes.len(), 1);
    assert_eq!(
        path_writes[0].value,
        puyudb_core::projection::FieldValue::Path(Path::new("a.txt").unwrap())
    );

    // Semantic manifest at a version equals the raw-cut manifest (T02 hash).
    let m_version = db.semantic_manifest_at_version(&v2).unwrap();
    let heads_v2 = db.resolve_version(&v2).unwrap().heads;
    let m_cut = db.semantic_manifest_at(&heads_v2).unwrap();
    assert_eq!(m_version.id().unwrap(), m_cut.id().unwrap());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn concurrent_path_alternatives_are_stable_bytewise_ordered() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let epoch_desc = text_epoch_descriptor(ArtifactId::from_array([0xab; 32]));
    let root = publish_meta(
        &db,
        meta(1, None, Vec::new(), 1),
        vec![create_op(epoch_desc)],
        1,
        1,
    );

    // Two concurrent path assignments over the same base cut.
    let left = publish_meta(
        &db,
        meta(2, Some(root), vec![root], 2),
        vec![Operation::Path {
            file: FILE,
            path: Path::new("left.txt").expect("path"),
        }],
        2,
        2,
    );
    // Concurrent writer: fresh writer epoch over the same base cut (§13.1).
    const FORK_EPOCH: WriterEpoch = WriterEpoch::from_array([5; 16]);
    let right = publish_meta_on(
        &FORK_EPOCH,
        &db,
        meta(1, None, vec![root], 2),
        vec![Operation::Path {
            file: FILE,
            path: Path::new("right.txt").expect("path"),
        }],
        3,
        3,
    );
    // No-content join over both heads (parents sorted unique, §13).
    let mut parents = vec![left, right];
    parents.sort_unstable();
    let join = publish_meta(&db, meta(3, Some(left), parents, 3), Vec::new(), 4, 4);
    let v_join = db.version_of(&[join]).unwrap();

    let register = db.register_at_version(&v_join).unwrap();
    let mut path_writes: Vec<_> = register
        .iter()
        .filter(|w| w.field == Field::Path && w.file == FILE)
        .collect();
    // Alternatives in stable bytewise identity order (smallest first).
    path_writes.sort_by_key(|w| w.write);
    assert_eq!(path_writes.len(), 2);
    let ids: Vec<_> = path_writes.iter().map(|w| w.write).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(
        ids, sorted,
        "alternatives returned in stable bytewise order"
    );
    let paths: Vec<_> = path_writes
        .iter()
        .map(|w| match &w.value {
            puyudb_core::projection::FieldValue::Path(p) => p.as_str().to_owned(),
            other => panic!("unexpected path value {other:?}"),
        })
        .collect();
    assert!(paths.contains(&"left.txt".to_owned()) && paths.contains(&"right.txt".to_owned()));

    // The conflict is recomputed at the version, never an arrival-order flag.
    let conflicts = db.conflicts_at_version(&v_join).unwrap();
    assert!(
        !conflicts.is_empty(),
        "concurrent path writes form a conflict"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn read_budget_cancels_typed_never_partial() {
    let w = text_world("hello");
    let c2 = publish_native(
        &w.db,
        meta(2, Some(w.root), vec![w.root], 2),
        w.epoch,
        &w.change(5, " world"),
        2,
        2,
    );
    let v2 = w.db.version_of(&[c2]).unwrap();

    // Generous budget succeeds and matches the unbounded read.
    assert_eq!(
        w.db.text_at_version(&v2, &w.epoch, Some(&generous_budget()))
            .unwrap(),
        w.db.text_at_version(&v2, &w.epoch, None).unwrap()
    );

    // Zero step budget: the typed cancellation fires, never a partial text.
    // Spec-negative per the final maintainer ruling: an exhausted/cancelled
    // budget read is §14 precondition_failed.
    let zero_steps = ReadBudget {
        max_steps: 0,
        max_output_bytes: u64::MAX,
        deadline: None,
    };
    let err =
        w.db.text_at_version(&v2, &w.epoch, Some(&zero_steps))
            .expect_err("zero-step budget must cancel");
    assert_eq!(err.code(), "precondition_failed");

    // Output-byte budget below the result length.
    let tiny_bytes = ReadBudget {
        max_steps: u64::MAX,
        max_output_bytes: 1,
        deadline: None,
    };
    let err =
        w.db.text_at_version(&v2, &w.epoch, Some(&tiny_bytes))
            .expect_err("tiny output budget must cancel");
    assert_eq!(err.code(), "precondition_failed");

    // Deadline already in the past.
    let past = ReadBudget {
        max_steps: u64::MAX,
        max_output_bytes: u64::MAX,
        deadline: Some(Instant::now() - Duration::from_secs(1)),
    };
    let err =
        w.db.text_at_version(&v2, &w.epoch, Some(&past))
            .expect_err("past deadline must cancel");
    assert_eq!(err.code(), "precondition_failed");
}

#[test]
fn snapshot_pin_read_resolution_is_none_until_pinned() {
    let dir = temp();
    let db = Database::create_space(&dir, &SPACE).expect("create");
    let _root = publish_meta(&db, meta(1, None, Vec::new(), 1), Vec::new(), 1, 1);
    // Phase 1 exposes read resolution only; the table starts empty (P3
    // writes pins).
    assert_eq!(db.snapshot_version("release-1").unwrap(), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn versions_survive_reload() {
    let dir = temp();
    let (v1, v2, text_v2) = {
        let w = text_world_in(&dir, "hello");
        let v1 = w.db.version_of(&[w.root]).unwrap();
        let c2 = publish_native(
            &w.db,
            meta(2, Some(w.root), vec![w.root], 2),
            w.epoch,
            &w.change(5, " world"),
            2,
            2,
        );
        let v2 = w.db.version_of(&[c2]).unwrap();
        let text = w.db.text_at_version(&v2, &w.epoch, None).unwrap();
        drop(w.db);
        (v1, v2, text)
    };
    let db = Database::open_space(&dir, &SPACE).expect("reopen");
    // Every retained version resolves and reads identically after reload.
    assert_eq!(db.resolve_version(&v1).unwrap().heads.len(), 1);
    assert_eq!(
        db.text_at_version(&v2, &epoch_of(&db, &v1), None).unwrap(),
        text_v2
    );
    assert_eq!(db.branch_current_version(&BRANCH).unwrap(), Some(v2));
    let _ = std::fs::remove_dir_all(&dir);
}

fn epoch_of(db: &Database, version: &VersionId) -> EpochId {
    db.epochs_at_version(version, FILE).unwrap()[0].0
}
