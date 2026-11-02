//! I05 P6–P8 integration tests: epoch lifecycle, native admission (§12.5),
//! native text reads, position conversion.

use std::path::PathBuf;

use puyudb_core::identity::{
    ArtifactId, BatchId, BranchId, ChunkId, EpochId, FileId, GenerationNonce, RepositoryId,
    SpaceId, WriterEpoch,
};
use puyudb_core::profile::{
    self, ArtifactManifest, BatchBody, EpochDescriptor, EpochKind, MetadataArtifact,
    NativeArtifact, Operation, Path,
};
use puyudb_core::publication::{Error, Publication, Store};
use puyudb_crdt::dt_text::{TextDoc, byte_to_char, char_to_byte};

const SPACE: SpaceId = SpaceId::from_array([1; 16]);
const BRANCH: BranchId = BranchId::from_array([2; 16]);
const KEY_SEED: [u8; 32] = [3; 32];
const WRITER_EPOCH: WriterEpoch = WriterEpoch::from_array([4; 16]);

fn temp() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "puyu-i05-native-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp");
    path
}

fn frame(kind: &str, payload: &[u8]) -> (ArtifactManifest, Vec<(ChunkId, Vec<u8>)>) {
    let manifest = ArtifactManifest::for_payload(kind, payload).expect("manifest");
    let chunks = payload
        .chunks(profile::CHUNK_RAW_BYTES)
        .map(|c| (profile::chunk_id(c), c.to_vec()))
        .collect();
    (manifest, chunks)
}

fn stage_all(store: &Store, chunks: &[(ChunkId, Vec<u8>)]) -> Vec<ChunkId> {
    chunks
        .iter()
        .map(|(id, bytes)| {
            store.stage_chunk(*id, bytes).expect("stage");
            *id
        })
        .collect()
}

fn sign(body: &BatchBody) -> [u8; 64] {
    profile::sign_batch(&KEY_SEED, &body.space, &body.id().expect("id"))
}

fn text_epoch(file: u8, baseline: ArtifactId, nonce: u8) -> EpochDescriptor {
    EpochDescriptor {
        file: FileId::from_array([file; 16]),
        nonce: GenerationNonce::from_array([nonce; 16]),
        kind: EpochKind::Text,
        baseline,
        seed_profile: Some(profile::TEXT_PROFILE.to_owned()),
        git_oid: None,
    }
}

struct World {
    store: Store,
    file: FileId,
    epoch: EpochId,
    baseline_manifest: ArtifactManifest,
    seed_native: Vec<u8>,
    root: BatchId,
    last: BatchId,
}

/// A store with a text-baseline artifact staged, a root batch creating file 1
/// with a text epoch over baseline "hello", and the epoch seed registered.
fn world(baseline_text: &str) -> World {
    let dir = temp();
    let store = Store::open(&dir, &SPACE).expect("open");
    let (baseline_manifest, baseline_chunks) = frame("text-baseline-v1", baseline_text.as_bytes());
    let baseline_ids = stage_all(&store, &baseline_chunks);
    store
        .stage_artifact(&baseline_manifest, &baseline_ids)
        .expect("stage baseline");

    let epoch_desc = text_epoch(1, baseline_manifest.id().expect("id"), 1);
    let epoch = epoch_desc.id().expect("epoch id");
    let create = Operation::Create {
        repository: RepositoryId::from_array([9; 16]),
        file: FileId::from_array([1; 16]),
        path: Path::new("a.txt").expect("path"),
        mode: profile::ModeTag::Regular,
        epoch: epoch_desc,
    };
    let (meta_manifest, meta_chunks) = frame(
        "metadata-v1",
        &MetadataArtifact {
            operations: vec![create],
            context_assertions: vec![vec![1]],
        }
        .encode()
        .expect("metadata"),
    );
    let mut body = BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: 1,
        previous: None,
        parents: Vec::new(),
        lamport: 1,
        native_artifacts: Vec::new(),
        metadata: meta_manifest.id().expect("id"),
    };
    let signature = sign(&body);
    let meta_ids = stage_all(&store, &meta_chunks);
    let root = body.id().expect("id");
    store
        .publish(&Publication {
            body: &body,
            signature_key: &body.public_key,
            signature: &signature,
            artifacts: std::slice::from_ref(&meta_manifest),
            chunks: &meta_ids,
            peer: 1,
        })
        .expect("publish root");

    // Seed: the seed actor inserts the baseline text as the first op.
    let seed_name = profile::seed_actor_name(&epoch);
    let mut doc = TextDoc::empty();
    let agent = doc.agent_id(&seed_name);
    doc.add_insert(agent, &[], 0, baseline_text);
    let seed_bytes = doc.encode();
    let (seed_manifest, seed_chunks) = frame(
        "native-change-v1",
        &NativeArtifact {
            epoch,
            profile: profile::TEXT_PROFILE.to_owned(),
            native: seed_bytes.clone(),
        }
        .encode()
        .expect("native artifact"),
    );
    let seed_ids = stage_all(&store, &seed_chunks);
    store
        .register_seed(&epoch, &seed_manifest, &seed_ids)
        .expect("register seed");
    body.metadata = meta_manifest.id().expect("id");
    World {
        store,
        file: FileId::from_array([1; 16]),
        epoch,
        baseline_manifest,
        seed_native: seed_bytes,
        last: root,
        root,
    }
}

impl World {
    /// Publish a batch on the main stream carrying one native change artifact.
    fn publish_edit(
        &mut self,
        sequence: u64,
        lamport: u64,
        parents: Vec<BatchId>,
        change: &[u8],
        salt: u8,
    ) -> Result<BatchId, Error> {
        self.publish_edit_epoch(sequence, lamport, parents, self.epoch, change, salt)
    }

    fn publish_edit_epoch(
        &mut self,
        sequence: u64,
        lamport: u64,
        parents: Vec<BatchId>,
        epoch: EpochId,
        change: &[u8],
        salt: u8,
    ) -> Result<BatchId, Error> {
        let (change_manifest, change_chunks) = frame(
            "native-change-v1",
            &NativeArtifact {
                epoch,
                profile: profile::TEXT_PROFILE.to_owned(),
                native: change.to_vec(),
            }
            .encode()
            .expect("native artifact"),
        );
        let change_ids = stage_all(&self.store, &change_chunks);
        let (meta_manifest, meta_chunks) = frame(
            "metadata-v1",
            &MetadataArtifact {
                operations: Vec::new(),
                context_assertions: vec![vec![salt]],
            }
            .encode()
            .expect("metadata"),
        );
        let meta_ids = stage_all(&self.store, &meta_chunks);
        let body = BatchBody {
            space: SPACE,
            branch: BRANCH,
            public_key: profile::public_key(&KEY_SEED),
            writer_epoch: WRITER_EPOCH,
            sequence,
            previous: if sequence > 1 { Some(self.last) } else { None },
            parents,
            lamport,
            native_artifacts: vec![change_manifest.id().expect("id")],
            metadata: meta_manifest.id().expect("id"),
        };
        let signature = sign(&body);
        let id = body.id().expect("id");
        self.store.publish(&Publication {
            body: &body,
            signature_key: &body.public_key,
            signature: &signature,
            artifacts: &[change_manifest, meta_manifest],
            chunks: &[change_ids, meta_ids].concat(),
            peer: 1,
        })?;
        self.last = id;
        Ok(id)
    }
}

/// Build a native change artifact: load the seed, insert `text` at char pos
/// with the writer actor, encode the patch after the seed frontier.
fn change_artifact(w: &World, pos: usize, text: &str, key_seed: &[u8; 32]) -> Vec<u8> {
    let mut full = TextDoc::load(&w.seed_native).expect("load seed");
    let seed_frontier = full.local_frontier();
    let writer_name = profile::writer_actor_name(
        &profile::public_key(key_seed),
        &BRANCH,
        &WRITER_EPOCH,
        &w.epoch,
    );
    let agent = full.agent_id(&writer_name);
    full.add_insert(agent, &seed_frontier, pos, text);
    full.encode_from(&seed_frontier)
}

#[test]
fn seed_registration_and_native_edit_read() {
    let mut w = world("hello");
    // At the root cut the text is the baseline (seed only).
    assert_eq!(
        w.store.text_at(&[w.root], &w.epoch).expect("text at root"),
        "hello"
    );
    let change = change_artifact(&w, 5, " world", &KEY_SEED);
    let head = w
        .publish_edit(2, 2, vec![w.root], &change, 2)
        .expect("publish edit");
    assert_eq!(
        w.store.text_at(&[head], &w.epoch).expect("text at head"),
        "hello world"
    );
    // Historical read at the root version still shows the baseline.
    assert_eq!(
        w.store.text_at(&[w.root], &w.epoch).expect("historical"),
        "hello"
    );
}

#[test]
fn concurrent_edits_converge_across_arrival_orders() {
    let build = |dir: &std::path::Path| {
        let store = Store::open(dir, &SPACE).expect("open");
        let (baseline_manifest, baseline_chunks) = frame("text-baseline-v1", b"ac");
        let ids = stage_all(&store, &baseline_chunks);
        store
            .stage_artifact(&baseline_manifest, &ids)
            .expect("baseline");
        let epoch_desc = text_epoch(1, baseline_manifest.id().expect("id"), 1);
        let epoch = epoch_desc.id().expect("id");
        let create = Operation::Create {
            repository: RepositoryId::from_array([9; 16]),
            file: FileId::from_array([1; 16]),
            path: Path::new("a.txt").expect("path"),
            mode: profile::ModeTag::Regular,
            epoch: epoch_desc,
        };
        let (meta_manifest, meta_chunks) = frame(
            "metadata-v1",
            &MetadataArtifact {
                operations: vec![create],
                context_assertions: vec![vec![1]],
            }
            .encode()
            .expect("metadata"),
        );
        let body = BatchBody {
            space: SPACE,
            branch: BRANCH,
            public_key: profile::public_key(&KEY_SEED),
            writer_epoch: WRITER_EPOCH,
            sequence: 1,
            previous: None,
            parents: Vec::new(),
            lamport: 1,
            native_artifacts: Vec::new(),
            metadata: meta_manifest.id().expect("id"),
        };
        let signature = sign(&body);
        let meta_ids = stage_all(&store, &meta_chunks);
        let root = body.id().expect("id");
        store
            .publish(&Publication {
                body: &body,
                signature_key: &body.public_key,
                signature: &signature,
                artifacts: std::slice::from_ref(&meta_manifest),
                chunks: &meta_ids,
                peer: 1,
            })
            .expect("root");
        let seed_name = profile::seed_actor_name(&epoch);
        let mut doc = TextDoc::empty();
        let agent = doc.agent_id(&seed_name);
        doc.add_insert(agent, &[], 0, "ac");
        let (seed_manifest, seed_chunks) = frame(
            "native-change-v1",
            &NativeArtifact {
                epoch,
                profile: profile::TEXT_PROFILE.to_owned(),
                native: doc.encode(),
            }
            .encode()
            .expect("native artifact"),
        );
        let seed_ids = stage_all(&store, &seed_chunks);
        store
            .register_seed(&epoch, &seed_manifest, &seed_ids)
            .expect("seed");
        (store, root, epoch)
    };
    let mk_change = |store_seed_payload: &[u8],
                     epoch: &EpochId,
                     key: &[u8; 32],
                     pos: usize,
                     text: &str| {
        let mut full = TextDoc::load(store_seed_payload).expect("load");
        let frontier = full.local_frontier();
        let name =
            profile::writer_actor_name(&profile::public_key(key), &BRANCH, &WRITER_EPOCH, epoch);
        let agent = full.agent_id(&name);
        full.add_insert(agent, &frontier, pos, text);
        full.encode_from(&frontier)
    };
    let publish = |store: &Store,
                   root: &BatchId,
                   epoch: &EpochId,
                   key: &[u8; 32],
                   seq: u64,
                   previous: Option<BatchId>,
                   change: &[u8],
                   salt: u8| {
        let (cm, cc) = frame(
            "native-change-v1",
            &NativeArtifact {
                epoch: *epoch,
                profile: profile::TEXT_PROFILE.to_owned(),
                native: change.to_vec(),
            }
            .encode()
            .expect("native artifact"),
        );
        let cids = stage_all(store, &cc);
        let (mm, mc) = frame(
            "metadata-v1",
            &MetadataArtifact {
                operations: Vec::new(),
                context_assertions: vec![vec![salt]],
            }
            .encode()
            .expect("meta"),
        );
        let mids = stage_all(store, &mc);
        let body = BatchBody {
            space: SPACE,
            branch: BRANCH,
            public_key: profile::public_key(key),
            writer_epoch: WRITER_EPOCH,
            sequence: seq,
            previous,
            parents: vec![*root],
            lamport: 2,
            native_artifacts: vec![cm.id().expect("id")],
            metadata: mm.id().expect("id"),
        };
        let signature = profile::sign_batch(key, &body.space, &body.id().expect("id"));
        let id = body.id().expect("id");
        store
            .publish(&Publication {
                body: &body,
                signature_key: &body.public_key,
                signature: &signature,
                artifacts: &[cm, mm],
                chunks: &[cids, mids].concat(),
                peer: 1,
            })
            .expect("publish");
        id
    };

    let dir_a = temp();
    let (store_a, root_a, epoch_a) = build(&dir_a);
    let seed_payload: Vec<u8> = {
        let mut doc = TextDoc::empty();
        let agent = doc.agent_id(&profile::seed_actor_name(&epoch_a));
        doc.add_insert(agent, &[], 0, "ac");
        doc.encode()
    };
    let change_x = mk_change(&seed_payload, &epoch_a, &KEY_SEED, 1, "X");
    let change_y = mk_change(&seed_payload, &epoch_a, &[4; 32], 1, "Y");
    let a = publish(
        &store_a,
        &root_a,
        &epoch_a,
        &KEY_SEED,
        2,
        Some(root_a),
        &change_x,
        2,
    );
    let b = publish(&store_a, &root_a, &epoch_a, &[4; 32], 1, None, &change_y, 3);

    let dir_b = temp();
    let (store_b, root_b, epoch_b) = build(&dir_b);
    assert_eq!(root_a, root_b);
    assert_eq!(epoch_a, epoch_b);
    let b2 = publish(&store_b, &root_b, &epoch_b, &[4; 32], 1, None, &change_y, 3);
    let a2 = publish(
        &store_b,
        &root_b,
        &epoch_b,
        &KEY_SEED,
        2,
        Some(root_b),
        &change_x,
        2,
    );
    assert_eq!((a, b), (a2, b2));
    let text_a = store_a.text_at(&[a, b], &epoch_a).expect("text a");
    let text_b = store_b.text_at(&[a2, b2], &epoch_b).expect("text b");
    assert_eq!(text_a, text_b);
    assert_eq!(text_a.len(), 4); // "ac" + two inserted chars
}

#[test]
fn native_scope_negatives_are_rejected() {
    let mut w = world("hello");
    // Wrong writer actor: artifact built with a different key.
    let wrong_actor = change_artifact(&w, 5, " world", &[4; 32]);
    let err = w
        .publish_edit(2, 2, vec![w.root], &wrong_actor, 4)
        .expect_err("wrong actor must be rejected");
    assert!(matches!(err, Error::InvalidNativeScope(_)), "got {err}");

    // Parents outside the parent cut: an artifact whose start frontier
    // already includes an edit the cut does not contain.
    let mut full = TextDoc::load(&w.seed_native).expect("load");
    let seed_frontier = full.local_frontier();
    let writer_name = profile::writer_actor_name(
        &profile::public_key(&KEY_SEED),
        &BRANCH,
        &WRITER_EPOCH,
        &w.epoch,
    );
    let agent = full.agent_id(&writer_name);
    full.add_insert(agent, &seed_frontier, 0, "A");
    let after_a = full.local_frontier();
    full.add_insert(agent, &after_a, 1, "B");
    // Patch containing only the second insert, whose parent is the first.
    let late = full.encode_from(&after_a);
    let err = w
        .publish_edit(2, 2, vec![w.root], &late, 5)
        .expect_err("out-of-cut parent must be rejected");
    assert!(matches!(err, Error::InvalidNativeScope(_)), "got {err}");

    // A valid edit publishes; re-publishing the same artifact in another
    // batch is a known-operation rejection.
    let change = change_artifact(&w, 5, "!", &KEY_SEED);
    let head = w
        .publish_edit(2, 2, vec![w.root], &change, 6)
        .expect("valid edit");
    let err = w
        .publish_edit(3, 3, vec![head], &change, 7)
        .expect_err("known ops must be rejected");
    assert!(matches!(err, Error::InvalidNativeScope(_)), "got {err}");
    assert_eq!(w.store.text_at(&[head], &w.epoch).expect("text"), "hello!");
}

#[test]
fn native_artifact_for_undeclared_epoch_is_rejected() {
    let mut w = world("hello");
    // An epoch nobody declared.
    let bogus = EpochId::from_array([0xee; 32]);
    let seed_name = profile::seed_actor_name(&bogus);
    let mut doc = TextDoc::empty();
    let agent = doc.agent_id(&seed_name);
    doc.add_insert(agent, &[], 0, "hello");
    let change = {
        let mut full = TextDoc::load(&doc.encode()).expect("load");
        let frontier = full.local_frontier();
        let name = profile::writer_actor_name(
            &profile::public_key(&KEY_SEED),
            &BRANCH,
            &WRITER_EPOCH,
            &bogus,
        );
        let a = full.agent_id(&name);
        full.add_insert(a, &frontier, 0, "x");
        full.encode_from(&frontier)
    };
    let err = w
        .publish_edit_epoch(2, 2, vec![w.root], bogus, &change, 8)
        .expect_err("undeclared epoch must be rejected");
    assert!(matches!(err, Error::InvalidNativeScope(_)), "got {err}");
}

#[test]
fn seed_validation_negatives() {
    let dir = temp();
    let store = Store::open(&dir, &SPACE).expect("open");
    let (baseline_manifest, baseline_chunks) = frame("text-baseline-v1", b"hello");
    let ids = stage_all(&store, &baseline_chunks);
    store
        .stage_artifact(&baseline_manifest, &ids)
        .expect("baseline");
    let epoch_desc = text_epoch(1, baseline_manifest.id().expect("id"), 1);
    let epoch = epoch_desc.id().expect("id");
    let create = Operation::Create {
        repository: RepositoryId::from_array([9; 16]),
        file: FileId::from_array([1; 16]),
        path: Path::new("a.txt").expect("path"),
        mode: profile::ModeTag::Regular,
        epoch: epoch_desc,
    };
    let (meta_manifest, meta_chunks) = frame(
        "metadata-v1",
        &MetadataArtifact {
            operations: vec![create],
            context_assertions: vec![vec![1]],
        }
        .encode()
        .expect("metadata"),
    );
    let body = BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: 1,
        previous: None,
        parents: Vec::new(),
        lamport: 1,
        native_artifacts: Vec::new(),
        metadata: meta_manifest.id().expect("id"),
    };
    let signature = sign(&body);
    let meta_ids = stage_all(&store, &meta_chunks);
    store
        .publish(&Publication {
            body: &body,
            signature_key: &body.public_key,
            signature: &signature,
            artifacts: std::slice::from_ref(&meta_manifest),
            chunks: &meta_ids,
            peer: 1,
        })
        .expect("root");

    // Seed text that does not match the baseline is rejected.
    let mut doc = TextDoc::empty();
    let agent = doc.agent_id(&profile::seed_actor_name(&epoch));
    doc.add_insert(agent, &[], 0, "WRONG");
    let (bad_manifest, bad_chunks) = frame(
        "native-change-v1",
        &NativeArtifact {
            epoch,
            profile: profile::TEXT_PROFILE.to_owned(),
            native: doc.encode(),
        }
        .encode()
        .expect("native artifact"),
    );
    let bad_ids = stage_all(&store, &bad_chunks);
    assert!(
        store
            .register_seed(&epoch, &bad_manifest, &bad_ids)
            .is_err()
    );

    // Empty baseline requires a zero-length seed payload.
    let (empty_manifest, empty_chunks) = frame("text-baseline-v1", b"");
    let empty_ids = stage_all(&store, &empty_chunks);
    store
        .stage_artifact(&empty_manifest, &empty_ids)
        .expect("empty baseline");
    let epoch2_desc = text_epoch(2, empty_manifest.id().expect("id"), 2);
    let epoch2 = epoch2_desc.id().expect("id");
    let create2 = Operation::Create {
        repository: RepositoryId::from_array([9; 16]),
        file: FileId::from_array([2; 16]),
        path: Path::new("b.txt").expect("path"),
        mode: profile::ModeTag::Regular,
        epoch: epoch2_desc,
    };
    let (meta2_manifest, meta2_chunks) = frame(
        "metadata-v1",
        &MetadataArtifact {
            operations: vec![create2],
            context_assertions: vec![vec![2]],
        }
        .encode()
        .expect("metadata"),
    );
    let body2 = BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: 2,
        previous: Some(body.id().expect("id")),
        parents: vec![body.id().expect("id")],
        lamport: 2,
        native_artifacts: Vec::new(),
        metadata: meta2_manifest.id().expect("id"),
    };
    let signature2 = sign(&body2);
    let meta2_ids = stage_all(&store, &meta2_chunks);
    store
        .publish(&Publication {
            body: &body2,
            signature_key: &body2.public_key,
            signature: &signature2,
            artifacts: &[meta2_manifest],
            chunks: &meta2_ids,
            peer: 1,
        })
        .expect("second root");
    let (nonempty_seed, nonempty_chunks) = frame(
        "native-change-v1",
        &NativeArtifact {
            epoch: epoch2,
            profile: profile::TEXT_PROFILE.to_owned(),
            native: doc.encode(),
        }
        .encode()
        .expect("native artifact"),
    );
    let nonempty_ids = stage_all(&store, &nonempty_chunks);
    let err = store
        .register_seed(&epoch2, &nonempty_seed, &nonempty_ids)
        .expect_err("empty baseline requires empty seed");
    assert!(matches!(err, Error::InvalidNativeScope(_)), "got {err}");
    // The empty seed for an empty baseline is accepted.
    let empty_seed_payload = NativeArtifact {
        epoch: epoch2,
        profile: profile::TEXT_PROFILE.to_owned(),
        native: Vec::new(),
    }
    .encode()
    .expect("native artifact");
    let (empty_seed_manifest, empty_seed_chunks) = frame("native-change-v1", &empty_seed_payload);
    let empty_seed_ids = stage_all(&store, &empty_seed_chunks);
    store
        .register_seed(&epoch2, &empty_seed_manifest, &empty_seed_ids)
        .expect("empty seed ok");
    // Registering the same seed again is idempotent.
    store
        .register_seed(&epoch2, &empty_seed_manifest, &empty_seed_ids)
        .expect("idempotent seed");
    // A different seed payload for the same epoch is rejected.
    let other_payload = NativeArtifact {
        epoch: epoch2,
        profile: profile::TEXT_PROFILE.to_owned(),
        native: vec![0x00],
    }
    .encode()
    .expect("native artifact");
    let (other_manifest, other_chunks) = frame("native-change-v1", &other_payload);
    let other_ids = stage_all(&store, &other_chunks);
    assert!(
        store
            .register_seed(&epoch2, &other_manifest, &other_ids)
            .is_err()
    );
}

#[test]
fn epoch_history_is_retained_and_queryable_after_reset() {
    let w = world("hello");
    let epoch1 = w.epoch;
    // Reset the content lineage to a second text epoch over the same baseline.
    let epoch2_desc = text_epoch(1, w.baseline_manifest.id().expect("id"), 2);
    let epoch2 = epoch2_desc.id().expect("id");
    let (meta_manifest, meta_chunks) = frame(
        "metadata-v1",
        &MetadataArtifact {
            operations: vec![Operation::Epoch {
                file: w.file,
                epoch: epoch2_desc,
            }],
            context_assertions: vec![vec![9]],
        }
        .encode()
        .expect("metadata"),
    );
    let body = BatchBody {
        space: SPACE,
        branch: BRANCH,
        public_key: profile::public_key(&KEY_SEED),
        writer_epoch: WRITER_EPOCH,
        sequence: 2,
        previous: Some(w.root),
        parents: vec![w.root],
        lamport: 2,
        native_artifacts: Vec::new(),
        metadata: meta_manifest.id().expect("id"),
    };
    let signature = sign(&body);
    let meta_ids = stage_all(&w.store, &meta_chunks);
    let head = body.id().expect("id");
    w.store
        .publish(&Publication {
            body: &body,
            signature_key: &body.public_key,
            signature: &signature,
            artifacts: &[meta_manifest],
            chunks: &meta_ids,
            peer: 1,
        })
        .expect("publish reset");
    let epochs = w.store.epochs_at(&[head], w.file).expect("epochs");
    let ids: Vec<EpochId> = epochs.iter().map(|(id, _)| *id).collect();
    assert!(ids.contains(&epoch1));
    assert!(ids.contains(&epoch2));
    assert_eq!(ids.len(), 2);
}

#[test]
fn utf8_byte_offset_conversion_is_boundary_checked() {
    let text = "héllo"; // é is 2 bytes
    assert_eq!(byte_to_char(text, 0), Some(0));
    assert_eq!(byte_to_char(text, 1), Some(1));
    assert_eq!(byte_to_char(text, 2), None); // middle of é
    assert_eq!(byte_to_char(text, 3), Some(2));
    assert_eq!(byte_to_char(text, text.len()), Some(5));
    assert_eq!(byte_to_char(text, text.len() + 1), None);
    assert_eq!(char_to_byte(text, 0), Some(0));
    assert_eq!(char_to_byte(text, 2), Some(3));
    assert_eq!(char_to_byte(text, 5), Some(text.len()));
    assert_eq!(char_to_byte(text, 6), None);
}
