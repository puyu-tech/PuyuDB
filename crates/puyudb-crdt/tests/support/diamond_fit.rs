// SPDX-License-Identifier: Apache-2.0

//! Synthetic DT-fit profile, not identity authentication or a frozen PuyuDB format.
//! Whole-byte reconstruction rejects ignored input, but the opposite-import-order
//! regression also proves false rejection of valid updates at the same closed cut.
//! It is NOT a general admission interface. No native canonicalization is assumed.

use std::{error::Error, ops::Range};

use diamond_types::{
    AgentId, DTRange, Frontier, HasLength,
    causalgraph::agent_assignment::remote_ids::RemoteFrontierOwned,
    list::{
        ListBranch, ListOpLog,
        encoding::EncodeOptions,
        operation::{ListOpKind, TextOperation},
    },
};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub(crate) struct FixtureWriter {
    pub key: [u8; 32],
    pub branch: [u8; 16],
    pub writer_epoch: [u8; 16],
}

pub(crate) fn writer_name(epoch: &[u8; 32], writer: &FixtureWriter) -> String {
    let digest = Sha256::new()
        .chain_update(b"puyudb/diamond-fit/writer/i02\0")
        .chain_update(writer.key)
        .chain_update(writer.branch)
        .chain_update(writer.writer_epoch)
        .chain_update(epoch)
        .finalize();
    format!("w:{}", base64url(&digest))
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for shift in (0..=18).rev().step_by(6).take(chunk.len() + 1) {
            result.push(char::from(ALPHABET[((bits >> shift) & 63) as usize]));
        }
    }
    result
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FitError {
    #[error("{0}")]
    Rejected(&'static str),
    #[error("native fixture decoding failed: {0}")]
    Native(#[source] Box<dyn Error + Send + Sync>),
}

pub(crate) type Result<T> = std::result::Result<T, FitError>;

fn load(bytes: &[u8]) -> Result<ListOpLog> {
    ListOpLog::load_from(bytes).map_err(|e| FitError::Native(Box::new(e)))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn seed_name(epoch: &[u8; 32]) -> String {
    let digest = Sha256::new()
        .chain_update(b"puyudb/diamond-fit/seed/i02\0")
        .chain_update(epoch)
        .finalize();
    format!("s:{}", base64url(&digest))
}

pub(crate) fn message(epoch: &[u8; 32], writer: &FixtureWriter, sequence: u64) -> String {
    format!(
        "puyudb/diamond-fit/i02 key={} branch={} writer_epoch={} epoch={} seq={sequence} ordinal=0",
        hex(&writer.key),
        hex(&writer.branch),
        hex(&writer.writer_epoch),
        hex(epoch),
    )
}

pub(crate) fn frontier(log: &ListOpLog) -> RemoteFrontierOwned {
    ids_at(log, log.local_frontier_ref())
}

fn ids_at(log: &ListOpLog, cut: &[usize]) -> RemoteFrontierOwned {
    let mut ids = log.cg.agent_assignment.local_to_remote_frontier_owned(cut);
    ids.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    ids
}

fn resolve(log: &ListOpLog, cut: &RemoteFrontierOwned) -> Result<Frontier> {
    if cut
        .windows(2)
        .any(|p| (&p[0].0, p[0].1) >= (&p[1].0, p[1].1))
    {
        return Err(FitError::Rejected("noncanonical fixture frontier"));
    }
    let local = log
        .cg
        .agent_assignment
        .try_remote_to_local_frontier(cut.iter())
        .map_err(|_| FitError::Rejected("unavailable fixture frontier"))?;
    for (index, &id) in local.iter().enumerate() {
        let others: Vec<_> = local
            .iter()
            .copied()
            .enumerate()
            .filter_map(|(i, v)| (i != index).then_some(v))
            .collect();
        if log.cg.graph.frontier_contains_frontier(&others, &[id]) {
            return Err(FitError::Rejected("redundant fixture frontier"));
        }
    }
    Ok(local)
}

/// Immutable trusted history and declared intent cut, not an admitted artifact.
#[derive(Clone)]
pub(crate) struct FixtureBase {
    epoch: [u8; 32],
    snapshot: Vec<u8>,
    cut: RemoteFrontierOwned,
}

impl FixtureBase {
    pub(crate) fn seed(epoch: [u8; 32], baseline: &str) -> Result<Self> {
        let mut log = ListOpLog::new();
        if !baseline.is_empty() {
            let seed = log.get_or_create_agent_id(&seed_name(&epoch));
            log.add_insert(seed, 0, baseline);
        }
        Self::capture(epoch, &log, frontier(&log))
    }

    pub(crate) fn capture(
        epoch: [u8; 32],
        log: &ListOpLog,
        cut: RemoteFrontierOwned,
    ) -> Result<Self> {
        resolve(log, &cut)?;
        Ok(Self {
            epoch,
            snapshot: log.encode(&EncodeOptions::full().compress_content(false)),
            cut,
        })
    }

    pub(crate) fn document(&self) -> Result<ListOpLog> {
        let full = load(&self.snapshot)?;
        let cut = resolve(&full, &self.cut)?;
        if full.len() > 1024 * 1024 {
            return Err(FitError::Rejected(
                "trusted fixture history exceeds experimental limit",
            ));
        }
        // Copy only native operations in the trusted causal closure. Native APIs
        // retain their original positions/parents and perform all CRDT merging.
        let mut prefix = ListOpLog::new();
        for (op, entry, remote) in full.iter_full() {
            let included = |id| {
                full.cg
                    .graph
                    .frontier_contains_frontier(cut.as_ref(), &[id])
            };
            if !included(entry.span.start) {
                continue;
            }
            let count = if included(entry.span.end - 1) {
                entry.span.len()
            } else {
                (entry.span.start..entry.span.end)
                    .take_while(|&id| included(id))
                    .count()
            };
            let mut part = op.clone();
            if count != entry.span.len() {
                match op.kind {
                    ListOpKind::Ins => {
                        if !op.loc.fwd {
                            return Err(FitError::Rejected(
                                "reversed insertion outside fixture profile",
                            ));
                        }
                        let text: String = op
                            .content_as_str()
                            .ok_or(FitError::Rejected("missing trusted insert content"))?
                            .chars()
                            .take(count)
                            .collect();
                        part = TextOperation::new_insert(op.start(), &text);
                    }
                    ListOpKind::Del => {
                        part.loc.span = if op.loc.fwd {
                            (op.start()..op.start() + count).into()
                        } else {
                            (op.end() - count..op.end()).into()
                        };
                    }
                }
            }
            let parents = full
                .cg
                .agent_assignment
                .local_to_remote_frontier_owned(entry.parents.as_ref());
            let parents = prefix
                .cg
                .agent_assignment
                .try_remote_to_local_frontier(parents.iter())
                .map_err(|_| FitError::Rejected("trusted closure predecessor missing"))?;
            let agent = prefix.get_or_create_agent_id(remote.0);
            prefix.add_operations_remote(agent, parents.as_ref(), remote.1.start, &[part]);
        }
        if frontier(&prefix) != self.cut {
            return Err(FitError::Rejected(
                "trusted native cut reconstruction differs",
            ));
        }
        Ok(prefix)
    }
}

fn next_sequence(log: &ListOpLog, name: &str) -> usize {
    log.iter_remote_mappings()
        .filter(|span| span.0 == name)
        .map(|span| span.1.end)
        .max()
        .unwrap_or(0)
}

fn configure(
    base: &FixtureBase,
    epoch: &[u8; 32],
    writer: &FixtureWriter,
    sequence: u64,
) -> Result<ListOpLog> {
    if &base.epoch != epoch || sequence == 0 {
        return Err(FitError::Rejected(
            "fixture epoch or outer sequence mismatch",
        ));
    }
    let prefix = base.document()?;
    let full = load(&base.snapshot)?;
    let name = writer_name(epoch, writer);
    if next_sequence(&full, &name) != next_sequence(&prefix, &name) {
        return Err(FitError::Rejected(
            "historical fixture edit requires a fresh writer epoch",
        ));
    }
    Ok(prefix)
}

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

pub(crate) fn author(
    base: &FixtureBase,
    epoch: &[u8; 32],
    writer: &FixtureWriter,
    sequence: u64,
    edits: &[ByteEdit<'_>],
) -> Result<CapturedUpdate> {
    let mut log = configure(base, epoch, writer, sequence)?;
    let from = log.local_frontier();
    let before = log.len();
    let agent = log.get_or_create_agent_id(&writer_name(epoch, writer));
    let mut view = log.checkout_tip();
    for edit in edits {
        let text = view.content().to_string();
        if edit.range.start > edit.range.end
            || !text.is_char_boundary(edit.range.start)
            || !text.is_char_boundary(edit.range.end)
        {
            return Err(FitError::Rejected("invalid fixture UTF-8 byte range"));
        }
        let start = text[..edit.range.start].chars().count();
        let deleted = text[edit.range.clone()].chars().count();
        if deleted != 0 {
            view.delete(&mut log, agent, start..start + deleted);
        }
        if !edit.inserted.is_empty() {
            view.insert(&mut log, agent, start, edit.inserted);
        }
    }
    if log.len() == before {
        return Err(FitError::Rejected("empty authored fixture"));
    }
    let context = message(epoch, writer, sequence);
    Ok(CapturedUpdate::new(
        log.encode_from(
            &EncodeOptions::patch()
                .compress_content(false)
                .user_data(context.as_bytes()),
            from.as_ref(),
        ),
    ))
}

/// Trusted-generated fixtures only. Native decoding can allocate or panic before
/// counts are available; the encoded cap is NOT a hostile-input resource proof.
pub(crate) fn inspect(
    base: &FixtureBase,
    epoch: &[u8; 32],
    writer: &FixtureWriter,
    sequence: u64,
    update: &CapturedUpdate,
) -> Result<ListOpLog> {
    if update.bytes.len() > 8 * 1024 * 1024 {
        return Err(FitError::Rejected(
            "encoded fixture exceeds experimental limit",
        ));
    }
    if <[u8; 32]>::from(Sha256::digest(&update.bytes)) != update.sha256 {
        return Err(FitError::Rejected("fixture artifact digest mismatch"));
    }
    let mut rebuilt = configure(base, epoch, writer, sequence)?;
    let from = rebuilt.local_frontier();
    let before = rebuilt.len();
    let name = writer_name(epoch, writer);
    let mut next = next_sequence(&rebuilt, &name);
    let mut stage = rebuilt.clone();
    stage
        .decode_and_add(&update.bytes)
        .map_err(|e| FitError::Native(Box::new(e)))?;
    if stage.len() <= before || stage.len() - before > 1024 * 1024 {
        return Err(FitError::Rejected("expected a bounded new fixture span"));
    }
    let agent = rebuilt.get_or_create_agent_id(&name);
    let mut view = rebuilt.checkout_tip();
    for (op, history, remote) in stage.iter_full_range(DTRange::from(before..stage.len())) {
        if remote.0 != name
            || remote.1.start != next
            || remote.1.len() != op.len()
            || history.span.len() != op.len()
        {
            return Err(FitError::Rejected(
                "native fixture writer or counter span mismatch",
            ));
        }
        if ids_at(&stage, history.parents.as_ref()) != ids_at(&rebuilt, view.local_frontier_ref()) {
            return Err(FitError::Rejected(
                "native fixture intent dependencies mismatch",
            ));
        }
        if op.is_empty() {
            return Err(FitError::Rejected("empty native fixture operation"));
        }
        next = next
            .checked_add(op.len())
            .ok_or(FitError::Rejected("fixture counter overflow"))?;
        replay(&mut rebuilt, &mut view, agent, &op)?;
    }
    let context = message(epoch, writer, sequence);
    let expected = rebuilt.encode_from(
        &EncodeOptions::patch()
            .compress_content(false)
            .user_data(context.as_bytes()),
        from.as_ref(),
    );
    if expected != update.bytes {
        return Err(FitError::Rejected(
            "fixture bytes differ from independent native reconstruction",
        ));
    }
    if view.content() != stage.checkout_tip().content() {
        return Err(FitError::Rejected(
            "native fixture reconstruction text mismatch",
        ));
    }
    Ok(rebuilt)
}

fn replay(
    log: &mut ListOpLog,
    view: &mut ListBranch,
    agent: AgentId,
    op: &TextOperation,
) -> Result<()> {
    match op.kind {
        ListOpKind::Ins => {
            let content = op
                .content_as_str()
                .ok_or(FitError::Rejected("missing native fixture content"))?;
            if !op.loc.fwd
                || op.start() > view.len()
                || content.chars().count() != op.len()
                || content.len() > 1024 * 1024
            {
                return Err(FitError::Rejected("unsupported native fixture insertion"));
            }
            // Native positional APIs regenerate observed origins/targets at
            // this exact cut; no private CRDT merger or submitted target copy.
            view.insert(log, agent, op.start(), content);
        }
        ListOpKind::Del => {
            if op.end() > view.len() || op.start() >= op.end() || op.content.is_some() {
                return Err(FitError::Rejected("unsupported native fixture deletion"));
            }
            if op.loc.fwd {
                view.delete(log, agent, op.start()..op.end());
            } else {
                for position in (op.start()..op.end()).rev() {
                    view.delete(log, agent, position..position + 1);
                }
            }
        }
    }
    Ok(())
}
