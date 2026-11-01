// SPDX-License-Identifier: Apache-2.0

//! Trusted-fixture benchmark adapters, not PuyuDB admission or persistent profiles.
//! Native identities/cuts remain candidate-specific. Human commit messages are
//! omitted in this common cost profile; the older Automerge profile is unchanged.

use std::{
    error::Error,
    ops::Range,
    time::{Duration, Instant},
};

use automerge::{
    Automerge, ChangeHash, ObjId, ReadDoc,
    transaction::{CommitOptions, Transactable},
};
use diamond_types::{
    AgentId,
    causalgraph::agent_assignment::remote_ids::RemoteFrontierOwned,
    list::{ListBranch, ListOpLog, encoding::EncodeOptions},
};
use loro::{ExportMode, Frontiers, LoroDoc};
use puyudb_crdt::experiment::seed;
use yrs::{
    ClientID, Doc, GetString, OffsetKind, Options, ReadTxn, Snapshot, StateVector, Text, TextRef,
    Transact, Update,
    updates::{
        decoder::Decode,
        encoder::{Encoder, EncoderV1},
    },
};

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Resolve the same byte-range intent using an independent text oracle.
/// Translation is deliberately outside the native edit interval.
pub(crate) struct Edit<'a> {
    scalar_start: usize,
    scalar_delete: usize,
    utf16_start: u32,
    utf16_delete: u32,
    inserted: &'a str,
}

impl<'a> Edit<'a> {
    pub(crate) fn at(text: &str, range: Range<usize>, inserted: &'a str) -> Result<Self> {
        if range.start > range.end
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
            || (range.is_empty() && inserted.is_empty())
        {
            return Err("expected a nonempty edit at valid UTF-8 byte boundaries".into());
        }
        let prefix = &text[..range.start];
        let deleted = &text[range];
        Ok(Self {
            scalar_start: prefix.chars().count(),
            scalar_delete: deleted.chars().count(),
            utf16_start: u32::try_from(prefix.encode_utf16().count())?,
            utf16_delete: u32::try_from(deleted.encode_utf16().count())?,
            inserted,
        })
    }
}

/// Only used with trusted local fixtures and cuts from the same lineage.
/// `splice` returns native mutation + required commit duration, excluding
/// coordinate resolution. Native APIs can defer physical work (notably DT's
/// rope buffer). UTF-8 materialization after seed/load/edit is timed separately.
pub(crate) trait NativeText: Sized {
    const NAME: &'static str;
    type Cut;
    fn seed(baseline: &str) -> Result<Self>;
    fn splice(&mut self, edit: &Edit<'_>) -> Result<Duration>;
    fn text(&self) -> Result<String>;
    fn cut(&self) -> Self::Cut;
    fn save(&mut self) -> Result<Vec<u8>>;
    fn load(bytes: &[u8]) -> Result<Self>;
    fn text_at(&mut self, cut: &Self::Cut) -> Result<String>;
}

pub(crate) struct AutomergeText {
    document: Automerge,
    text: ObjId,
}

impl NativeText for AutomergeText {
    const NAME: &'static str = "automerge";
    type Cut = Vec<ChangeHash>;

    fn seed(baseline: &str) -> Result<Self> {
        let seeded = seed(&[7; 32], baseline)?;
        let mut document = seeded.document;
        document.set_actor(automerge::ActorId::from(&b"comparison-writer"[..]));
        Ok(Self {
            document,
            text: seeded.text,
        })
    }

    fn splice(&mut self, edit: &Edit<'_>) -> Result<Duration> {
        let deleted = isize::try_from(edit.scalar_delete)?;
        let options = CommitOptions::default().with_time(0);
        let started = Instant::now();
        let mut tx = self.document.transaction();
        tx.splice_text(&self.text, edit.scalar_start, deleted, edit.inserted)?;
        tx.commit_with(options);
        Ok(started.elapsed())
    }

    fn text(&self) -> Result<String> {
        Ok(self.document.text(&self.text)?)
    }
    fn cut(&self) -> Self::Cut {
        self.document.get_heads()
    }
    fn save(&mut self) -> Result<Vec<u8>> {
        Ok(self.document.save())
    }

    fn load(bytes: &[u8]) -> Result<Self> {
        let document = Automerge::load_with_options(
            bytes,
            automerge::LoadOptions::new().text_encoding(automerge::TextEncoding::UnicodeCodePoint),
        )?;
        let text = puyudb_crdt::experiment::text_object(&document, &[7; 32])?;
        Ok(Self { document, text })
    }

    fn text_at(&mut self, cut: &Self::Cut) -> Result<String> {
        Ok(self.document.text_at(&self.text, cut)?)
    }
}

fn yrs_document(client: u64) -> Doc {
    Doc::with_options(Options {
        client_id: ClientID::new(client),
        offset_kind: OffsetKind::Utf16,
        skip_gc: true,
        ..Options::default()
    })
}

pub(crate) struct YrsText {
    document: Doc,
    text: TextRef,
}

impl NativeText for YrsText {
    const NAME: &'static str = "yrs";
    type Cut = Snapshot;

    fn seed(baseline: &str) -> Result<Self> {
        let source = yrs_document(1);
        {
            let text = source.get_or_insert_text("text");
            let mut tx = source.transact_mut();
            text.insert(&mut tx, 0, baseline);
            tx.commit();
        }
        let bytes = source
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        // A fresh writer cannot share the seed's native client identity.
        let document = yrs_document(2);
        {
            let mut tx = document.transact_mut();
            tx.apply_update(Update::decode_v1(&bytes)?)?;
            tx.commit();
        }
        let text = document.get_or_insert_text("text");
        Ok(Self { document, text })
    }

    fn splice(&mut self, edit: &Edit<'_>) -> Result<Duration> {
        let started = Instant::now();
        {
            let mut tx = self.document.transact_mut();
            if edit.utf16_delete != 0 {
                self.text
                    .remove_range(&mut tx, edit.utf16_start, edit.utf16_delete);
            }
            if !edit.inserted.is_empty() {
                self.text.insert(&mut tx, edit.utf16_start, edit.inserted);
            }
            tx.commit();
        }
        Ok(started.elapsed())
    }

    fn text(&self) -> Result<String> {
        Ok(self.text.get_string(&self.document.transact()))
    }
    fn cut(&self) -> Self::Cut {
        self.document.transact().snapshot()
    }
    fn save(&mut self) -> Result<Vec<u8>> {
        Ok(self
            .document
            .transact()
            .encode_state_as_update_v1(&StateVector::default()))
    }

    fn load(bytes: &[u8]) -> Result<Self> {
        let document = yrs_document(3);
        {
            let mut tx = document.transact_mut();
            tx.apply_update(Update::decode_v1(bytes)?)?;
            if tx.store().pending_update().is_some() || tx.store().pending_ds().is_some() {
                return Err("full Yrs fixture unexpectedly has pending dependencies".into());
            }
            tx.commit();
        }
        let text = document.get_or_insert_text("text");
        Ok(Self { document, text })
    }

    fn text_at(&mut self, cut: &Self::Cut) -> Result<String> {
        let mut encoder = EncoderV1::new();
        self.document
            .transact()
            .encode_state_from_snapshot(cut, &mut encoder)?;
        Self::load(&encoder.to_vec())?.text()
    }
}

fn loro_document(peer: u64) -> Result<LoroDoc> {
    let document = LoroDoc::new();
    document.set_peer_id(peer)?;
    document.set_record_timestamp(false);
    document.set_change_merge_interval(-1);
    Ok(document)
}

pub(crate) struct LoroText {
    document: LoroDoc,
}

impl NativeText for LoroText {
    const NAME: &'static str = "loro";
    type Cut = Frontiers;

    fn seed(baseline: &str) -> Result<Self> {
        let document = loro_document(1)?;
        if !baseline.is_empty() {
            document.get_text("text").insert(0, baseline)?;
        }
        document.commit_with(loro::CommitOptions::new().timestamp(0));
        document.set_peer_id(2)?;
        Ok(Self { document })
    }

    fn splice(&mut self, edit: &Edit<'_>) -> Result<Duration> {
        let text = self.document.get_text("text");
        let options = loro::CommitOptions::new().timestamp(0);
        let started = Instant::now();
        if edit.scalar_delete != 0 {
            text.delete(edit.scalar_start, edit.scalar_delete)?;
        }
        if !edit.inserted.is_empty() {
            text.insert(edit.scalar_start, edit.inserted)?;
        }
        self.document.commit_with(options);
        Ok(started.elapsed())
    }

    fn text(&self) -> Result<String> {
        Ok(self.document.get_text("text").to_string())
    }
    fn cut(&self) -> Self::Cut {
        self.document.oplog_frontiers()
    }
    fn save(&mut self) -> Result<Vec<u8>> {
        Ok(self.document.export(ExportMode::Snapshot)?)
    }

    fn load(bytes: &[u8]) -> Result<Self> {
        let document = loro_document(3)?;
        if document.import(bytes)?.pending.is_some() || document.is_shallow() {
            return Err("expected complete, non-shallow Loro fixture history".into());
        }
        Ok(Self { document })
    }

    fn text_at(&mut self, cut: &Self::Cut) -> Result<String> {
        self.document.checkout(cut)?;
        self.text()
    }
}

pub(crate) struct DiamondText {
    log: ListOpLog,
    view: ListBranch,
    writer: AgentId,
}

impl NativeText for DiamondText {
    const NAME: &'static str = "diamond-types";
    type Cut = RemoteFrontierOwned;

    fn seed(baseline: &str) -> Result<Self> {
        let mut log = ListOpLog::new();
        let seed = log.get_or_create_agent_id("comparison-seed");
        let mut view = log.checkout_tip();
        if !baseline.is_empty() {
            view.insert(&mut log, seed, 0, baseline);
        }
        let writer = log.get_or_create_agent_id("comparison-writer");
        Ok(Self { log, view, writer })
    }

    fn splice(&mut self, edit: &Edit<'_>) -> Result<Duration> {
        let started = Instant::now();
        if edit.scalar_delete != 0 {
            self.view.delete(
                &mut self.log,
                self.writer,
                edit.scalar_start..edit.scalar_start + edit.scalar_delete,
            );
        }
        if !edit.inserted.is_empty() {
            self.view
                .insert(&mut self.log, self.writer, edit.scalar_start, edit.inserted);
        }
        Ok(started.elapsed())
    }

    fn text(&self) -> Result<String> {
        Ok(self.view.content().to_string())
    }
    fn cut(&self) -> Self::Cut {
        self.log
            .cg
            .agent_assignment
            .local_to_remote_frontier_owned(self.view.local_frontier_ref())
    }
    fn save(&mut self) -> Result<Vec<u8>> {
        Ok(self
            .log
            .encode(&EncodeOptions::full().compress_content(false)))
    }

    fn load(bytes: &[u8]) -> Result<Self> {
        let mut log = ListOpLog::load_from(bytes)?;
        let view = log.checkout_tip();
        let writer = log.get_or_create_agent_id("comparison-writer");
        Ok(Self { log, view, writer })
    }

    fn text_at(&mut self, cut: &Self::Cut) -> Result<String> {
        let local = self
            .log
            .cg
            .agent_assignment
            .try_remote_to_local_frontier(cut.iter())
            .map_err(|error| format!("cannot resolve diamond-types fixture cut: {error:?}"))?;
        Ok(self.log.checkout(local.as_ref()).content().to_string())
    }
}
