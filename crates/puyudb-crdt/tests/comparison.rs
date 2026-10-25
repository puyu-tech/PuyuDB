// SPDX-License-Identifier: Apache-2.0

//! Identical synthetic traces, not a universal CRDT ranking.
//! diamond-types uses a reviewed upstream revision without its optional LZ4.

mod common;

use automerge::ReadDoc;
use common::context;
use loro::{CommitOptions, ExportMode, LoroDoc};
use puyudb_crdt::experiment::{edit_at, seed, utf16_offset_to_byte};
use yrs::{
    Assoc, ClientID, Doc, GetString, IndexedSequence, OffsetKind, Options, ReadTxn, StateVector,
    StickyIndex, Text, Transact, Update,
    updates::{
        decoder::Decode,
        encoder::{Encode, Encoder, EncoderV1},
    },
};

#[test]
fn automerge_shared_observed_delete_trace_retains_each_historical_cut() {
    let seeded = seed(&[7; 32], "abcdef").unwrap();
    let base = seeded.document.get_heads();
    let a = edit_at(&seeded.document, &base, &context(1, 1), 2..2, "X").unwrap();
    let b = edit_at(&seeded.document, &base, &context(2, 1), 1..3, "").unwrap();
    for order in [[a.clone(), b.clone()], [b.clone(), a.clone()]] {
        let mut replica = seeded.document.clone();
        for change in order {
            replica.apply_changes([change.clone(), change]).unwrap();
        }
        let loaded = automerge::Automerge::load(&replica.save()).unwrap();
        assert_eq!(loaded.text(&seeded.text).unwrap(), "aXdef");
        for (heads, expected) in [
            (&base[..], "abcdef"),
            (&[a.hash()][..], "abXcdef"),
            (&[b.hash()][..], "adef"),
        ] {
            assert_eq!(loaded.text_at(&seeded.text, heads).unwrap(), expected);
        }
    }
}

fn yrs_document(client: u64, skip_gc: bool) -> Doc {
    Doc::with_options(Options {
        client_id: ClientID::new(client),
        offset_kind: OffsetKind::Bytes,
        skip_gc,
        ..Options::default()
    })
}

fn yrs_utf16_document(client: u64) -> Doc {
    Doc::with_options(Options {
        client_id: ClientID::new(client),
        offset_kind: OffsetKind::Utf16,
        skip_gc: true,
        ..Options::default()
    })
}

fn yrs_import(document: &Doc, bytes: &[u8]) {
    document
        .transact_mut()
        .apply_update(Update::decode_v1(bytes).unwrap())
        .unwrap();
}

#[test]
fn yrs_shared_observed_delete_trace_requires_retained_deleted_content() {
    let seed = yrs_document(1, true);
    let text = seed.get_or_insert_text("text");
    text.insert(&mut seed.transact_mut(), 0, "abcdef");
    let base_snapshot = seed.transact().snapshot();
    let base = seed
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let a = yrs_document(2, true);
    let b = yrs_document(3, true);
    yrs_import(&a, &base);
    yrs_import(&b, &base);
    a.get_or_insert_text("text")
        .insert(&mut a.transact_mut(), 2, "X");
    b.get_or_insert_text("text")
        .remove_range(&mut b.transact_mut(), 1, 2);
    let a_snapshot = a.transact().snapshot();
    let b_snapshot = b.transact().snapshot();
    let a_update = a
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let b_update = b
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    for (document, update) in [(&a, &b_update), (&b, &a_update)] {
        yrs_import(document, update);
        let version = document.transact().state_vector();
        yrs_import(document, update);
        assert_eq!(document.transact().state_vector(), version);
        assert_eq!(
            document
                .get_or_insert_text("text")
                .get_string(&document.transact()),
            "aXdef"
        );
        let full = document
            .transact()
            .encode_state_as_update_v1(&StateVector::default());
        let restored = yrs_document(4, true);
        yrs_import(&restored, &full);
        for (snapshot, expected) in [
            (&base_snapshot, "abcdef"),
            (&a_snapshot, "abXcdef"),
            (&b_snapshot, "adef"),
        ] {
            let mut encoder = EncoderV1::new();
            restored
                .transact()
                .encode_state_from_snapshot(snapshot, &mut encoder)
                .unwrap();
            let historical = yrs_document(5, true);
            yrs_import(&historical, &encoder.to_vec());
            assert_eq!(
                historical
                    .get_or_insert_text("text")
                    .get_string(&historical.transact()),
                expected
            );
        }
    }
}

fn loro_document(peer: u64) -> LoroDoc {
    let document = LoroDoc::new();
    document.set_peer_id(peer).unwrap();
    document.set_record_timestamp(false);
    document.set_change_merge_interval(-1);
    document
}

fn loro_commit(document: &LoroDoc, message: &str) {
    document.commit_with(CommitOptions::new().timestamp(0).commit_msg(message));
}

#[test]
fn loro_shared_observed_delete_trace_retains_each_historical_cut() {
    let seed = loro_document(1);
    seed.get_text("text").insert(0, "abcdef").unwrap();
    loro_commit(&seed, "fixture-seed");
    let base = seed.oplog_frontiers();
    let base_vv = seed.oplog_vv();
    let a = seed.fork_at(&base).unwrap();
    let b = seed.fork_at(&base).unwrap();
    for (document, peer) in [(&a, 2), (&b, 3)] {
        document.set_peer_id(peer).unwrap();
        document.set_record_timestamp(false);
        document.set_change_merge_interval(-1);
    }
    a.get_text("text").insert(2, "X").unwrap();
    loro_commit(&a, "fixture-a");
    b.get_text("text").delete(1, 2).unwrap();
    loro_commit(&b, "fixture-b");
    let a_cut = a.oplog_frontiers();
    let b_cut = b.oplog_frontiers();
    let a_update = a.export(ExportMode::updates(&base_vv)).unwrap();
    let b_update = b.export(ExportMode::updates(&base_vv)).unwrap();
    for (document, update) in [(&a, &b_update), (&b, &a_update)] {
        assert!(document.import(update).unwrap().pending.is_none());
        let version = document.oplog_vv();
        assert!(document.import(update).unwrap().pending.is_none());
        assert_eq!(document.oplog_vv(), version);
        assert_eq!(document.get_text("text").to_string(), "aXdef");
        let snapshot = document.export(ExportMode::Snapshot).unwrap();
        let restored = loro_document(4);
        assert!(restored.import(&snapshot).unwrap().pending.is_none());
        for (cut, expected) in [(&base, "abcdef"), (&a_cut, "abXcdef"), (&b_cut, "adef")] {
            restored.checkout(cut).unwrap();
            assert_eq!(restored.get_text("text").to_string(), expected);
        }
        restored.checkout_to_latest();
        assert_eq!(restored.get_text("text").to_string(), "aXdef");
    }
    assert_eq!(a.oplog_vv(), b.oplog_vv());
}

#[test]
fn yrs_byte_offset_anchor_profile_fails_the_astral_position_fixture() {
    let document = yrs_document(1, true);
    let text = document.get_or_insert_text("text");
    text.insert(&mut document.transact_mut(), 0, "A😀bc");
    let anchor = text
        .sticky_index(&document.transact(), 5, Assoc::After)
        .unwrap();
    text.insert(&mut document.transact_mut(), 0, "X");
    // Pin the failed candidate profile as evidence, not as the desired contract.
    // A future dependency fix should force this finding to be reconsidered.
    assert_eq!(anchor.get_offset(&document.transact()).unwrap().index, 0);
    assert_ne!(anchor.get_offset(&document.transact()).unwrap().index, 6);
}

#[test]
fn yrs_utf16_anchor_profile_maps_back_to_exact_utf8_positions_after_reload() {
    let document = yrs_utf16_document(1);
    let text = document.get_or_insert_text("text");
    text.insert(&mut document.transact_mut(), 0, "A😀bc");
    let snapshot = document.transact().snapshot();
    let anchor = text
        .sticky_index(&document.transact(), 3, Assoc::After)
        .unwrap();
    let anchor = StickyIndex::decode_v1(&anchor.encode_v1()).unwrap();
    text.insert(&mut document.transact_mut(), 0, "X");
    let native_position = anchor.get_offset(&document.transact()).unwrap().index;
    assert_eq!(native_position, 4);
    assert_eq!(
        utf16_offset_to_byte(
            &text.get_string(&document.transact()),
            native_position as usize
        )
        .unwrap(),
        6
    );
    text.remove_range(&mut document.transact_mut(), 2, 2);
    assert_eq!(text.get_string(&document.transact()), "XAbc");
    assert_eq!(anchor.get_offset(&document.transact()).unwrap().index, 2);
    let full = document
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let restored = yrs_utf16_document(2);
    yrs_import(&restored, &full);
    assert_eq!(anchor.get_offset(&restored.transact()).unwrap().index, 2);
    let mut encoder = EncoderV1::new();
    restored
        .transact()
        .encode_state_from_snapshot(&snapshot, &mut encoder)
        .unwrap();
    let historical = yrs_utf16_document(3);
    yrs_import(&historical, &encoder.to_vec());
    assert_eq!(
        historical
            .get_or_insert_text("text")
            .get_string(&historical.transact()),
        "A😀bc"
    );
    let native_position = anchor.get_offset(&historical.transact()).unwrap().index;
    assert_eq!(native_position, 3);
    assert_eq!(
        utf16_offset_to_byte("A😀bc", native_position as usize).unwrap(),
        5
    );
}

#[test]
fn yrs_default_gc_cannot_be_assumed_to_preserve_deleted_historical_content() {
    let document = yrs_document(1, false);
    let text = document.get_or_insert_text("text");
    text.insert(&mut document.transact_mut(), 0, "abcdef");
    let old = document.transact().snapshot();
    text.remove_range(&mut document.transact_mut(), 0, 6);
    let mut encoder = EncoderV1::new();
    assert!(
        document
            .transact()
            .encode_state_from_snapshot(&old, &mut encoder)
            .is_err()
    );
}

#[test]
fn loro_scalar_anchors_survive_reload_and_resolve_in_exact_historical_forks() {
    let document = loro_document(1);
    let text = document.get_text("text");
    text.insert(0, "A😀bc").unwrap();
    loro_commit(&document, "unicode-base");
    assert_eq!(
        (text.len_unicode(), text.len_utf8(), text.len_utf16()),
        (4, 7, 5)
    );
    let base = document.oplog_frontiers();
    let cursor = text.get_cursor(2, loro::cursor::Side::Right).unwrap();
    let cursor = loro::cursor::Cursor::decode(&cursor.encode()).unwrap();
    text.insert_utf8(0, "😀").unwrap();
    loro_commit(&document, "unicode-insert");
    assert_eq!(document.get_cursor_pos(&cursor).unwrap().current.pos, 3);
    text.delete_utf8(5, 4).unwrap();
    loro_commit(&document, "unicode-delete");
    assert_eq!(text.to_string(), "😀Abc");
    assert_eq!(document.get_cursor_pos(&cursor).unwrap().current.pos, 2);
    let restored = loro_document(2);
    restored
        .import(&document.export(ExportMode::Snapshot).unwrap())
        .unwrap();
    assert_eq!(restored.get_cursor_pos(&cursor).unwrap().current.pos, 2);
    // Restrict both native state and native oplog; checkout alone is insufficient
    // evidence for every deleted-anchor fallback in this pinned implementation.
    let historical = restored.fork_at(&base).unwrap();
    assert_eq!(historical.get_text("text").to_string(), "A😀bc");
    assert_eq!(historical.get_cursor_pos(&cursor).unwrap().current.pos, 2);
}

#[test]
fn challenger_seed_updates_repeat_in_fresh_documents_with_fixed_native_identities() {
    let mut yrs_bytes = Vec::new();
    let mut loro_bytes = Vec::new();
    for _ in 0..3 {
        let y = yrs_document(1, true);
        y.get_or_insert_text("text")
            .insert(&mut y.transact_mut(), 0, "A😀e\u{301}");
        yrs_bytes.push(
            y.transact()
                .encode_state_as_update_v1(&StateVector::default()),
        );
        let l = loro_document(1);
        l.get_text("text").insert(0, "A😀e\u{301}").unwrap();
        loro_commit(&l, "fixed-seed");
        loro_bytes.push(l.export(ExportMode::all_updates()).unwrap());
    }
    assert!(yrs_bytes.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(loro_bytes.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn pinned_diamond_types_traces_retain_portable_historical_frontiers() {
    use diamond_types::list::{ListOpLog, encoding::EncodeOptions};

    for (baseline, inserted, delete_end, merged, a_text, b_text) in [
        ("abcdef", "X", 3, "aXdef", "abXcdef", "adef"),
        (
            "A😀e\u{301}\r\n",
            "日本",
            2,
            "A日本e\u{301}\r\n",
            "A😀日本e\u{301}\r\n",
            "Ae\u{301}\r\n",
        ),
    ] {
        let options = EncodeOptions::full().compress_content(false);
        let patch_options = EncodeOptions::patch().compress_content(false);
        let mut original = ListOpLog::new();
        let actor = original.get_or_create_agent_id("fixture-seed");
        let mut view = original.checkout_tip();
        view.insert(&mut original, actor, 0, baseline);
        let base_remote = original
            .cg
            .agent_assignment
            .local_to_remote_frontier_owned(view.local_frontier_ref());
        let saved = original.encode(&options);
        let mut a = ListOpLog::load_from(&saved).unwrap();
        let mut b = ListOpLog::load_from(&saved).unwrap();
        let mut a_view = a.checkout_tip();
        let mut b_view = b.checkout_tip();
        let a_base = a_view.local_frontier_ref().to_owned();
        let b_base = b_view.local_frontier_ref().to_owned();
        let actor_a = a.get_or_create_agent_id("fixture-a");
        let actor_b = b.get_or_create_agent_id("fixture-b");
        a_view.insert(&mut a, actor_a, 2, inserted);
        b_view.delete(&mut b, actor_b, 1..delete_end);
        let a_remote =
            a.cg.agent_assignment
                .local_to_remote_frontier_owned(a_view.local_frontier_ref());
        let b_remote =
            b.cg.agent_assignment
                .local_to_remote_frontier_owned(b_view.local_frontier_ref());
        let a_update = a.encode_from(&patch_options, a_base.as_ref());
        let b_update = b.encode_from(&patch_options, b_base.as_ref());
        for (document, update) in [(&mut a, &b_update), (&mut b, &a_update)] {
            document.decode_and_add(update).unwrap();
            let before = document.len();
            document.decode_and_add(update).unwrap();
            assert_eq!(document.len(), before);
            assert_eq!(document.checkout_tip().content().to_string(), merged);
            let loaded = ListOpLog::load_from(&document.encode(&options)).unwrap();
            for (cut, expected) in [
                (&base_remote, baseline),
                (&a_remote, a_text),
                (&b_remote, b_text),
            ] {
                let local = loaded
                    .cg
                    .agent_assignment
                    .try_remote_to_local_frontier(cut.iter())
                    .unwrap();
                assert_eq!(
                    loaded.checkout(local.as_ref()).content().to_string(),
                    expected
                );
            }
        }
    }
}
