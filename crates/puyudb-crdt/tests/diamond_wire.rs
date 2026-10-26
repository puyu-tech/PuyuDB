// SPDX-License-Identifier: Apache-2.0

//! Native-wire counterexamples at diamond-types 89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922.
//! These characterize public native APIs, not a PuyuDB admission policy or engine choice.
//! Defaults/LZ4 are disabled in Cargo.toml; every export also disables compression.
//! No fuzzing, private APIs, copied upstream implementation, or substitute decoder.

#[path = "support/diamond_bytes.rs"]
mod bytes;

use bytes::{crc32c, seal};
use diamond_types::{
    causalgraph::agent_assignment::remote_ids::RemoteVersion,
    list::{ListOpLog, encoding::EncodeOptions, operation::TextOperation},
};
use std::{cmp::Ordering, panic::catch_unwind};

fn options() -> EncodeOptions<'static> {
    // encode() still exports all history; patches omit redundant base text in updates.
    EncodeOptions::patch().compress_content(false)
}

fn inserted(agent: &str, content: &str) -> ListOpLog {
    let mut log = ListOpLog::new();
    let agent = log.get_or_create_agent_id(agent);
    if !content.is_empty() {
        log.add_insert_at(agent, &[], 0, content);
    }
    log
}

fn text(log: &ListOpLog) -> String {
    log.checkout_tip().content().to_string()
}

// Independently assembled, empty protocol-0 list envelope. All tags and lengths
// fit one byte. Layout provenance: pinned src/list/encoding/mod.rs:27-64 and
// encode_oplog.rs:699-764, not the older/drifted BINARY.md. There are no operation
// bodies to parse or invent. Tests first compare this with the public encoder.
fn empty_prefix() -> Vec<u8> {
    let mut bytes = b"DMNDTYPS\0".to_vec(); // Magic and protocol version.
    bytes.extend([1, 2, 3, 0]); // FileInfo containing empty AgentNames.
    bytes.extend([10, 0]); // Empty/root StartBranch.
    bytes.extend([20, 6, 21, 0, 22, 0, 23, 0]); // Patches: empty IDs, ops, parents.
    bytes
}

#[test]
fn user_data_is_encoded_but_not_retained_by_native_snapshot_reload() {
    let metadata = b"outer epoch descriptor, not native history";
    for content in ["", "A"] {
        let source = inserted("writer", content);
        let plain = source.encode(&options());
        let original = source.encode(&options().user_data(metadata));
        assert_ne!(original, plain);
        assert!(original.windows(metadata.len()).any(|w| w == metadata));

        let decoded = ListOpLog::load_from(&original).unwrap();
        assert_eq!(text(&decoded), content);
        assert_eq!(decoded.encode(&options()), plain);
        // Reattaching the separately retained metadata works; the oplog alone
        // does not archive the original envelope, even for an empty seed.
        assert_eq!(decoded.encode(&options().user_data(metadata)), original);
    }
}

#[test]
fn trailing_bytes_and_a_second_valid_artifact_are_not_consumed() {
    let original = inserted("first", "A").encode(&options());
    let second = inserted("second", "B").encode(&options());
    for trailing in [b"unconsumed".as_slice(), second.as_slice()] {
        let mut extended = original.clone();
        extended.extend_from_slice(trailing);
        let decoded = ListOpLog::load_from(&extended).unwrap();
        assert_eq!(text(&decoded), "A");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded.num_agents(), 1);
        assert_eq!(decoded.encode(&options()), original);

        let mut receiver = ListOpLog::new();
        receiver.decode_and_add(&extended).unwrap();
        assert_eq!(receiver.encode(&options()), original);
    }
}

#[test]
fn valid_crc_unknown_chunk_is_skipped_and_lost_on_reencoding() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    let plain = ListOpLog::new().encode(&options());
    assert_eq!(seal(empty_prefix()), plain);

    let mut prefix = empty_prefix();
    // Unassigned tag 99, one-byte payload, between FileInfo and StartBranch.
    // No existing chunk length or operation data is modified.
    prefix.splice(13..13, [99, 1, 42]);
    let original = seal(prefix);
    let decoded = ListOpLog::load_from(&original).unwrap();
    assert!(decoded.is_empty());
    assert_eq!(decoded.encode(&options()), plain);
    assert_ne!(original, plain);

    // This placement really is covered by native CRC verification: changing
    // only the unknown payload, without resealing, produces a returned error.
    let mut changed = original;
    changed[15] ^= 1;
    let error = ListOpLog::load_from(&changed).err().unwrap();
    assert_eq!(format!("{error:?}"), "ChecksumFailed");
}

#[test]
fn unknown_chunk_immediately_before_crc_prevents_even_checksum_validation() {
    let mut prefix = empty_prefix();
    prefix.extend([99, 1, 42]);
    let valid = seal(prefix);
    let mut invalid_crc = valid.clone();
    *invalid_crc.last_mut().unwrap() ^= 1;

    // The optional CRC probe sees tag 99 and stops instead of skipping it.
    // Both artifacts load, including the one with an intentionally wrong CRC.
    for bytes in [&valid, &invalid_crc] {
        let decoded = ListOpLog::load_from(bytes).unwrap();
        assert!(decoded.is_empty());
        assert_eq!(decoded.encode(&options()), seal(empty_prefix()));
    }
}

#[test]
fn fully_known_conflicting_ids_are_ignored_just_like_exact_duplicates() {
    let a = inserted("writer", "A").encode(&options());
    let b = inserted("writer", "B").encode(&options());
    assert_ne!(a, b);
    // Both independently generated artifacts claim exactly (writer, 0).
    for (first, conflicting, expected) in [(&a, &b, "A"), (&b, &a, "B")] {
        let mut receiver = ListOpLog::load_from(first).unwrap();
        let before = receiver.encode(&options());
        let frontier = receiver.decode_and_add(first).unwrap();
        assert_eq!(frontier.as_ref(), receiver.local_frontier_ref());
        assert_eq!(receiver.encode(&options()), before);

        let frontier = receiver.decode_and_add(conflicting).unwrap();
        assert_eq!(frontier.as_ref(), receiver.local_frontier_ref());
        assert_eq!(receiver.len(), 1);
        assert_eq!(text(&receiver), expected);
        assert_eq!(receiver.encode(&options()), before);
    }
    // Success here is identity deduplication, not conflict/content validation.
}

#[test]
fn overlapping_conflicting_prefix_is_discarded_while_new_suffix_is_imported() {
    let known = inserted("writer", "A").encode(&options());
    for incoming_text in ["AX", "BX"] {
        let incoming = inserted("writer", incoming_text).encode(&options());
        assert_eq!(
            text(&ListOpLog::load_from(&incoming).unwrap()),
            incoming_text
        );
        let mut receiver = ListOpLog::load_from(&known).unwrap();
        receiver.decode_and_add(&incoming).unwrap();
        assert_eq!(receiver.len(), 2);
        assert_eq!(text(&receiver), "AX");
        assert_eq!(
            receiver.encode(&options()),
            inserted("writer", "AX").encode(&options())
        );
        let before = receiver.encode(&options());
        receiver.decode_and_add(&incoming).unwrap();
        assert_eq!(receiver.encode(&options()), before);
    }
    // The new suffix alone looks legitimate; inspecting only newly applied
    // operations cannot detect that the original known prefix claimed "B".
}

#[test]
fn encode_from_base_includes_unrelated_newer_tip_history() {
    let mut source = inserted("seed", "S");
    let base = source.local_frontier_ref().to_vec();
    let seed = source.encode(&options());
    let unrelated = source.get_or_create_agent_id("unrelated");
    let unrelated_tip = source.add_insert_at(unrelated, &base, 1, "U");
    let author = source.get_or_create_agent_id("author");
    let authored_tip = source.add_insert_at(author, &base, 1, "A");
    assert_eq!(source.checkout(&[authored_tip]).content().to_string(), "SA");
    assert_eq!(
        source.checkout(&[unrelated_tip]).content().to_string(),
        "SU"
    );

    let update = source.encode_from(&options(), &base);
    let mut receiver = ListOpLog::load_from(&seed).unwrap();
    receiver.decode_and_add(&update).unwrap();
    assert_eq!(receiver.len(), 3); // Seed plus both branches, not just the author.
    for name in ["author", "unrelated"] {
        assert!(
            receiver
                .cg
                .agent_assignment
                .try_remote_to_local_version(RemoteVersion(name, 0))
                .is_ok()
        );
    }
    assert_eq!(text(&receiver), text(&source));
    assert_eq!(receiver.local_frontier_ref().len(), 2);
    // encode_from has a lower frontier but no upper authored-batch boundary.
}

#[test]
fn adjacent_author_batches_coalesce_and_snapshot_cannot_recover_their_envelopes() {
    let mut source = inserted("writer", "A");
    let first = source.encode(&options());
    let base = source.local_frontier_ref().to_vec();
    let writer = source.get_or_create_agent_id("writer");
    source.add_operations_at(writer, &base, &[TextOperation::new_insert(1, "B")]);
    let second = source.encode_from(&options(), &base);
    let snapshot = source.encode(&options());
    assert_ne!(snapshot, first);
    assert_ne!(snapshot, second);

    // One public insertion and two separately exported authoring calls collapse
    // to the same snapshot. Public iterators expose one run, not two batches.
    let single_call = inserted("writer", "AB");
    assert_eq!(snapshot, single_call.encode(&options()));
    let entries: Vec<_> = source.iter_full().collect();
    assert_eq!(entries.len(), 1);
    let (op, history, ids) = &entries[0];
    assert_eq!(op.content_as_str(), Some("AB"));
    assert_eq!(history.span, (0..2).into());
    assert!(history.parents.is_empty());
    assert_eq!(ids.0, "writer");
    assert_eq!(ids.1, (0..2).into());

    // Original envelopes remain replayable if retained separately. This is not
    // a native batch ledger: producer/dependency associations still live outside.
    let mut replay = ListOpLog::new();
    for artifact in [&first, &second] {
        replay.decode_and_add(artifact).unwrap();
    }
    let reloaded = ListOpLog::load_from(&replay.encode(&options())).unwrap();
    assert_eq!(reloaded.iter_full().count(), 1);
    assert_eq!(reloaded.encode(&options()), snapshot);
    assert_eq!(text(&reloaded), "AB");
}

#[test]
fn missing_predecessor_returns_error_rolls_back_and_does_not_buffer_update() {
    let mut source = inserted("predecessor", "A");
    let predecessor = source.encode(&options());
    let base = source.local_frontier_ref().to_vec();
    let writer = source.get_or_create_agent_id("suffix");
    source.add_insert_at(writer, &base, 1, "B");
    let update = source.encode_from(&options(), &base);
    let mut receiver = ListOpLog::new();
    let before = receiver.encode(&options());

    let error = receiver.decode_and_add(&update).unwrap_err();
    // The native error enum is not publicly nameable; compare its Debug value.
    assert_eq!(format!("{error:?}"), "BaseVersionUnknown");
    assert!(receiver.is_empty());
    assert!(receiver.local_frontier_ref().is_empty());
    assert_eq!(receiver.num_agents(), 0); // Even newly interned agents roll back.
    assert_eq!(receiver.iter_full().count(), 0);
    assert_eq!(text(&receiver), "");
    assert_eq!(receiver.encode(&options()), before);

    receiver.decode_and_add(&predecessor).unwrap();
    assert_eq!(text(&receiver), "A"); // No automatic replay of the rejected suffix.
    assert_eq!(receiver.len(), 1);
    assert_eq!(receiver.num_agents(), 1);
    receiver.decode_and_add(&update).unwrap(); // Explicit retry is required.
    assert_eq!(text(&receiver), "AB");
    assert_eq!(receiver.len(), 2);
}

#[test]
fn late_checksum_error_rolls_back_native_operations_history_agents_and_content() {
    let mut source = inserted("seed", "A");
    let seed = source.encode(&options());
    let base = source.local_frontier_ref().to_vec();
    let writer = source.get_or_create_agent_id("new-writer");
    source.add_insert_at(writer, &base, 1, "B");
    let valid = source.encode_from(&options(), &base);
    let mut invalid_crc = valid.clone();
    *invalid_crc.last_mut().unwrap() ^= 1; // Only corrupt the final CRC payload.
    let mut receiver = ListOpLog::load_from(&seed).unwrap();
    let before_ops: Vec<_> = receiver.iter_ops().collect();
    let before_history: Vec<_> = receiver.iter_history().collect();
    let before_frontier = receiver.local_frontier_ref().to_vec();

    let error = receiver.decode_and_add(&invalid_crc).unwrap_err();
    assert_eq!(format!("{error:?}"), "ChecksumFailed");
    assert_eq!(receiver.len(), 1);
    assert_eq!(receiver.num_agents(), 1);
    assert_eq!(receiver.local_frontier_ref(), before_frontier);
    assert_eq!(receiver.iter_ops().collect::<Vec<_>>(), before_ops);
    assert_eq!(receiver.iter_history().collect::<Vec<_>>(), before_history);
    assert_eq!(text(&receiver), "A");
    assert_eq!(receiver.encode(&options()), seed);

    receiver.decode_and_add(&valid).unwrap();
    assert_eq!(text(&receiver), "AB");
    assert_eq!(receiver.encode(&options()), source.encode(&options()));
    // Evidence for returned-error rollback only, not panic safety/resource bounds.
}

#[test]
fn root_cut_wrapper_contains_an_existing_operation_but_public_graph_query_does_not() {
    let log = inserted("writer", "A");
    let target = 0; // A real existing LV; no invalid target or private API call.
    assert_eq!(log.checkout(&[]).content().to_string(), "");
    assert!(log.version_contains_lv(&[], target));
    assert!(!log.cg.graph.frontier_contains_frontier(&[], &[target]));
    assert_eq!(
        log.cg.graph.frontier_cmp(&[], &[target]),
        Some(Ordering::Less)
    );
    // Non-root control: both APIs agree when the operation actually is present.
    assert!(log.version_contains_lv(&[target], target));
    assert!(
        log.cg
            .graph
            .frontier_contains_frontier(&[target], &[target])
    );
}

#[test]
fn native_encoder_can_omit_insert_content_that_decoder_accepts_but_checkout_panics() {
    let source = inserted("writer", "A");
    let artifact = source.encode(&options().store_inserted_content(false));
    let decoded = ListOpLog::load_from(&artifact).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(
        decoded.iter_ops().collect::<Vec<_>>()[0].content_as_str(),
        None
    );
    // Deliberately isolate this one natively generated, contentless case.
    // Checkout only reads the log; discard it afterward. A caught Rust panic
    // here is neither a returned decode error nor evidence of undefined behavior.
    // Pinned panic site: src/list/merge.rs:136, missing insert content unwrap.
    let panic = catch_unwind(|| text(&decoded))
        .expect_err("pinned native checkout requires insert content");
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
    assert_eq!(message, Some("called `Option::unwrap()` on a `None` value"));
}
