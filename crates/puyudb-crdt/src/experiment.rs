// SPDX-License-Identifier: Apache-2.0

//! Experimental native-only seam for Phase 0 evidence.
//!
//! Native hashes/heads here are not PuyuDB batch identities or versions. These
//! helpers are not an authorization, storage, or production admission interface.

use automerge::{
    ActorId, Automerge, AutomergeError, Change, ChangeHash, ObjId, ObjType, ROOT, ReadDoc,
    ScalarValue, TextEncoding, Value,
    legacy::{ElementId, Key, ObjectId, OpId, OpType},
    transaction::{CommitOptions, Transactable},
};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, ops::Range};

/// A failed experimental native-profile check, not a stable domain error code.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A native document operation failed.
    #[error(transparent)]
    Native(#[from] AutomergeError),
    /// Native bytes could not be decoded.
    #[error(transparent)]
    Decode(#[from] automerge::LoadChangeError),
    /// The candidate lies outside the experimental text-only profile.
    #[error("{0}")]
    Invalid(&'static str),
}

/// Synthetic inputs to native actor/message binding; not authenticated identity.
#[derive(Clone, Debug)]
pub struct EditContext {
    /// Fixture public key, not an authenticated signer.
    pub public_key: [u8; 32],
    /// Fixture branch identity.
    pub branch: [u8; 16],
    /// Fresh fixture writer epoch for independently writable historical forks.
    pub writer_epoch: [u8; 16],
    /// Fixture file content epoch.
    pub content_epoch: [u8; 32],
    /// Outer fixture sequence, deliberately separate from native change sequence.
    pub outer_sequence: u64,
}

impl EditContext {
    /// Derive the ordinary actor in a namespace disjoint from seed actors.
    #[must_use]
    pub fn actor(&self) -> ActorId {
        let mut hash = Sha256::new();
        hash.update(b"puyudb/am-actor/v1\0");
        hash.update(self.public_key);
        hash.update(self.branch);
        hash.update(self.writer_epoch);
        hash.update(self.content_epoch);
        let mut bytes = vec![1];
        bytes.extend_from_slice(&hash.finalize());
        ActorId::from(bytes)
    }

    /// Candidate deterministic metadata; this message syntax is not frozen.
    #[must_use]
    pub fn options(&self) -> CommitOptions {
        let epoch: String = self
            .content_epoch
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        CommitOptions::default().with_time(0).with_message(format!(
            "puyudb/am-change/i02 seq={} epoch={} ordinal=0",
            self.outer_sequence, epoch
        ))
    }
}

/// An epoch-owned seed, never owned by an editing writer or batch.
pub struct Seed {
    /// Native document containing exactly the seed change.
    pub document: Automerge,
    /// The seed's only root text object.
    pub text: ObjId,
    /// Retain these original bytes rather than regenerate old seeds after upgrades.
    pub change: Change,
}

/// Derive the reserved native seed actor from a fixture epoch.
#[must_use]
pub fn seed_actor(epoch: &[u8; 32]) -> ActorId {
    let mut hash = Sha256::new();
    hash.update(b"puyudb/am-seed-actor/v1\0");
    hash.update(epoch);
    let mut bytes = vec![0];
    bytes.extend_from_slice(&hash.finalize());
    ActorId::from(bytes)
}

/// Construct a candidate seed with explicit scalar indexing and zero-time metadata.
///
/// The message and native bytes are experimental fixtures, not a frozen profile.
///
/// # Errors
///
/// Returns the native engine's error if root creation or baseline insertion fails.
pub fn seed(epoch: &[u8; 32], baseline: &str) -> Result<Seed, AutomergeError> {
    let mut document =
        Automerge::new_with_encoding(TextEncoding::UnicodeCodePoint).with_actor(seed_actor(epoch));
    let mut tx = document.transaction();
    let text = tx.put_object(ROOT, "text", ObjType::Text)?;
    tx.splice_text(&text, 0, 0, baseline)?;
    tx.commit_with(
        CommitOptions::default()
            .with_time(0)
            .with_message("puyudb/am-text-seed/i02"),
    );
    let change = document
        .get_last_local_change()
        .expect("creating the root text always commits a change");
    Ok(Seed {
        document,
        text,
        change,
    })
}

/// Locate the only root text object in a trusted candidate document.
///
/// # Errors
///
/// Rejects extra root fields, wrong object types, or an unexpected seed actor.
pub fn text_object(document: &Automerge, epoch: &[u8; 32]) -> Result<ObjId, Error> {
    if document.text_encoding() != TextEncoding::UnicodeCodePoint {
        return Err(Error::Invalid("expected explicit native scalar indexing"));
    }
    if document.keys(ROOT).collect::<Vec<_>>() != ["text"] {
        return Err(Error::Invalid("expected exactly one root text field"));
    }
    match document.get(ROOT, "text")? {
        Some((Value::Object(ObjType::Text), id @ ObjId::Id(1, _, _))) if matches!(&id, ObjId::Id(_, actor, _) if actor == &seed_actor(epoch)) => {
            Ok(id)
        }
        _ => Err(Error::Invalid("unexpected text object or content epoch")),
    }
}

/// Author one change against exactly the requested native historical cut.
///
/// Byte positions are checked against that cut, then converted to scalar offsets.
/// The supplied document is never modified. A caller must allocate a new writer
/// epoch when reusing an actor would fork its existing sequence.
///
/// # Errors
///
/// Rejects unknown heads, empty edits, invalid UTF-8 boundaries/ranges, oversized
/// insertions, or unsupported native metadata/operations.
pub fn edit_at(
    document: &Automerge,
    heads: &[ChangeHash],
    context: &EditContext,
    range: Range<usize>,
    inserted: &str,
) -> Result<Change, Error> {
    let base = document.fork_at(heads)?;
    let object = text_object(&base, &context.content_epoch)?;
    let text = base.text(&object)?;
    if range.start > range.end
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return Err(Error::Invalid(
            "range must use valid UTF-8 scalar boundaries",
        ));
    }
    if (range.is_empty() && inserted.is_empty()) || inserted.len() > 1024 * 1024 {
        return Err(Error::Invalid(
            "empty edit or insertion exceeds experimental limit",
        ));
    }
    let start = text[..range.start].chars().count();
    let deleted = text[range].chars().count();
    let mut fork = base.clone();
    fork.set_actor(context.actor());
    let mut tx = fork.transaction();
    tx.splice_text(
        &object,
        start,
        isize::try_from(deleted)
            .map_err(|_| Error::Invalid("delete length exceeds native positional range"))?,
        inserted,
    )?;
    tx.commit_with(context.options());
    let change = fork
        .get_last_local_change()
        .ok_or(Error::Invalid("edit did not commit a native change"))?;
    inspect(&base, context, change.raw_bytes())
}

/// Convert a UTF-16 offset without accepting a split surrogate pair.
///
/// # Errors
///
/// Rejects positions inside a surrogate pair or beyond the supplied base text.
pub fn utf16_offset_to_byte(text: &str, offset: usize) -> Result<usize, Error> {
    let mut units = 0;
    for (byte, scalar) in text.char_indices() {
        if units == offset {
            return Ok(byte);
        }
        units += scalar.len_utf16();
        if units > offset {
            return Err(Error::Invalid("UTF-16 offset splits a surrogate pair"));
        }
    }
    if units == offset {
        Ok(text.len())
    } else {
        Err(Error::Invalid("UTF-16 offset exceeds base text"))
    }
}

/// Inspect a single pinned-encoder, uncompressed native change against its base.
///
/// This is a Phase-0 feasibility experiment, **not** a hardened untrusted-input
/// decoder. The base must already be a trusted, validated text-only document.
/// Outer signatures, authorization, batch IDs, producer mapping, and durable
/// admission remain outside this seam.
///
/// # Errors
///
/// Rejects non-text operations and references, actor/sequence/context mismatches,
/// dependencies outside the base cut, non-roundtripping native encodings,
/// compressed chunks, or inputs above experimental bounds.
pub fn inspect(base: &Automerge, context: &EditContext, raw: &[u8]) -> Result<Change, Error> {
    // Reject compressed chunks before upstream's potentially unbounded inflation.
    if raw.len() > 8 * 1024 * 1024 || raw.get(8) != Some(&1) {
        return Err(Error::Invalid(
            "expected a bounded uncompressed native change",
        ));
    }
    let change = Change::from_bytes(raw.to_vec())?;
    if change.is_empty() || change.len() > 1024 * 1024 {
        return Err(Error::Invalid("unsupported native operation count"));
    }
    let end = change.start_op().get().checked_add(change.len() as u64 - 1);
    if end.is_none_or(|end| end > u64::from(u32::MAX)) {
        return Err(Error::Invalid(
            "native operation counter exceeds checked profile",
        ));
    }
    let decoded = change.decode();
    if Change::from(decoded.clone()).raw_bytes() != raw {
        return Err(Error::Invalid(
            "native bytes do not roundtrip through pinned encoder",
        ));
    }
    if context.outer_sequence == 0
        || change.actor_id() != &context.actor()
        || change.timestamp() != 0
        || change.message() != context.options().message.as_deref()
        || !change.extra_bytes().is_empty()
        || change.author().is_some()
    {
        return Err(Error::Invalid(
            "native writer or deterministic metadata mismatch",
        ));
    }
    let object = text_object(base, &context.content_epoch)?;
    let ObjId::Id(counter, actor, _) = object else {
        return Err(Error::Invalid("root is not a text object"));
    };
    let object = ObjectId::Id(OpId::new(counter, &actor));
    if change
        .deps()
        .iter()
        .any(|hash| base.get_change_by_hash(hash).is_none())
        || !base.get_changes(change.deps()).is_empty()
    {
        return Err(Error::Invalid(
            "native dependencies do not represent the exact base cut",
        ));
    }
    let history = base.get_changes(&[]);
    let next_sequence = history
        .iter()
        .filter(|c| c.actor_id() == change.actor_id())
        .map(Change::seq)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    let next_counter = history
        .iter()
        .map(|c| c.start_op().get() + c.len() as u64 - 1)
        .max()
        .unwrap_or(0)
        .checked_add(1);
    if next_sequence != Some(change.seq()) || next_counter != Some(change.start_op().get()) {
        return Err(Error::Invalid(
            "native change sequence or base Lamport counter mismatch",
        ));
    }

    let mut known = HashSet::new();
    let mut visible = HashSet::new();
    for prior in &history {
        for (index, op) in prior.decode().operations.iter().enumerate() {
            if op.obj != object {
                continue;
            }
            let id = OpId::new(prior.start_op().get() + index as u64, prior.actor_id());
            if op.insert {
                known.insert(id.clone());
                visible.insert(id);
            } else if op.action == OpType::Delete
                && let Key::Seq(ElementId::Id(id)) = &op.key
            {
                visible.remove(id);
            }
        }
    }
    for (index, op) in decoded.operations.iter().enumerate() {
        if op.obj != object {
            return Err(Error::Invalid("operation targets a root or foreign object"));
        }
        match (&op.action, &op.key) {
            (OpType::Put(ScalarValue::Str(value)), Key::Seq(anchor))
                if op.insert && op.pred.is_empty() && value.chars().count() == 1 =>
            {
                if let ElementId::Id(id) = anchor
                    && !known.contains(id)
                {
                    return Err(Error::Invalid(
                        "insertion anchor is outside the base or earlier operations",
                    ));
                }
                let id = OpId::new(change.start_op().get() + index as u64, change.actor_id());
                known.insert(id.clone());
                visible.insert(id);
            }
            (OpType::Delete, Key::Seq(ElementId::Id(id))) if !op.insert => {
                if op.pred.len() != 1 || op.pred.get(0) != Some(id) || !visible.remove(id) {
                    return Err(Error::Invalid(
                        "deletion must target a visible text element",
                    ));
                }
            }
            _ => {
                return Err(Error::Invalid(
                    "unsupported operation in text-only native profile",
                ));
            }
        }
    }
    let mut staged = base.clone();
    staged.apply_changes([change.clone()])?;
    if staged.get_heads() != [change.hash()] {
        return Err(Error::Invalid(
            "native engine did not apply the requested change",
        ));
    }
    Ok(change)
}
