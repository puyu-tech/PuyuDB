// SPDX-License-Identifier: Apache-2.0

//! Production `dt-text-v1` binding over the pinned diamond-types fork
//! (spec §12.1: `DMNDTYPS` protocol version 0, no LZ4, no other feature
//! profile). This is the only crate that touches native types; `puyudb-core`
//! consumes it through the wrappers below and never sees a native object.
//!
//! Portable identity is (agent name, sequence) — never the replica-local
//! numeric `AgentId` (§12.1). Seed and writer actor names come from
//! `puyudb-core`'s frozen derivations (§12.2–12.3) and are passed in as
//! strings; this crate never derives them.

use diamond_types::DTRange;
use diamond_types::causalgraph::agent_assignment::remote_ids::RemoteVersion;
use diamond_types::list::ListOpLog;
use diamond_types::list::encoding::{EncodeOptions, ParseError};
use diamond_types::list::operation::ListOpKind;
use thiserror::Error;

fn dt_range(range: (usize, usize)) -> DTRange {
    (range.0..range.1).into()
}

/// Failure of the `dt-text-v1` binding.
#[derive(Debug, Error)]
pub enum Error {
    /// Native bytes are malformed or outside the frozen dialect.
    #[error("invalid_encoding: {0}")]
    Encoding(String),
    /// The update is not entirely new (known prefix/suffix/replay).
    #[error("known_operation")]
    KnownOperation,
    /// The update's base version is not contained in the receiving document —
    /// i.e. a native parent outside the declared cut.
    #[error("invalid_native_scope: base version unknown to the receiving document")]
    BaseVersionUnknown,
    /// The update violates the caller's native scope policy.
    #[error("invalid_native_scope: {0}")]
    Scope(String),
}

impl Error {
    /// Stable public code, matching the profile's §14 list.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Encoding(_) => "invalid_encoding",
            Self::KnownOperation | Self::BaseVersionUnknown | Self::Scope(_) => {
                "invalid_native_scope"
            }
        }
    }
}

fn parse_error(error: ParseError) -> Error {
    match error {
        ParseError::KnownOperation => Error::KnownOperation,
        ParseError::BaseVersionUnknown => Error::BaseVersionUnknown,
        other => Error::Encoding(format!("{other:?}")),
    }
}

/// One submitted native operation span: portable agent name and half-open
/// sequence range. Op kinds are insert/delete only by construction
/// (`TextOperation` has no other variants, §12.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedSpan {
    /// Portable agent name.
    pub agent: String,
    /// First operation index of this agent in the span (inclusive).
    pub seq_start: usize,
    /// End of the span (exclusive); `seq_end - seq_start` is the op count.
    pub seq_end: usize,
}

/// Insert/delete, the only operation kinds §12.5 allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// Insertion.
    Insert,
    /// Deletion.
    Delete,
}

/// One submitted history entry: local span and its parent LVs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmittedEntry {
    /// Local LV span of the entry.
    pub span: (usize, usize),
    /// Parent LVs in the staged log's (base-preserving) LV space.
    pub parents: Vec<usize>,
}

/// Metadata of one strictly decoded new-update artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrictReport {
    /// Declared agent names in wire order, including unused declarations.
    pub agent_names: Vec<String>,
    /// Local range of ALL submitted operations (none trimmed as known).
    pub submitted: (usize, usize),
    /// Original start frontier as LVs in the staged log's LV space (the staged
    /// log is a base clone, so base LVs keep their indices).
    pub start_frontier_lvs: Vec<usize>,
}

/// A native text document: the only native handle crossing into core.
pub struct TextDoc {
    log: ListOpLog,
}

/// Frozen encode options: patch dialect, no content compression.
fn encode_options() -> EncodeOptions<'static> {
    EncodeOptions::patch().compress_content(false)
}

impl TextDoc {
    /// An empty document (root only).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            log: ListOpLog::new(),
        }
    }

    /// Load native bytes (full document or patch).
    ///
    /// # Errors
    /// Returns [`Error::Encoding`] for malformed or out-of-dialect bytes.
    pub fn load(bytes: &[u8]) -> Result<Self, Error> {
        ListOpLog::load_from(bytes)
            .map(|log| Self { log })
            .map_err(parse_error)
    }

    /// Encode the full document in the frozen dialect.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.log.encode(&encode_options())
    }

    /// Encode a patch containing only the operations after `from_version`.
    #[must_use]
    pub fn encode_from(&self, from_version: &[usize]) -> Vec<u8> {
        self.log.encode_from(&encode_options(), from_version)
    }

    /// Current text at the tip.
    #[must_use]
    pub fn text(&self) -> String {
        self.log.checkout_tip().content().to_string()
    }

    /// Text at a local version (historical read), as UTF-8.
    #[must_use]
    pub fn text_at(&self, version: &[usize]) -> String {
        self.log.checkout(version).content().to_string()
    }

    /// Portable remote frontier: (agent name, next sequence) pairs.
    #[must_use]
    pub fn remote_frontier(&self) -> Vec<(String, usize)> {
        self.log
            .remote_frontier()
            .iter()
            .map(|RemoteVersion(name, seq)| ((*name).to_owned(), *seq))
            .collect()
    }

    /// Local frontier LVs.
    #[must_use]
    pub fn local_frontier(&self) -> Vec<usize> {
        self.log.local_frontier_ref().to_vec()
    }

    /// Number of operations in the graph (LV space bound).
    #[must_use]
    pub fn op_len(&self) -> usize {
        self.log.len()
    }

    /// Intern or fetch the local agent id for a portable name.
    pub fn agent_id(&mut self, name: &str) -> u32 {
        self.log.get_or_create_agent_id(name)
    }

    /// Append a local insert at a char position with explicit parent LVs.
    pub fn add_insert(&mut self, agent: u32, parents: &[usize], pos: usize, text: &str) -> usize {
        self.log.add_insert_at(agent, parents, pos, text)
    }

    /// Append a local delete of a char range with explicit parent LVs.
    pub fn add_delete(
        &mut self,
        agent: u32,
        parents: &[usize],
        loc: std::ops::Range<usize>,
    ) -> usize {
        self.log.add_delete_at(agent, parents, loc)
    }

    /// Strictly decode a new-update artifact against this base.
    ///
    /// The base is not mutated; the returned staged doc is a clone of the base
    /// with the submitted ops appended, so existing LV indices are preserved
    /// and parent LVs of submitted ops can be checked against the base.
    ///
    /// # Errors
    /// Returns [`Error::KnownOperation`] when any submitted op is already
    /// known, and [`Error::Encoding`] for malformed input.
    pub fn strict_append(&self, bytes: &[u8]) -> Result<(TextDoc, StrictReport), Error> {
        let (log, info) = self.log.decode_new_strict(bytes).map_err(parse_error)?;
        Ok((
            Self { log },
            StrictReport {
                agent_names: info.agent_names,
                submitted: (info.submitted.start, info.submitted.end),
                start_frontier_lvs: info.start_frontier.iter().copied().collect(),
            },
        ))
    }

    /// Submitted op spans as (agent name, seq range), in wire order.
    ///
    /// The seq ranges are portable (agent name + operation index), so actor
    /// sequence continuity can be checked without any local id.
    #[must_use]
    pub fn submitted_spans(&self, submitted: (usize, usize)) -> Vec<SubmittedSpan> {
        self.log
            .iter_remote_mappings_range(dt_range(submitted))
            .map(|span| SubmittedSpan {
                agent: span.0.to_owned(),
                seq_start: span.1.start,
                seq_end: span.1.end,
            })
            .collect()
    }

    /// Submitted history entries (span + parent LVs) for scope checks.
    #[must_use]
    pub fn submitted_entries(&self, submitted: (usize, usize)) -> Vec<SubmittedEntry> {
        self.log
            .iter_history_range(dt_range(submitted))
            .map(|entry| SubmittedEntry {
                span: (entry.span.start, entry.span.end),
                parents: entry.parents.iter().copied().collect(),
            })
            .collect()
    }

    /// Op kinds of a local range, in order.
    #[must_use]
    pub fn op_kinds(&self, range: (usize, usize)) -> Vec<OpKind> {
        self.log
            .iter_ops_range(dt_range(range))
            .map(|op| match op.kind {
                ListOpKind::Ins => OpKind::Insert,
                ListOpKind::Del => OpKind::Delete,
            })
            .collect()
    }

    /// Whether `lv` is contained in this log's local frontier closure.
    #[must_use]
    pub fn version_contains(&self, lv: usize) -> bool {
        self.log
            .version_contains_lv(self.log.local_frontier_ref(), lv)
    }
}

/// UTF-8 byte offset → char index; `None` when not on a scalar boundary
/// (§5.5: positions must be Unicode scalar boundaries).
#[must_use]
pub fn byte_to_char(text: &str, byte_offset: usize) -> Option<usize> {
    if byte_offset > text.len() || !text.is_char_boundary(byte_offset) {
        return None;
    }
    Some(text[..byte_offset].chars().count())
}

/// Char index → UTF-8 byte offset; `None` when out of range.
#[must_use]
pub fn char_to_byte(text: &str, char_index: usize) -> Option<usize> {
    if char_index > text.chars().count() {
        return None;
    }
    Some(text.chars().take(char_index).map(char::len_utf8).sum())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experiment::{self, EditContext};
    use automerge::ReadDoc;

    /// T01/T02-style parity: the same seed + edit sequence produces the same
    /// text on the DT production path and on the AM slow-reference path.
    #[test]
    fn dt_matches_am_reference_on_shared_edits() {
        let epoch = [7u8; 32];
        // DT path: seed actor inserts the baseline, writer inserts at char 5.
        let mut dt = TextDoc::empty();
        let seed_agent = dt.agent_id(&format!("s:{}", "seed"));
        dt.add_insert(seed_agent, &[], 0, "hello");
        let dt_seed_frontier = dt.local_frontier();
        let writer = dt.agent_id("w:writer");
        dt.add_insert(writer, &dt_seed_frontier, 5, " world");
        let dt_text = dt.text();

        // AM slow reference: experiment seed + edit_at at byte range 5..5.
        let am_seed = experiment::seed(&epoch, "hello").expect("am seed");
        let context = EditContext {
            public_key: [3; 32],
            branch: [2; 16],
            writer_epoch: [4; 16],
            content_epoch: epoch,
            outer_sequence: 2,
        };
        let change = experiment::edit_at(
            &am_seed.document,
            &am_seed.document.get_heads(),
            &context,
            5..5,
            " world",
        )
        .expect("am edit");
        let mut am_doc = am_seed.document.clone();
        am_doc.apply_changes([change]).expect("apply");
        let am_obj = experiment::text_object(&am_doc, &epoch).expect("text object");
        let am_text = am_doc.text(&am_obj).expect("am text");

        assert_eq!(dt_text, "hello world");
        assert_eq!(dt_text, am_text);
    }

    /// T01 convergence: two concurrent inserts at the same position converge
    /// to the same text in both application orders.
    #[test]
    fn dt_converges_across_application_orders() {
        let seed_doc = || {
            let mut doc = TextDoc::empty();
            let seed = doc.agent_id("s:seed");
            doc.add_insert(seed, &[], 0, "ac");
            doc
        };
        let patch = |agent: &str, text: &str| {
            let mut doc = seed_doc();
            let frontier = doc.local_frontier();
            let a = doc.agent_id(agent);
            doc.add_insert(a, &frontier, 1, text);
            doc.encode_from(&frontier)
        };
        let patch_a = patch("w:a", "X");
        let patch_b = patch("w:b", "Y");
        let base1 = seed_doc();
        let (s1, _) = base1.strict_append(&patch_a).expect("append a");
        let (s1, _) = s1.strict_append(&patch_b).expect("append b");
        let base2 = TextDoc::load(&seed_doc().encode()).expect("reload");
        let (s2, _) = base2.strict_append(&patch_b).expect("append b");
        let (s2, _) = s2.strict_append(&patch_a).expect("append a");
        assert_eq!(s1.text(), s2.text());
        assert_eq!(s1.text().len(), 4);
    }
}
