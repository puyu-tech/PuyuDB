// SPDX-License-Identifier: Apache-2.0

//! Shared I08 test helpers: a facade-driven space with a text epoch, batch
//! builders, and publication through the keyed transaction path.

#![allow(dead_code)]

use std::path::PathBuf;

use puyudb_core::api::{Database, ReadBudget};
use puyudb_core::identity::{
    ArtifactId, BatchId, BranchId, ChunkId, EpochId, FileId, GenerationNonce, RepositoryId,
    SpaceId, WriterEpoch,
};
use puyudb_core::profile::{
    self, ArtifactManifest, BatchBody, EpochDescriptor, EpochKind, MetadataArtifact, ModeTag,
    NativeArtifact, Operation, Path,
};
use puyudb_core::publication::Publication;
use puyudb_crdt::dt_text::TextDoc;

pub const SPACE: SpaceId = SpaceId::from_array([1; 16]);
pub const BRANCH: BranchId = BranchId::from_array([2; 16]);
pub const KEY_SEED: [u8; 32] = [3; 32];
pub const WRITER_EPOCH: WriterEpoch = WriterEpoch::from_array([4; 16]);
pub const FILE: FileId = FileId::from_array([1; 16]);

pub fn temp() -> PathBuf {
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

pub fn frame_payload(
    db: &Database,
    kind: &str,
    payload: &[u8],
) -> (ArtifactManifest, Vec<ChunkId>) {
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

pub fn ops_payload(ops: Vec<Operation>, salt: u8) -> Vec<u8> {
    MetadataArtifact {
        operations: ops,
        context_assertions: vec![vec![salt]],
    }
    .encode()
    .expect("metadata artifact")
}

pub struct Meta {
    sequence: u64,
    previous: Option<BatchId>,
    parents: Vec<BatchId>,
    lamport: u64,
}

pub fn meta(sequence: u64, previous: Option<BatchId>, parents: Vec<BatchId>, lamport: u64) -> Meta {
    Meta {
        sequence,
        previous,
        parents,
        lamport,
    }
}

pub fn body(m: Meta, metadata: ArtifactId) -> BatchBody {
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

pub fn publish(
    db: &Database,
    b: &BatchBody,
    artifacts: &[ArtifactManifest],
    chunks: &[ChunkId],
    key: &[u8],
) -> BatchId {
    let signature = profile::sign_batch(&KEY_SEED, &b.space, &b.id().expect("id"));
    let expected = db.branch_frontier(&BRANCH).expect("frontier");
    let receipt = db
        .transact(
            b"i08-p3",
            key,
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

pub fn publish_meta(db: &Database, m: Meta, ops: Vec<Operation>, salt: u8, key: &[u8]) -> BatchId {
    let (manifest, chunks) = frame_payload(db, "metadata-v1", &ops_payload(ops, salt));
    let metadata = manifest.id().expect("id");
    let b = body(m, metadata);
    let id = b.id().expect("id");
    publish(db, &b, std::slice::from_ref(&manifest), &chunks, key);
    id
}

pub fn publish_native(
    db: &Database,
    m: Meta,
    epoch: EpochId,
    change: &[u8],
    salt: u8,
    key: &[u8],
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

pub fn create_op(epoch: EpochDescriptor) -> Operation {
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
pub struct TextWorld {
    pub db: Database,
    pub epoch: EpochId,
    pub seed_native: Vec<u8>,
    pub root: BatchId,
}

pub fn text_world_in(dir: &std::path::Path, baseline_text: &str) -> TextWorld {
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
        &[1],
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
    pub fn change_after(&self, chain: &[Vec<u8>], pos: usize, text: &str) -> Vec<u8> {
        let mut full = TextDoc::load(&self.seed_native).expect("load seed");
        for change in chain {
            if !change.is_empty() {
                full.merge(change).expect("merge base change");
            }
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

    /// Produce the next change from a running document (O(doc) per step, no
    /// chain re-merge): insert `text` at char `pos` over the doc's frontier.
    pub fn next_change(&self, doc: &mut TextDoc, pos: usize, text: &str) -> Vec<u8> {
        let frontier = doc.local_frontier();
        let writer_name = profile::writer_actor_name(
            &profile::public_key(&KEY_SEED),
            &BRANCH,
            &WRITER_EPOCH,
            &self.epoch,
        );
        let agent = doc.agent_id(&writer_name);
        doc.add_insert(agent, &frontier, pos, text);
        doc.encode_from(&frontier)
    }
}

pub fn generous_budget() -> ReadBudget {
    ReadBudget {
        max_steps: 1_000_000,
        max_output_bytes: 16 * 1024 * 1024,
        deadline: None,
    }
}
