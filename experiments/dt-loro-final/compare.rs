// SPDX-License-Identifier: Apache-2.0
//! Bounded integrated DT/Loro experiment; not production admission, authentication,
//! an engine-selection decision, or a hostile-input resource/panic-safety proof.
//!
//! Protocol: `compare correctness` or `compare bench ENGINE CASE PREFILL`.
//! ENGINE is exactly dt|loro; CASE is ascii10k|unicode100k|churn1m.
//! The approved supplemental CASE churn1m_bounded changes ONLY the combined
//! replacement from 4096 to 512 scalars. Original churn1m is retained unchanged:
//! Loro's native 4096-byte ChangeStore block splitting can violate the existing
//! fixture's one-native-change restriction; a rejected cell fails, not shrinks.
//! Baselines contain 10 KiB ASCII bytes, or Unicode within three bytes below
//! 100 KiB / 1 MiB, truncated at a UTF-8 scalar boundary. All native positions
//! are Unicode scalar positions; edits cross the fixture boundary as checked
//! UTF-8 byte ranges.
//!
//! Bench CSV has one header and exactly 64 lossless integer-nanosecond sample
//! rows, including every sample, with no warmup or trimming. Four writers rotate.
//! `_ns` fields measure wall time with Instant. author includes exact-base
//! capture, private clone/fork/import, authoring, encoding and fixture digest.
//! inspect includes the COMPLETE binding inspection, private staging and
//! captured-base teardown.
//! publish measures publishing the inspected stage (not importing it a second
//! time). anchor_create includes two native-identity anchors and Loro cursor
//! serialization; anchor_resolve includes new and saved anchors. historical_read
//! includes native checkout/fork plus text materialization at a stable saved cut.
//! save/load are full native history export/import, repeated on EVERY sample;
//! load also includes reloaded text/anchor/history probes and load/save teardown.
//! digest_retain is ADDITIONAL outer SHA-256 + original-byte copying/indexing.
//! total encloses all those operations, private reconstruction, cut capture,
//! native text materialization, checks and scratch teardown; no output I/O.
//! Independent String oracle editing, reporting hashes and saved oracle-text
//! copies are outside total/component intervals. Comparisons ARE timed.
//!
//! retained_original_bytes includes the original seed, ALL trusted-generated
//! prefill updates, and all measured updates through this row; digest bytes are
//! 32 per retained artifact. trusted_prefill_original_bytes is the seed+prefill
//! subset, not validated admissions. snapshot_bytes is a separate native full
//! history component. These are NOT full database storage or allocator/RSS totals.
//! Native-only prefill is generated at setup, never called validated admission.
//!
//! Algorithmic asymmetry is intentional, not hidden: DT exact-tip bases use
//! native clones and decode_new_strict with original owned envelope metadata and
//! semantic checks; Loro uses the complete existing snapshot/fork/reconstruction
//! fixture, including prefix JSON inspection and exact reconstructed export.
//! DT anchors scan patched native tombstone spans; Loro uses public cursors.
//! DT Left/Right gap affinity selects different neighboring IDs; Loro native
//! Side does not promise that same neighbor-selection policy. Only each
//! policy's own import/reload invariants are required. The abc -> aZc control
//! resolves DT Left/Right at old gap 1 to 1/2, and Loro Left/Right to 2/2.

#[allow(dead_code)]
#[path = "loro_fit.rs"]
pub(crate) mod loro_fit;

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ops::Range,
    time::Instant,
};

use diamond_types::{
    Frontier, HasLength,
    causalgraph::agent_assignment::remote_ids::RemoteVersionOwned,
    list::{ListOpLog, encoding::EncodeOptions, operation::ListOpKind},
};
use loro::{
    CommitOptions, ExportMode, Frontiers, LoroDoc,
    cursor::{Cursor, Side},
};
use sha2::{Digest, Sha256};

const EPOCH: [u8; 32] = [7; 32];
const SAMPLES: usize = 64;
const SEEDS: usize = 32;
const BATCHES: usize = 24;
type Cut = Vec<RemoteVersionOwned>;
type Check<T> = Result<T, String>;

fn need(condition: bool, message: &str) -> Check<()> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}

fn writer(index: usize) -> loro_fit::FixtureWriter {
    loro_fit::FixtureWriter {
        public_key: [u8::try_from(index + 1).expect("bounded writer"); 32],
        branch: [1; 16],
        writer_epoch: [1; 16],
    }
}

fn dt_name(epoch: &[u8; 32], owner: &loro_fit::FixtureWriter) -> String {
    let digest = Sha256::new()
        .chain_update(b"puyudb/diamond-fit/writer/i02\0")
        .chain_update(owner.public_key)
        .chain_update(owner.branch)
        .chain_update(owner.writer_epoch)
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

fn dt_message(epoch: &[u8; 32], owner: &loro_fit::FixtureWriter, seq: u64) -> String {
    format!(
        "puyudb/diamond-fit/i02 key={} branch={} writer_epoch={} epoch={} seq={seq} ordinal=0",
        loro_fit::hex(&owner.public_key),
        loro_fit::hex(&owner.branch),
        loro_fit::hex(&owner.writer_epoch),
        loro_fit::hex(epoch),
    )
}

fn dt_ids(log: &ListOpLog, cut: &[usize]) -> Cut {
    let mut ids: Cut = log
        .cg
        .agent_assignment
        .local_to_remote_frontier_owned(cut)
        .into_iter()
        .collect();
    ids.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    ids
}

fn dt_tip(log: &ListOpLog) -> Cut {
    dt_ids(log, log.local_frontier_ref())
}

fn dt_local(log: &ListOpLog, cut: &Cut) -> Frontier {
    log.cg
        .agent_assignment
        .try_remote_to_local_frontier(cut.iter())
        .expect("saved native cut must be available")
}

fn dt_save(log: &ListOpLog) -> Vec<u8> {
    log.encode(&EncodeOptions::full().compress_content(false))
}

fn dt_next(log: &ListOpLog, name: &str) -> usize {
    log.iter_remote_mappings()
        .filter(|span| span.0 == name)
        .map(|span| span.1.end)
        .max()
        .unwrap_or(0)
}

#[derive(Clone)]
struct Edit {
    range: Range<usize>,
    inserted: String,
}

fn scalar_edit(text: &str, range: Range<usize>, inserted: &str) -> Edit {
    let byte = |n| {
        text.char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .nth(n)
            .expect("scalar oracle range")
    };
    Edit {
        range: byte(range.start)..byte(range.end),
        inserted: inserted.to_owned(),
    }
}

fn oracle_edit(text: &str, edit: &Edit) -> String {
    let mut expected = text.to_owned();
    expected.replace_range(edit.range.clone(), &edit.inserted);
    expected
}

fn dt_edit(log: &mut ListOpLog, name: &str, edit: &Edit) -> Check<()> {
    let agent = log.get_or_create_agent_id(name);
    let mut branch = log.checkout_tip();
    let text = branch.content().to_string();
    need(
        edit.range.start <= edit.range.end
            && text.is_char_boundary(edit.range.start)
            && text.is_char_boundary(edit.range.end),
        "invalid UTF-8 byte range",
    )?;
    let start = text[..edit.range.start].chars().count();
    let deleted = text[edit.range.clone()].chars().count();
    if deleted != 0 {
        branch.delete(log, agent, start..start + deleted);
    }
    if !edit.inserted.is_empty() {
        branch.insert(log, agent, start, &edit.inserted);
    }
    Ok(())
}

fn dt_author(
    base: &ListOpLog,
    owner: &loro_fit::FixtureWriter,
    seq: u64,
    edit: &Edit,
) -> Check<loro_fit::CapturedUpdate> {
    need(seq != 0, "zero outer sequence")?;
    let mut log = base.clone();
    dt_edit(&mut log, &dt_name(&EPOCH, owner), edit)?;
    need(log.len() > base.len(), "empty update")?;
    let msg = dt_message(&EPOCH, owner, seq);
    Ok(loro_fit::CapturedUpdate::new(
        log.encode_from(
            &EncodeOptions::patch()
                .compress_content(false)
                .user_data(msg.as_bytes()),
            base.local_frontier_ref(),
        ),
    ))
}

/// Inspect ALL new submitted original operations before publishing a native
/// stage. Parent topology plus positional bounds binds deletions to the exact
/// observed text; no receiver-tip substitution, target-ID copying, or reencoding.
fn dt_inspect(
    base: &ListOpLog,
    epoch: &[u8; 32],
    owner: &loro_fit::FixtureWriter,
    seq: u64,
    update: &loro_fit::CapturedUpdate,
) -> Check<ListOpLog> {
    need(epoch == &EPOCH && seq != 0, "epoch/sequence")?;
    need(update.bytes.len() <= 8 * 1024 * 1024, "encoded input limit")?;
    need(
        <[u8; 32]>::from(Sha256::digest(&update.bytes)) == update.sha256,
        "original digest mismatch",
    )?;
    let (stage, info) = base
        .decode_new_strict(&update.bytes)
        .map_err(|e| e.to_string())?;
    let msg = dt_message(epoch, owner, seq);
    need(
        info.doc_id.is_none()
            && info.user_data.as_deref() == Some(msg.as_bytes())
            && info.start_content.is_none(),
        "original metadata",
    )?;
    need(
        dt_ids(&stage, info.start_frontier.as_ref()) == dt_tip(base),
        "exact intent cut",
    )?;
    let name = dt_name(epoch, owner);
    let mut declarations = BTreeSet::new();
    for declared in &info.agent_names {
        need(
            declarations.insert(declared.as_str()),
            "duplicate agent declaration",
        )?;
        need(
            declared == &name || base.iter_remote_mappings().any(|span| span.0 == declared),
            "foreign agent declaration",
        )?;
    }
    need(
        info.submitted.start == base.len()
            && info.submitted.end == stage.len()
            && !info.submitted.is_empty()
            && info.submitted.len() <= 1024 * 1024,
        "submitted range",
    )?;
    let mut replay = base.clone();
    let agent = replay.get_or_create_agent_id(&name);
    let mut view = replay.checkout_tip();
    let mut next = dt_next(base, &name);
    let mut local_next = info.submitted.start;
    let mut parents = dt_tip(base);
    for (op, history, remote) in stage.iter_full_range(info.submitted) {
        need(
            remote.0 == name
                && remote.1.start == next
                && remote.1.len() == op.len()
                && history.span.len() == op.len()
                && history.span.start == local_next
                && !op.is_empty(),
            "owner/counters/coverage",
        )?;
        need(
            dt_ids(&stage, history.parents.as_ref()) == parents,
            "parent topology",
        )?;
        match op.kind {
            ListOpKind::Ins => {
                let content = op.content_as_str().ok_or("missing insertion content")?;
                need(
                    op.loc.fwd
                        && op.start() <= view.len()
                        && content.chars().count() == op.len()
                        && content.len() <= 1024 * 1024,
                    "text insertion bounds/content",
                )?;
                view.insert(&mut replay, agent, op.start(), content);
            }
            ListOpKind::Del => {
                need(
                    op.start() < op.end() && op.end() <= view.len() && op.content.is_none(),
                    "text deletion scope",
                )?;
                if op.loc.fwd {
                    view.delete(&mut replay, agent, op.start()..op.end());
                } else {
                    for pos in (op.start()..op.end()).rev() {
                        view.delete(&mut replay, agent, pos..pos + 1);
                    }
                }
            }
        }
        next = next.checked_add(op.len()).ok_or("counter overflow")?;
        local_next = history.span.end;
        parents = dt_ids(&stage, &[history.span.end - 1]);
    }
    need(
        local_next == info.submitted.end && next - dt_next(base, &name) == info.submitted.len(),
        "complete submitted coverage",
    )?;
    need(
        dt_ids(&stage, info.end_frontier.as_ref()) == parents
            && dt_tip(&stage) == parents
            && dt_tip(&replay) == parents,
        "end frontier",
    )?;
    need(
        view.content() == stage.checkout_tip().content(),
        "native replay text",
    )?;
    Ok(stage)
}

/// Fixed small fixture for the separately supervised security screen.
#[allow(dead_code)] // Called by the separate screening binary and this test target.
pub(crate) fn dt_screen_fixture() -> (ListOpLog, Vec<u8>) {
    let state = State::seed(Engine::Dt, "A😀bc");
    let State::Dt(base) = state else {
        unreachable!()
    };
    let edit = Edit {
        range: 5..6,
        inserted: "日本".to_owned(),
    };
    let update = dt_author(&base, &writer(0), 1, &edit).expect("screen fixture author");
    (base, update.bytes)
}

/// The screen invokes the SAME full semantic inspector, with a fresh digest.
#[allow(dead_code)] // Called by the separate screening binary and this test target.
pub(crate) fn dt_screen_inspect(base: &ListOpLog, bytes: &[u8]) -> Check<()> {
    dt_inspect(
        base,
        &EPOCH,
        &writer(0),
        1,
        &loro_fit::CapturedUpdate::new(bytes.to_vec()),
    )
    .map(|_| ())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    Dt,
    Loro,
}

impl Engine {
    fn name(self) -> &'static str {
        match self {
            Self::Dt => "dt",
            Self::Loro => "loro",
        }
    }
}

// Only a handful of native states are live; avoid a benchmark-only allocation.
#[allow(clippy::large_enum_variant)]
enum State {
    Dt(ListOpLog),
    Loro(LoroDoc),
}
enum Base {
    Dt(ListOpLog),
    Loro(loro_fit::FixtureBase),
}
#[derive(Clone)]
enum SavedCut {
    Dt(Cut),
    Loro(Frontiers),
}

impl State {
    fn seed(engine: Engine, text: &str) -> Self {
        match engine {
            Engine::Dt => {
                let mut log = ListOpLog::new();
                if !text.is_empty() {
                    let digest = Sha256::new()
                        .chain_update(b"puyudb/diamond-fit/seed/i02\0")
                        .chain_update(EPOCH)
                        .finalize();
                    let seed_name = format!("s:{}", base64url(&digest));
                    let seed = log.get_or_create_agent_id(&seed_name);
                    log.add_insert(seed, 0, text);
                }
                Self::Dt(log)
            }
            Engine::Loro => Self::Loro(
                loro_fit::FixtureBase::seed(EPOCH, text)
                    .unwrap()
                    .document()
                    .unwrap(),
            ),
        }
    }

    fn capture(&self) -> Base {
        match self {
            Self::Dt(log) => Base::Dt(log.clone()),
            Self::Loro(doc) => Base::Loro(
                loro_fit::FixtureBase::capture(EPOCH, doc, doc.oplog_frontiers()).unwrap(),
            ),
        }
    }

    fn cut(&self) -> SavedCut {
        match self {
            Self::Dt(log) => SavedCut::Dt(dt_tip(log)),
            Self::Loro(doc) => SavedCut::Loro(doc.oplog_frontiers()),
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Dt(log) => log.checkout_tip().content().to_string(),
            Self::Loro(doc) => doc.get_text("text").to_string(),
        }
    }

    fn text_at(&self, cut: &SavedCut) -> String {
        match (self, cut) {
            (Self::Dt(log), SavedCut::Dt(cut)) => log
                .checkout(dt_local(log, cut).as_ref())
                .content()
                .to_string(),
            (Self::Loro(doc), SavedCut::Loro(cut)) => {
                doc.fork_at(cut).unwrap().get_text("text").to_string()
            }
            _ => unreachable!(),
        }
    }

    fn save(&self) -> Vec<u8> {
        match self {
            Self::Dt(log) => dt_save(log),
            Self::Loro(doc) => doc.export(ExportMode::Snapshot).unwrap(),
        }
    }

    fn load(engine: Engine, bytes: &[u8]) -> Self {
        match engine {
            Engine::Dt => Self::Dt(ListOpLog::load_from(bytes).unwrap()),
            Engine::Loro => {
                let doc = loro_fit::document(99).unwrap();
                assert!(doc.import(bytes).unwrap().pending.is_none());
                Self::Loro(doc)
            }
        }
    }

    fn merge(&mut self, bytes: &[u8]) {
        match self {
            Self::Dt(log) => {
                log.decode_and_add(bytes).unwrap();
            }
            Self::Loro(doc) => {
                assert!(doc.import(bytes).unwrap().pending.is_none());
            }
        }
    }

    fn anchor(&self, pos: usize, right: bool) -> Anchor {
        match self {
            Self::Dt(log) => Anchor::Dt(DtAnchor::create(log, pos, right)),
            Self::Loro(doc) => Anchor::Loro(
                doc.get_text("text")
                    .get_cursor(pos, if right { Side::Right } else { Side::Left })
                    .unwrap()
                    .encode(),
            ),
        }
    }

    fn resolve_at(&self, anchor: &Anchor, cut: &SavedCut) -> Option<(usize, i8)> {
        match (self, cut) {
            (Self::Dt(log), SavedCut::Dt(cut)) => {
                let Anchor::Dt(anchor) = anchor else {
                    unreachable!()
                };
                anchor
                    .resolve(log, dt_local(log, cut).as_ref())
                    .map(|pos| (pos, 0))
            }
            (Self::Loro(doc), SavedCut::Loro(cut)) => {
                let historical = doc.fork_at(cut).unwrap();
                Self::loro_resolve(&historical, anchor)
            }
            _ => unreachable!(),
        }
    }

    fn loro_resolve(doc: &LoroDoc, anchor: &Anchor) -> Option<(usize, i8)> {
        let Anchor::Loro(bytes) = anchor else {
            unreachable!()
        };
        let cursor = Cursor::decode(bytes).unwrap();
        doc.get_cursor_pos(&cursor).ok().map(|p| {
            (
                p.current.pos,
                match p.current.side {
                    Side::Left => -1,
                    Side::Middle => 0,
                    Side::Right => 1,
                },
            )
        })
    }

    fn resolve(&self, anchor: &Anchor) -> Option<(usize, i8)> {
        match (self, anchor) {
            (Self::Dt(log), Anchor::Dt(anchor)) => anchor
                .resolve(log, log.local_frontier_ref())
                .map(|pos| (pos, 0)),
            (Self::Loro(doc), Anchor::Loro(_)) => Self::loro_resolve(doc, anchor),
            _ => unreachable!(),
        }
    }

    fn trusted_prefill(&mut self, index: usize) -> Vec<u8> {
        match self {
            Self::Dt(log) => {
                let from = log.local_frontier();
                let agent = log.get_or_create_agent_id(&dt_name(&EPOCH, &writer(index % 4)));
                let mut view = log.checkout_tip();
                let pos = index % view.len();
                view.delete(log, agent, pos..pos + 1);
                view.insert(
                    log,
                    agent,
                    pos,
                    if index.is_multiple_of(2) { "a" } else { "😀" },
                );
                log.encode_from(
                    &EncodeOptions::patch()
                        .compress_content(false)
                        .user_data(b"trusted generated prefill; NOT inspected admission"),
                    from.as_ref(),
                )
            }
            Self::Loro(doc) => {
                doc.set_peer_id(u64::try_from(index % 4 + 2).unwrap())
                    .unwrap();
                let from = doc.oplog_vv();
                let text = doc.get_text("text");
                let pos = index % text.len_unicode();
                text.delete(pos, 1).unwrap();
                text.insert(pos, if index.is_multiple_of(2) { "a" } else { "😀" })
                    .unwrap();
                doc.commit_with(CommitOptions::new().timestamp(0).commit_msg(&format!(
                    "trusted-generated-prefill-{index}; NOT inspected admission"
                )));
                doc.export(ExportMode::updates(&from)).unwrap()
            }
        }
    }
}

fn binding(owner: loro_fit::FixtureWriter, epoch: [u8; 32], peer: u64) -> loro_fit::Binding {
    loro_fit::EpochPeers::new(epoch).bind(owner, peer).unwrap()
}

impl Base {
    fn author(&self, index: usize, seq: u64, edit: &Edit) -> loro_fit::CapturedUpdate {
        match self {
            Self::Dt(log) => dt_author(log, &writer(index), seq, edit).unwrap(),
            Self::Loro(base) => loro_fit::author(
                base,
                &binding(writer(index), EPOCH, u64::try_from(index + 2).unwrap()),
                seq,
                &[loro_fit::ByteEdit {
                    range: edit.range.clone(),
                    inserted: &edit.inserted,
                }],
            )
            .unwrap(),
        }
    }

    fn inspect(&self, index: usize, seq: u64, update: &loro_fit::CapturedUpdate) -> Check<State> {
        match self {
            Self::Dt(log) => dt_inspect(log, &EPOCH, &writer(index), seq, update).map(State::Dt),
            Self::Loro(base) => loro_fit::inspect(
                base,
                &binding(writer(index), EPOCH, u64::try_from(index + 2).unwrap()),
                seq,
                update,
            )
            .map(State::Loro)
            .map_err(|e| {
                format!(
                    "{e}; original_blob_meta={:?}",
                    LoroDoc::decode_import_blob_meta(&update.bytes, true)
                )
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DtAnchor {
    Start,
    End,
    Item(RemoteVersionOwned, bool),
}
#[derive(Clone)]
enum Anchor {
    Dt(DtAnchor),
    Loro(Vec<u8>),
}

impl DtAnchor {
    fn create(log: &ListOpLog, pos: usize, right: bool) -> Self {
        let spans = log.insertion_spans_at(log.local_frontier_ref());
        let length: usize = spans
            .iter()
            .filter(|(_, visible)| *visible)
            .map(|(range, _)| range.len())
            .sum();
        assert!(pos <= length);
        if !right && pos == 0 {
            return Self::Start;
        }
        if right && pos == length {
            return Self::End;
        }
        let mut target = if right { pos } else { pos - 1 };
        for (range, visible) in spans {
            if !visible {
                continue;
            }
            if target < range.len() {
                // A native LV range may cross agent assignments. Convert only
                // this exact LV, never extrapolate the first remote ID.
                return Self::Item(
                    log.cg
                        .agent_assignment
                        .local_to_remote_version(range.start + target)
                        .to_owned(),
                    !right,
                );
            }
            target -= range.len();
        }
        unreachable!()
    }

    fn resolve(&self, log: &ListOpLog, cut: &[usize]) -> Option<usize> {
        let spans = log.insertion_spans_at(cut);
        let (primary, after) = match self {
            Self::Start => return Some(0),
            Self::End => {
                return Some(
                    spans
                        .iter()
                        .filter(|(_, visible)| *visible)
                        .map(|(range, _)| range.len())
                        .sum(),
                );
            }
            Self::Item(primary, after) => (primary, after),
        };
        let lv = log
            .cg
            .agent_assignment
            .try_remote_to_local_version(primary.into())
            .ok()?;
        let mut pos = 0;
        for (range, visible) in spans {
            if (range.start..range.end).contains(&lv) {
                return Some(
                    pos + if visible {
                        lv - range.start + usize::from(*after)
                    } else {
                        0
                    },
                );
            }
            if visible {
                pos += range.len();
            }
        }
        None
    }
}

#[derive(Default)]
struct Originals {
    // Full writer identities are fixed in this process; index+sequence is the
    // outer original ID, not a truncated native identity or a signature.
    entries: BTreeMap<(usize, u64), loro_fit::CapturedUpdate>,
    trusted: Vec<loro_fit::CapturedUpdate>,
}

impl Originals {
    fn record(&mut self, id: (usize, u64), bytes: &[u8]) -> Check<bool> {
        let captured = loro_fit::CapturedUpdate::new(bytes.to_vec());
        if let Some(prior) = self.entries.get(&id) {
            need(
                prior.sha256 == captured.sha256 && prior.bytes == captured.bytes,
                "same outer ID with different original bytes",
            )?;
            return Ok(false);
        }
        self.entries.insert(id, captured);
        Ok(true)
    }

    fn retain_trusted(&mut self, bytes: Vec<u8>) {
        self.trusted.push(loro_fit::CapturedUpdate::new(bytes));
    }

    fn bytes(&self) -> usize {
        self.trusted_bytes()
            + self
                .entries
                .values()
                .map(|entry| entry.bytes.len())
                .sum::<usize>()
    }

    fn trusted_bytes(&self) -> usize {
        self.trusted.iter().map(|entry| entry.bytes.len()).sum()
    }

    fn digest_bytes(&self) -> usize {
        (self.entries.len() + self.trusted.len()) * 32
    }
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, u128) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed().as_nanos())
}

fn baseline(case: &str) -> String {
    match case {
        "ascii10k" => "a".repeat(10 * 1024),
        "unicode100k" => unicode_bytes(100 * 1024),
        "churn1m" | "churn1m_bounded" => unicode_bytes(1024 * 1024),
        _ => unreachable!(),
    }
}

fn unicode_bytes(bytes: usize) -> String {
    let pattern = "a😀日e\u{301}";
    let mut text = pattern.repeat(bytes.div_ceil(pattern.len()));
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}

fn workload(case: &str, sample: usize, text: &str) -> Edit {
    let len = text.chars().count();
    if matches!(case, "churn1m" | "churn1m_bounded") {
        match sample % 8 {
            0 => return scalar_edit(text, 256..256, &"😀a日e\u{301}".repeat(3277)),
            1 => return scalar_edit(text, 256..256 + 16_385, ""),
            2 => {
                let replaced = if case == "churn1m" { 4096 } else { 512 };
                return scalar_edit(
                    text,
                    len / 3..len / 3 + replaced,
                    &"a😀".repeat(replaced / 2),
                );
            }
            _ => {}
        }
    }
    let pos = (sample * 7919 + 17) % (len - 8);
    match sample % 4 {
        0 => scalar_edit(text, pos..pos, "X"),
        1 => scalar_edit(text, pos..pos + 3, ""),
        2 => scalar_edit(text, pos..pos + 2, "日本😀"),
        3 => scalar_edit(text, pos..pos + 1, "e\u{301}"),
        _ => unreachable!(),
    }
}

fn bench(engine: Engine, case: &str, prefill: usize) {
    let mut state = State::seed(engine, &baseline(case));
    let mut originals = Originals::default();
    originals.retain_trusted(state.save());
    for index in 0..prefill {
        originals.retain_trusted(state.trusted_prefill(index));
    }
    let mut oracle = state.text();
    // Independently repeat trusted setup on a scalar vector, not native text.
    let mut setup_oracle: Vec<char> = baseline(case).chars().collect();
    for index in 0..prefill {
        let pos = index % setup_oracle.len();
        setup_oracle[pos] = if index % 2 == 0 { 'a' } else { '😀' };
    }
    assert_eq!(oracle, setup_oracle.into_iter().collect::<String>());
    let mut cuts = vec![(state.cut(), oracle.clone())];
    let stable = [state.anchor(1, false), state.anchor(1, true)];
    println!(
        "schema_version,engine,case,prefill,trusted_generated_prefill,sample,writer,author_ns,inspect_ns,publish_ns,anchor_create_ns,anchor_resolve_ns,historical_read_ns,save_ns,load_ns,digest_retain_ns,total_ns,original_update_bytes,retained_original_bytes,retained_digest_bytes,trusted_prefill_original_bytes,snapshot_bytes,text_scalars,text_utf8_bytes,text_sha256"
    );
    for sample in 0..SAMPLES {
        let index = sample % 4;
        let seq = u64::try_from(sample / 4 + 1).unwrap();
        let edit = workload(case, sample, &oracle);
        let expected = oracle_edit(&oracle, &edit);
        let anchor_pos = oracle[..edit.range.start].chars().count();
        let history = &cuts[sample % cuts.len()];
        let total = Instant::now();
        let (anchors, anchor_create_ns) = timed(|| {
            [
                state.anchor(anchor_pos, false),
                state.anchor(anchor_pos, true),
            ]
        });
        let ((base, update), author_ns) = timed(|| {
            let base = state.capture();
            let update = base.author(index, seq, &edit);
            (base, update)
        });
        let (stage, inspect_ns) = timed(|| {
            let stage = base.inspect(index, seq, &update).unwrap_or_else(|error| {
                panic!(
                    "engine={} case={case} prefill={prefill} sample={sample}: {error}",
                    engine.name()
                )
            });
            drop(base);
            stage
        });
        let (cut, publish_ns) = timed(|| {
            state = stage;
            state.cut()
        });
        let (positions, anchor_resolve_ns) = timed(|| {
            anchors
                .iter()
                .chain(stable.iter())
                .map(|anchor| state.resolve(anchor))
                .collect::<Vec<_>>()
        });
        let (historical, historical_read_ns) = timed(|| state.text_at(&history.0));
        let (saved, save_ns) = timed(|| state.save());
        let (snapshot_bytes, load_ns) = timed(|| {
            let loaded = State::load(engine, &saved);
            assert_eq!(loaded.text(), expected);
            for (anchor, position) in anchors.iter().chain(stable.iter()).zip(&positions) {
                assert_eq!(loaded.resolve(anchor), *position);
            }
            assert_eq!(loaded.text_at(&cuts[0].0), cuts[0].1);
            let size = saved.len();
            drop(loaded);
            drop(saved);
            size
        });
        let (original_update_bytes, digest_retain_ns) = timed(|| {
            assert!(originals.record((index, seq), &update.bytes).unwrap());
            let size = update.bytes.len();
            drop(update);
            size
        });
        let actual = state.text();
        assert_eq!(actual, expected, "{} sample {sample}", engine.name());
        assert_eq!(historical, history.1);
        for position in &positions {
            assert!(position.is_some());
        }
        drop(actual);
        drop(historical);
        drop(positions);
        drop(anchors);
        let retained_cut = if sample % 8 == 7 {
            Some(cut)
        } else {
            drop(cut);
            None
        };
        let total_ns = total.elapsed().as_nanos();
        // Oracle generation/reporting and retained oracle-copy cost are NOT timed.
        oracle = expected;
        if let Some(cut) = retained_cut {
            cuts.push((cut, oracle.clone()));
        }
        let digest = loro_fit::hex(&Sha256::digest(oracle.as_bytes()));
        println!(
            "1,{},{case},{prefill},{prefill},{sample},{index},{author_ns},{inspect_ns},{publish_ns},{anchor_create_ns},{anchor_resolve_ns},{historical_read_ns},{save_ns},{load_ns},{digest_retain_ns},{total_ns},{},{},{},{},{},{},{},{}",
            engine.name(),
            original_update_bytes,
            originals.bytes(),
            originals.digest_bytes(),
            originals.trusted_bytes(),
            snapshot_bytes,
            oracle.chars().count(),
            oracle.len(),
            digest
        );
    }
    assert_eq!(originals.entries.len(), SAMPLES);
}

fn draw(state: &mut u64, bound: usize) -> usize {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
    usize::try_from(*state >> 32).unwrap() % bound
}

struct Batch {
    writer: usize,
    seq: u64,
    dependencies: BTreeSet<usize>,
    base: Base,
    update: loro_fit::CapturedUpdate,
    base_cut: SavedCut,
    before: String,
    cut: SavedCut,
    expected: String,
    anchors: Vec<Anchor>,
    anchor_pos: usize,
}

#[derive(Default)]
struct Counts {
    authored: usize,
    inspections: usize,
    published: usize,
    deferred: usize,
    duplicates: usize,
    reloads: usize,
    historical: usize,
    anchors: usize,
}

fn observed(seed: &[u8], engine: Engine, batches: &[Batch], seen: &BTreeSet<usize>) -> State {
    let mut state = State::load(engine, seed);
    for &index in seen {
        state.merge(&batches[index].update.bytes);
    }
    state
}

fn check_receiver(
    state: &State,
    batches: &[Batch],
    counts: &mut Counts,
) -> Vec<Option<(usize, i8)>> {
    let mut resolved = Vec::new();
    for batch in batches {
        assert_eq!(state.text_at(&batch.base_cut), batch.before);
        assert_eq!(state.text_at(&batch.cut), batch.expected);
        counts.historical += 2;
        for anchor in &batch.anchors {
            resolved.push(state.resolve(anchor));
            assert!(resolved.last().unwrap().is_some());
            assert_eq!(
                state.resolve_at(anchor, &batch.base_cut).unwrap().0,
                batch.anchor_pos
            );
            counts.anchors += 2;
        }
    }
    resolved
}

fn deliver(
    seed: &[u8],
    engine: Engine,
    batches: &[Batch],
    order: &[usize],
    counts: &mut Counts,
) -> (String, Vec<Option<(usize, i8)>>) {
    let mut receiver = State::load(engine, seed);
    let mut arrived = BTreeSet::new();
    let mut applied = BTreeSet::new();
    let mut originals = Originals::default();
    for (step, &index) in order.iter().enumerate() {
        arrived.insert(index);
        if !batches[index].dependencies.is_subset(&applied) {
            counts.deferred += 1;
        }
        loop {
            let ready = arrived.iter().copied().find(|index| {
                !applied.contains(index) && batches[*index].dependencies.is_subset(&applied)
            });
            let Some(ready) = ready else { break };
            let batch = &batches[ready];
            // Receiver tip may include concurrent siblings. Validate this NEW
            // original at its DECLARED author base, NEVER the receiver tip.
            let checked = batch
                .base
                .inspect(batch.writer, batch.seq, &batch.update)
                .unwrap();
            assert_eq!(checked.text(), batch.expected);
            counts.inspections += 1;
            assert!(
                originals
                    .record((batch.writer, batch.seq), &batch.update.bytes)
                    .unwrap()
            );
            receiver.merge(&batch.update.bytes);
            applied.insert(ready);
            counts.published += 1;
            assert!(
                !originals
                    .record((batch.writer, batch.seq), &batch.update.bytes)
                    .unwrap()
            );
            counts.duplicates += 1;
            // Ordinary native duplicate handling is distinct from strict-new
            // inspection and from the exact original-byte archive.
            let before = receiver.text();
            receiver.merge(&batch.update.bytes);
            assert_eq!(receiver.text(), before);
        }
        if step % 5 == 2 {
            // Deferred original bytes live in the outer arrived/batch archive,
            // not an unsupported claim that native snapshots save pending input.
            receiver = State::load(engine, &receiver.save());
            counts.reloads += 1;
        }
    }
    assert_eq!(applied.len(), batches.len());
    let positions = check_receiver(&receiver, batches, counts);
    let loaded = State::load(engine, &receiver.save());
    counts.reloads += 1;
    assert_eq!(receiver.text(), loaded.text());
    assert_eq!(positions, check_receiver(&loaded, batches, counts));
    (loaded.text(), positions)
}

fn generated(engine: Engine, history: usize, counts: &mut Counts) {
    let seed = State::seed(engine, "a😀b日cde\u{301}fg");
    let seed_bytes = seed.save();
    let mut batches: Vec<Batch> = Vec::new();
    let mut seen: [BTreeSet<usize>; 4] = std::array::from_fn(|_| BTreeSet::new());
    let mut sequences = [0_u64; 4];
    let mut random = u64::try_from(history).unwrap() + 19;
    for batch in 0..BATCHES {
        let index = batch % 4;
        let other = draw(&mut random, 4);
        let observations = seen[other].clone();
        seen[index].extend(observations);
        if batch % 7 == 6 {
            seen[index].extend(0..batch);
        }
        let state = observed(&seed_bytes, engine, &batches, &seen[index]);
        let before = state.text();
        let len = before.chars().count();
        let start = draw(&mut random, len + 1);
        let mode = draw(&mut random, 3);
        let deleted = if mode != 0 && start < len {
            1 + draw(&mut random, (len - start).min(3))
        } else {
            0
        };
        let inserted = if mode == 1 && deleted != 0 {
            ""
        } else {
            ["a", "😀", "日本", "e\u{301}", "aa"][draw(&mut random, 5)]
        };
        let edit = scalar_edit(&before, start..start + deleted, inserted);
        let expected = oracle_edit(&before, &edit);
        let anchors = vec![state.anchor(start, false), state.anchor(start, true)];
        for anchor in &anchors {
            assert_eq!(state.resolve(anchor).unwrap().0, start);
            counts.anchors += 1;
        }
        let unchanged = state.save();
        let base = state.capture();
        sequences[index] += 1;
        let update = base.author(index, sequences[index], &edit);
        let checked = base.inspect(index, sequences[index], &update).unwrap();
        counts.authored += 1;
        counts.inspections += 1;
        assert_eq!(checked.text(), expected);
        assert_eq!(
            state.save(),
            unchanged,
            "author/inspect leave caller unchanged"
        );
        batches.push(Batch {
            writer: index,
            seq: sequences[index],
            dependencies: seen[index].clone(),
            base,
            update,
            base_cut: state.cut(),
            before,
            cut: checked.cut(),
            expected,
            anchors,
            anchor_pos: start,
        });
        seen[index].insert(batch);
    }
    let forward: Vec<_> = (0..BATCHES).collect();
    let reverse: Vec<_> = forward.iter().copied().rev().collect();
    let mut shuffled = forward.clone();
    for i in (1..BATCHES).rev() {
        shuffled.swap(i, draw(&mut random, i + 1));
    }
    // Partitioned peers reconnect with the opposite partition arriving last.
    let disconnected: Vec<_> = (0..BATCHES)
        .filter(|i| i % 2 == 1)
        .chain((0..BATCHES).filter(|i| i % 2 == 0))
        .collect();
    let mut reference = None;
    for order in [forward, reverse, shuffled, disconnected] {
        let result = deliver(&seed_bytes, engine, &batches, &order, counts);
        if let Some(reference) = &reference {
            assert_eq!(&result, reference);
        } else {
            reference = Some(result);
        }
    }
}

fn sequential_control(engine: Engine) -> String {
    let mut state = State::seed(engine, "A😀bc日de\u{301}");
    let mut expected = "A😀bc日de\u{301}".to_owned();
    let original_cut = state.cut();
    for sample in 0..24 {
        let len = expected.chars().count();
        let start = sample % (len - 1);
        let edit = scalar_edit(&expected, start..start + 1, ["a", "😀", "日本"][sample % 3]);
        expected = oracle_edit(&expected, &edit);
        let base = state.capture();
        let update = base.author(sample % 4, u64::try_from(sample / 4 + 1).unwrap(), &edit);
        state = base
            .inspect(sample % 4, u64::try_from(sample / 4 + 1).unwrap(), &update)
            .unwrap();
        assert_eq!(state.text(), expected);
        assert_eq!(state.text_at(&original_cut), "A😀bc日de\u{301}");
        state = State::load(engine, &state.save());
    }
    expected
}

fn fixed_regressions(engine: Engine) {
    let seed = State::seed(engine, "abc");
    let original_seed = seed.save();
    let base = seed.capture();
    let edit = scalar_edit("abc", 1..2, "日本😀");
    let valid = base.author(0, 1, &edit);
    let checked = base.inspect(0, 1, &valid).unwrap();
    assert_eq!(checked.text(), "a日本😀c");
    assert_eq!(seed.save(), original_seed);
    assert!(base.inspect(0, 0, &valid).is_err());
    assert!(base.inspect(0, 2, &valid).is_err());
    assert!(base.inspect(1, 1, &valid).is_err());
    assert!(
        checked.capture().inspect(0, 1, &valid).is_err(),
        "stale cut"
    );
    let mut corrupted = valid.clone();
    corrupted.bytes.push(0);
    assert!(base.inspect(0, 1, &corrupted).is_err(), "stale digest");
    let mut originals = Originals::default();
    assert!(originals.record((0, 1), &valid.bytes).unwrap());
    assert!(!originals.record((0, 1), &valid.bytes).unwrap());
    let different = base.author(0, 1, &scalar_edit("abc", 1..2, "Z"));
    assert!(
        originals.record((0, 1), &different.bytes).is_err(),
        "same-ID equivocation"
    );
    match &base {
        Base::Dt(log) => {
            for owner in [
                loro_fit::FixtureWriter {
                    public_key: [2; 32],
                    ..writer(0)
                },
                loro_fit::FixtureWriter {
                    branch: [2; 16],
                    ..writer(0)
                },
                loro_fit::FixtureWriter {
                    writer_epoch: [2; 16],
                    ..writer(0)
                },
            ] {
                assert!(dt_inspect(log, &EPOCH, &owner, 1, &valid).is_err());
            }
            assert!(dt_inspect(log, &[8; 32], &writer(0), 1, &valid).is_err());
            // Correct context cannot excuse an operation authored by a foreign owner.
            let mut foreign = log.clone();
            dt_edit(&mut foreign, "foreign", &edit).unwrap();
            let msg = dt_message(&EPOCH, &writer(0), 1);
            let bytes = foreign.encode_from(
                &EncodeOptions::patch()
                    .compress_content(false)
                    .user_data(msg.as_bytes()),
                log.local_frontier_ref(),
            );
            assert!(
                base.inspect(0, 1, &loro_fit::CapturedUpdate::new(bytes))
                    .is_err()
            );
        }
        Base::Loro(base) => {
            for owner in [
                loro_fit::FixtureWriter {
                    public_key: [2; 32],
                    ..writer(0)
                },
                loro_fit::FixtureWriter {
                    branch: [2; 16],
                    ..writer(0)
                },
                loro_fit::FixtureWriter {
                    writer_epoch: [2; 16],
                    ..writer(0)
                },
            ] {
                assert!(loro_fit::inspect(base, &binding(owner, EPOCH, 2), 1, &valid).is_err());
            }
            assert!(loro_fit::inspect(base, &binding(writer(0), [8; 32], 2), 1, &valid).is_err());
            let foreign = base.document().unwrap();
            let from = foreign.oplog_vv();
            foreign.set_peer_id(88).unwrap();
            foreign.get_text("text").insert(0, "X").unwrap();
            foreign.commit_with(
                CommitOptions::new()
                    .timestamp(0)
                    .commit_msg(&loro_fit::message(&binding(writer(0), EPOCH, 2), 1)),
            );
            let bytes = foreign.export(ExportMode::updates(&from)).unwrap();
            assert!(
                loro_fit::inspect(
                    base,
                    &binding(writer(0), EPOCH, 2),
                    1,
                    &loro_fit::CapturedUpdate::new(bytes)
                )
                .is_err()
            );
        }
    }
    // Original opposite-import-order witness: retain BOTH original replacement
    // byte strings and inspect each on BOTH equal closed bases.
    let left = base.author(0, 1, &scalar_edit("abc", 1..1, "X"));
    let right = base.author(1, 1, &scalar_edit("abc", 2..2, "Y"));
    assert_eq!(base.inspect(0, 1, &left).unwrap().text(), "aXbc");
    assert_eq!(base.inspect(1, 1, &right).unwrap().text(), "abYc");
    let mut orders = Vec::new();
    for order in [[&left, &right], [&right, &left]] {
        let mut state = State::load(engine, &original_seed);
        for update in order {
            state.merge(&update.bytes);
        }
        assert_eq!(state.text(), "aXbYc");
        orders.push(state);
    }
    let replacement = scalar_edit("aXbYc", 2..3, "Z");
    let originals: Vec<_> = orders
        .iter()
        .map(|state| state.capture().author(2, 1, &replacement))
        .collect();
    if engine == Engine::Dt {
        assert_ne!(originals[0].bytes, originals[1].bytes);
    }
    for state in &orders {
        for original in &originals {
            assert_eq!(
                state.capture().inspect(2, 1, original).unwrap().text(),
                "aXZYc"
            );
        }
    }
    let replacement_anchor = seed.anchor(1, true);
    let after = base.inspect(0, 1, &different).unwrap();
    let replacement_position = after.resolve(&replacement_anchor).unwrap().0;
    assert_eq!(replacement_position, 2);
    let left_anchor = seed.anchor(1, false);
    assert_eq!(
        after.resolve(&left_anchor).unwrap().0,
        if engine == Engine::Dt { 1 } else { 2 },
        "distinct native replacement policies"
    );
    let loaded = State::load(engine, &after.save());
    assert_eq!(
        loaded.resolve(&replacement_anchor),
        after.resolve(&replacement_anchor)
    );
    assert_eq!(loaded.resolve(&left_anchor), after.resolve(&left_anchor));
    deleted_anchor_witness(engine);
    resumed_writer_anchor_witness(engine);
}

fn resumed_writer_anchor_witness(engine: Engine) {
    let mut state = State::seed(engine, "");
    for (index, seq, expected, inserted) in [(0, 1, "", "a"), (1, 1, "a", "b"), (0, 2, "ab", "c")] {
        let edit = scalar_edit(expected, expected.len()..expected.len(), inserted);
        let base = state.capture();
        let original = base.author(index, seq, &edit);
        state = base.inspect(index, seq, &original).unwrap();
    }
    assert_eq!(state.text(), "abc");
    if let State::Dt(log) = &state {
        assert_eq!(
            log.insertion_spans_at(log.local_frontier_ref()),
            [((0..3).into(), true)],
            "one native span crosses resumed agent assignments"
        );
        let Anchor::Dt(DtAnchor::Item(id, false)) = state.anchor(2, true) else {
            panic!("item anchor")
        };
        assert_eq!(id.0.as_str(), dt_name(&EPOCH, &writer(0)));
        assert_eq!(id.1, 1);
    }
    let anchor = state.anchor(2, true);
    let loaded = State::load(engine, &state.save());
    assert_eq!(state.resolve(&anchor).unwrap().0, 2);
    assert_eq!(loaded.resolve(&anchor), state.resolve(&anchor));
}

fn deleted_anchor_witness(engine: Engine) {
    let seed = State::seed(engine, "ab");
    let bytes = seed.save();
    let base = seed.capture();
    let anchor = seed.anchor(0, true);
    let updates = [
        base.author(0, 1, &scalar_edit("ab", 0..0, "😀")),
        base.author(1, 1, &scalar_edit("ab", 2..2, "e\u{301}")),
        base.author(2, 1, &scalar_edit("ab", 0..2, "")),
    ];
    for (index, update) in updates.iter().enumerate() {
        base.inspect(index, 1, update).unwrap();
    }
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let mut receiver = State::load(engine, &bytes);
        for index in order {
            receiver.merge(&updates[index].bytes);
        }
        assert_eq!(receiver.text(), "😀e\u{301}");
        assert_eq!(receiver.resolve(&anchor).unwrap().0, 1);
        let loaded = State::load(engine, &receiver.save());
        assert_eq!(loaded.resolve(&anchor), receiver.resolve(&anchor));
        assert_eq!(loaded.resolve_at(&anchor, &seed.cut()).unwrap().0, 0);
    }
}

fn correctness() {
    assert_eq!(
        sequential_control(Engine::Dt),
        sequential_control(Engine::Loro)
    );
    for engine in [Engine::Dt, Engine::Loro] {
        fixed_regressions(engine);
        let mut counts = Counts::default();
        for history in 0..SEEDS {
            generated(engine, history, &mut counts);
        }
        assert_eq!(counts.authored, SEEDS * BATCHES);
        assert_eq!(counts.published, SEEDS * BATCHES * 4);
        assert!(counts.deferred > 0);
        println!(
            "mode=correctness,engine={},seeds={SEEDS},batches_per_seed={BATCHES},schedules=4,authored={},full_inspections={},receiver_publications={},deferred_arrivals={},exact_outer_duplicates={},reloads={},historical_checks={},anchor_checks={},fixed_regressions=passed,shared_sequential_oracle=passed,benchmark_timings=none",
            engine.name(),
            counts.authored,
            counts.inspections,
            counts.published,
            counts.deferred,
            counts.duplicates,
            counts.reloads,
            counts.historical,
            counts.anchors
        );
    }
}

fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "correctness" => correctness(),
        [mode, engine, case, prefill]
            if mode == "bench"
                && matches!(engine.as_str(), "dt" | "loro")
                && matches!(
                    case.as_str(),
                    "ascii10k" | "unicode100k" | "churn1m" | "churn1m_bounded"
                ) =>
        {
            let engine = if engine == "dt" {
                Engine::Dt
            } else {
                Engine::Loro
            };
            let prefill = prefill.parse::<usize>().unwrap_or_else(|_| {
                eprintln!("PREFILL must be a usize");
                std::process::exit(2);
            });
            bench(engine, case, prefill);
        }
        _ => {
            eprintln!(
                "usage: compare correctness | compare bench dt|loro ascii10k|unicode100k|churn1m|churn1m_bounded PREFILL"
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_baselines_use_predeclared_utf8_byte_budgets() {
        assert_eq!(baseline("ascii10k").len(), 10 * 1024);
        for (case, budget) in [("unicode100k", 100 * 1024), ("churn1m", 1024 * 1024)] {
            let text = baseline(case);
            assert!(text.len() <= budget && text.len() + 3 >= budget);
            assert!(text.contains('😀') && text.contains('日') && text.contains('\u{301}'));
        }
        assert_eq!(baseline("churn1m"), baseline("churn1m_bounded"));
    }

    #[test]
    fn original_large_replacement_retains_single_change_profile_failure() {
        let text = unicode_bytes(32 * 1024);
        let base = State::seed(Engine::Loro, &text).capture();
        let large = base.author(0, 1, &workload("churn1m", 2, &text));
        let meta = LoroDoc::decode_import_blob_meta(&large.bytes, true).unwrap();
        assert!(meta.change_num > 1);
        assert!(base.inspect(0, 1, &large).is_err());
        let bounded = base.author(0, 1, &workload("churn1m_bounded", 2, &text));
        assert_eq!(
            LoroDoc::decode_import_blob_meta(&bounded.bytes, true)
                .unwrap()
                .change_num,
            1
        );
        base.inspect(0, 1, &bounded).unwrap();
    }

    #[test]
    fn integrated_fixed_regressions() {
        fixed_regressions(Engine::Dt);
        fixed_regressions(Engine::Loro);
    }

    #[test]
    fn integrated_bounded_corpus() {
        correctness();
    }

    #[test]
    fn security_hooks_use_full_inspector() {
        let (base, bytes) = dt_screen_fixture();
        eprintln!("screen_fixture_bytes={}", bytes.len());
        assert!(
            bytes.len() <= 440,
            "leave room for bounded screening mutations"
        );
        let before = dt_save(&base);
        dt_screen_inspect(&base, &bytes).unwrap();
        let mut trailing = bytes;
        trailing.push(0);
        assert!(dt_screen_inspect(&base, &trailing).is_err());
        assert_eq!(before, dt_save(&base));
    }
}
