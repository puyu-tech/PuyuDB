// SPDX-License-Identifier: Apache-2.0

//! Synthetic candidate-fit controls, not production identity allocation,
//! authenticated ownership, consensus, or the proposed 33-byte AM actor profile.

use std::{collections::BTreeMap, ops::Range};

use loro::{
    ContainerID, ContainerType, EncodedBlobMode, ExportMode, Frontiers, JsonOpContent, JsonTextOp,
    LoroDoc, VersionVector,
};
use sha2::{Digest, Sha256};

pub(crate) const SEED_PEER: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FixtureWriter {
    pub public_key: [u8; 32],
    pub branch: [u8; 16],
    pub writer_epoch: [u8; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    epoch: [u8; 32],
    writer: FixtureWriter,
    peer: u64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FitError {
    #[error("native peer is already bound to another full fixture writer")]
    PeerCollision,
    #[error("full fixture writer is already bound to another native peer")]
    PeerReassignment,
    #[error("ordinary writer cannot use the reserved fixture seed peer")]
    ReservedSeedPeer,
    #[error("{0}")]
    Rejected(&'static str),
    #[error(transparent)]
    Native(#[from] loro::LoroError),
    #[error(transparent)]
    Encoding(#[from] loro::LoroEncodeError),
}

/// A caller-supplied peer-assignment experiment. It rejects collisions rather
/// than truncating keys, and does not specify how independent writers agree IDs.
pub(crate) struct EpochPeers {
    epoch: [u8; 32],
    owners: BTreeMap<u64, FixtureWriter>,
}

impl EpochPeers {
    pub(crate) fn new(epoch: [u8; 32]) -> Self {
        Self {
            epoch,
            owners: BTreeMap::new(),
        }
    }

    pub(crate) fn bind(&mut self, writer: FixtureWriter, peer: u64) -> Result<Binding> {
        if peer == SEED_PEER {
            return Err(FitError::ReservedSeedPeer);
        }
        if self.owners.get(&peer).is_some_and(|owner| owner != &writer) {
            return Err(FitError::PeerCollision);
        }
        if self
            .owners
            .iter()
            .any(|(id, owner)| owner == &writer && *id != peer)
        {
            return Err(FitError::PeerReassignment);
        }
        self.owners.insert(peer, writer);
        Ok(Binding {
            epoch: self.epoch,
            writer,
            peer,
        })
    }
}

pub(crate) type Result<T> = std::result::Result<T, FitError>;

pub(crate) fn document(peer: u64) -> Result<LoroDoc> {
    let doc = LoroDoc::new();
    doc.set_peer_id(peer)?;
    doc.set_record_timestamp(false);
    doc.set_change_merge_interval(-1);
    Ok(doc)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn message(binding: &Binding, sequence: u64) -> String {
    format!(
        "puyudb/loro-fit/i02 key={} branch={} writer_epoch={} epoch={} seq={} ordinal=0",
        hex(&binding.writer.public_key),
        hex(&binding.writer.branch),
        hex(&binding.writer.writer_epoch),
        hex(&binding.epoch),
        sequence,
    )
}

fn text_container() -> ContainerID {
    ContainerID::Root {
        name: "text".into(),
        container_type: ContainerType::Text,
    }
}

/// Immutable, already trusted fixture snapshot plus its declared native intent
/// cut. Loading/forking cannot implicitly commit a caller's live document.
#[derive(Clone)]
pub(crate) struct FixtureBase {
    epoch: [u8; 32],
    snapshot: Vec<u8>,
    cut: Frontiers,
}

impl FixtureBase {
    pub(crate) fn seed(epoch: [u8; 32], baseline: &str) -> Result<Self> {
        let doc = document(SEED_PEER)?;
        if !baseline.is_empty() {
            doc.get_text("text").insert(0, baseline)?;
        }
        // An empty implicit root has no native seed change. The outer fixture
        // still owns the epoch; this is not the proposed AM seed contract.
        doc.commit_with(loro::CommitOptions::new().timestamp(0).commit_msg(&format!(
            "puyudb/loro-fit-seed/i02 epoch={} baseline={}",
            hex(&epoch),
            hex(&Sha256::digest(baseline.as_bytes()))
        )));
        Self::capture(epoch, &doc, doc.oplog_frontiers())
    }

    /// Caller asserts trusted fixture history, not an imported candidate.
    /// Pending local edits are rejected before export can implicitly commit.
    pub(crate) fn capture(epoch: [u8; 32], doc: &LoroDoc, cut: Frontiers) -> Result<Self> {
        if doc.get_pending_txn_len() != 0 {
            return Err(FitError::Rejected(
                "trusted fixture must have no pending local operations",
            ));
        }
        Ok(Self {
            epoch,
            snapshot: doc.export(ExportMode::Snapshot)?,
            cut,
        })
    }

    pub(crate) fn document(&self) -> Result<LoroDoc> {
        let full = document(SEED_PEER)?;
        if full.import(&self.snapshot)?.pending.is_some() || full.is_shallow() {
            return Err(FitError::Rejected(
                "expected complete trusted fixture history",
            ));
        }
        let prefix = full.fork_at(&self.cut)?;
        prefix.set_record_timestamp(false);
        prefix.set_change_merge_interval(-1);
        Ok(prefix)
    }
}

/// Retained input bytes + unsigned expected digest, not a PuyuDB artifact ID,
/// native identity, signature, producer record, or durable store.
#[derive(Clone)]
pub(crate) struct CapturedUpdate {
    pub bytes: Vec<u8>,
    pub sha256: [u8; 32],
}

impl CapturedUpdate {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        let sha256 = Sha256::digest(&bytes).into();
        Self { bytes, sha256 }
    }
}

pub(crate) struct ByteEdit<'a> {
    pub range: Range<usize>,
    pub inserted: &'a str,
}

fn configure_writer(base: &FixtureBase, binding: &Binding, sequence: u64) -> Result<LoroDoc> {
    if base.epoch != binding.epoch || sequence == 0 {
        return Err(FitError::Rejected(
            "fixture epoch or outer sequence mismatch",
        ));
    }
    let doc = base.document()?;
    doc.set_peer_id(binding.peer)?;
    Ok(doc)
}

pub(crate) fn author(
    base: &FixtureBase,
    binding: &Binding,
    sequence: u64,
    edits: &[ByteEdit<'_>],
) -> Result<CapturedUpdate> {
    let doc = configure_writer(base, binding, sequence)?;
    let from = doc.oplog_vv();
    let text = doc.get_text("text");
    for edit in edits {
        let before = text.to_string();
        if edit.range.start > edit.range.end
            || !before.is_char_boundary(edit.range.start)
            || !before.is_char_boundary(edit.range.end)
        {
            return Err(FitError::Rejected("invalid fixture UTF-8 byte range"));
        }
        let start = before[..edit.range.start].chars().count();
        let deleted = before[edit.range.clone()].chars().count();
        if deleted != 0 {
            text.delete(start, deleted)?;
        }
        if !edit.inserted.is_empty() {
            text.insert(start, edit.inserted)?;
        }
    }
    doc.commit_with(
        loro::CommitOptions::new()
            .timestamp(0)
            .commit_msg(&message(binding, sequence)),
    );
    Ok(CapturedUpdate::new(doc.export(ExportMode::updates(&from))?))
}

/// Trusted-generated fixture experiment only. Native metadata decoding itself
/// allocates/decodes, so the encoded cap is NOT a decompression/resource proof.
/// Reconstruct with native positional APIs; do not implement a CRDT merger.
pub(crate) fn inspect(
    base: &FixtureBase,
    binding: &Binding,
    sequence: u64,
    update: &CapturedUpdate,
) -> Result<LoroDoc> {
    if update.bytes.len() > 8 * 1024 * 1024 {
        return Err(FitError::Rejected(
            "encoded fixture exceeds experimental limit",
        ));
    }
    if <[u8; 32]>::from(Sha256::digest(&update.bytes)) != update.sha256 {
        return Err(FitError::Rejected("fixture artifact digest mismatch"));
    }
    let rebuilt = configure_writer(base, binding, sequence)?;
    let from = rebuilt.oplog_vv();
    let frontier = rebuilt.oplog_frontiers();
    // Rich-text entity positions equal scalar positions only in this no-mark
    // fixture profile. Check the entire trusted prefix, not only current text.
    let prior =
        rebuilt.export_json_updates_without_peer_compression(&VersionVector::default(), &from)?;
    for change in prior.changes {
        for op in change.ops {
            if op.container != text_container()
                || !matches!(
                    op.content,
                    JsonOpContent::Text(JsonTextOp::Insert { .. } | JsonTextOp::Delete { .. })
                )
            {
                return Err(FitError::Rejected(
                    "trusted prefix is outside plain-text fixture profile",
                ));
            }
        }
    }
    let meta = LoroDoc::decode_import_blob_meta(&update.bytes, true)?;
    if !matches!(meta.mode, EncodedBlobMode::Updates) || meta.change_num != 1 {
        return Err(FitError::Rejected(
            "expected exactly one native update change",
        ));
    }
    let start = from.get(&binding.peer).copied().unwrap_or(0);
    let end = meta.partial_end_vv.get(&binding.peer).copied().unwrap_or(0);
    if meta
        .partial_start_vv
        .iter()
        .any(|(peer, _)| *peer != binding.peer)
        || meta.partial_end_vv.len() != 1
        || meta
            .partial_start_vv
            .get(&binding.peer)
            .copied()
            .unwrap_or(0)
            != start
        || end <= start
        || end.checked_sub(start).is_none_or(|n| n > 1024 * 1024)
        || meta.start_timestamp != 0
        || meta.end_timestamp != 0
        || meta.start_frontiers != frontier
    {
        return Err(FitError::Rejected(
            "native span, timestamps or intent dependencies mismatch",
        ));
    }
    let stage = base.document()?;
    if stage.import(&update.bytes)?.pending.is_some() {
        return Err(FitError::Rejected(
            "native fixture has pending dependencies",
        ));
    }
    let mut expected = from.clone();
    expected.insert(binding.peer, end);
    if stage.oplog_vv() != expected {
        return Err(FitError::Rejected(
            "native import did not apply exactly the expected span",
        ));
    }
    let decoded = stage.export_json_updates_without_peer_compression(&from, &expected)?;
    if decoded.schema_version != 1 || decoded.peers.is_some() || decoded.changes.len() != 1 {
        return Err(FitError::Rejected("unexpected decoded fixture history"));
    }
    let change = &decoded.changes[0];
    if change.id.peer != binding.peer
        || change.id.counter != start
        || change.timestamp != 0
        || change.msg.as_deref() != Some(&message(binding, sequence))
        || Frontiers::from_iter(change.deps.iter().copied()) != frontier
        || change.ops.is_empty()
        || change.ops.len() > 65536
    {
        return Err(FitError::Rejected(
            "native writer, metadata or exact-cut mismatch",
        ));
    }
    let text = rebuilt.get_text("text");
    let mut next = start;
    for op in &change.ops {
        if op.container != text_container() {
            return Err(FitError::Rejected(
                "operation targets a foreign fixture container",
            ));
        }
        if op.counter != next {
            return Err(FitError::Rejected(
                "noncontiguous fixture operation counters",
            ));
        }
        let length = i32::try_from(op.content.op_len())
            .map_err(|_| FitError::Rejected("unsupported fixture operation length"))?;
        if length == 0 {
            return Err(FitError::Rejected("empty fixture operation"));
        }
        next = next
            .checked_add(length)
            .ok_or(FitError::Rejected("fixture counter overflow"))?;
        match &op.content {
            JsonOpContent::Text(JsonTextOp::Insert {
                pos,
                text: inserted,
            }) => {
                let position = usize::try_from(*pos)
                    .map_err(|_| FitError::Rejected("fixture position overflow"))?;
                if position > text.len_unicode() || inserted.len() > 1024 * 1024 {
                    return Err(FitError::Rejected(
                        "invalid fixture insertion range or limit",
                    ));
                }
                text.insert(position, inserted)?;
            }
            JsonOpContent::Text(JsonTextOp::Delete {
                pos,
                len,
                start_id: _,
            }) => {
                let (position, stop) = if *len > 0 {
                    (i64::from(*pos), i64::from(*pos) + i64::from(*len))
                } else {
                    (i64::from(*pos) + 1 + i64::from(*len), i64::from(*pos) + 1)
                };
                if position < 0 || stop <= position || stop > text.len_unicode() as i64 {
                    return Err(FitError::Rejected("invalid fixture deletion range"));
                }
                // Do not copy the submitted target ID: native delete computes
                // observed targets. Exact re-export below must reproduce them.
                if *len < 0 {
                    // Preserve backwards native spans, not just visible text.
                    for index in (position as usize..stop as usize).rev() {
                        text.delete(index, 1)?;
                    }
                } else {
                    text.delete(position as usize, (stop - position) as usize)?;
                }
            }
            _ => {
                return Err(FitError::Rejected(
                    "unsupported operation in plain-text fixture",
                ));
            }
        }
    }
    if next != end {
        return Err(FitError::Rejected(
            "decoded span differs from native blob metadata",
        ));
    }
    rebuilt.commit_with(
        loro::CommitOptions::new()
            .timestamp(0)
            .commit_msg(&message(binding, sequence)),
    );
    if rebuilt.oplog_vv() != expected || rebuilt.export(ExportMode::updates(&from))? != update.bytes
    {
        return Err(FitError::Rejected(
            "fixture bytes differ from independently reconstructed native export",
        ));
    }
    if rebuilt.get_text("text").to_string() != stage.get_text("text").to_string() {
        return Err(FitError::Rejected(
            "native fixture reconstruction text mismatch",
        ));
    }
    Ok(rebuilt)
}
