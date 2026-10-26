// SPDX-License-Identifier: Apache-2.0

//! Candidate-fit experiments, not a production Loro binding or engine selection.

#[path = "support/loro_fit.rs"]
mod fit;

use fit::{EpochPeers, FitError, FixtureWriter};

fn writer(key: u8, branch: u8, epoch: u8) -> FixtureWriter {
    FixtureWriter {
        public_key: [key; 32],
        branch: [branch; 16],
        writer_epoch: [epoch; 16],
    }
}

#[test]
fn full_writer_ownership_is_not_silently_truncated_to_a_native_peer() {
    let mut peers = EpochPeers::new([7; 32]);
    let original = peers.bind(writer(1, 1, 1), 2).unwrap();
    assert_eq!(peers.bind(writer(1, 1, 1), 2).unwrap(), original);
    assert!(matches!(
        peers.bind(writer(2, 1, 1), 2),
        Err(FitError::PeerCollision)
    ));
    assert!(matches!(
        peers.bind(writer(1, 1, 1), 3),
        Err(FitError::PeerReassignment)
    ));
    assert!(matches!(
        peers.bind(writer(1, 2, 1), 2),
        Err(FitError::PeerCollision)
    ));
    assert!(matches!(
        peers.bind(writer(1, 1, 2), 2),
        Err(FitError::PeerCollision)
    ));
    assert!(matches!(
        peers.bind(writer(2, 1, 1), 1),
        Err(FitError::ReservedSeedPeer)
    ));
    assert!(peers.bind(writer(2, 1, 1), 3).is_ok());
    let another_epoch = EpochPeers::new([8; 32]).bind(writer(1, 1, 1), 2).unwrap();
    assert_ne!(original, another_epoch);
}

#[test]
fn exact_cut_unicode_edits_are_inspected_without_mutating_the_trusted_base() {
    let base = fit::FixtureBase::seed([7; 32], "A😀bc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let update = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 5..6,
            inserted: "日本",
        }],
    )
    .unwrap();
    let checked = fit::inspect(&base, &binding, 1, &update).unwrap();
    assert_eq!(checked.get_text("text").to_string(), "A😀日本c");
    assert_eq!(
        base.document().unwrap().get_text("text").to_string(),
        "A😀bc"
    );
    let parsed = checked
        .export_json_updates_without_peer_compression(
            &base.document().unwrap().oplog_vv(),
            &checked.oplog_vv(),
        )
        .unwrap();
    assert_eq!(parsed.changes.len(), 1);
    assert!(parsed.changes[0].ops.len() >= 2);
}

fn raw_fixture(
    base: &fit::FixtureBase,
    peer: u64,
    message: &str,
    timestamp: i64,
    edit: impl FnOnce(&loro::LoroDoc),
) -> fit::CapturedUpdate {
    let source = base.document().unwrap();
    source.set_peer_id(peer).unwrap();
    let from = source.oplog_vv();
    edit(&source);
    source.commit_with(
        loro::CommitOptions::new()
            .timestamp(timestamp)
            .commit_msg(message),
    );
    fit::CapturedUpdate::new(source.export(loro::ExportMode::updates(&from)).unwrap())
}

#[test]
fn hidden_nontext_and_foreign_containers_are_rejected_even_when_text_is_unchanged() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    for kind in [
        "map",
        "same-name-map",
        "list",
        "tree",
        "foreign-text",
        "nested-text",
    ] {
        let update = raw_fixture(&base, 2, &fit::message(&binding, 1), 0, |doc| {
            match kind {
                "map" | "same-name-map" => {
                    let map = doc.get_map(if kind == "map" { "foreign" } else { "text" });
                    map.insert("temporary", "hidden").unwrap();
                    map.delete("temporary").unwrap();
                }
                "list" => {
                    let list = doc.get_list("foreign");
                    list.insert(0, "hidden").unwrap();
                    list.delete(0, 1).unwrap();
                }
                "tree" => {
                    doc.get_tree("foreign").create(None).unwrap();
                }
                "foreign-text" => {
                    let text = doc.get_text("foreign");
                    text.insert(0, "hidden").unwrap();
                    text.delete(0, 6).unwrap();
                }
                "nested-text" => {
                    let text = doc
                        .get_map("parent")
                        .insert_container("child", loro::LoroText::new())
                        .unwrap();
                    text.insert(0, "hidden").unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(doc.get_text("text").to_string(), "abc");
        });
        assert!(
            matches!(
                fit::inspect(&base, &binding, 1, &update),
                Err(FitError::Rejected(
                    "operation targets a foreign fixture container"
                ))
            ),
            "{kind}"
        );
        assert_eq!(base.document().unwrap().get_text("text").to_string(), "abc");
    }
}

#[test]
fn rich_text_marks_are_not_mistaken_for_plain_text_operations() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let update = raw_fixture(&base, 2, &fit::message(&binding, 1), 0, |doc| {
        let mut styles = loro::StyleConfigMap::new();
        styles.insert(
            "emphasis".into(),
            loro::StyleConfig {
                expand: loro::ExpandType::After,
            },
        );
        doc.config_text_style(styles);
        doc.get_text("text").mark(0..2, "emphasis", true).unwrap();
        assert_eq!(doc.get_text("text").to_string(), "abc");
    });
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &update),
        Err(FitError::Rejected(
            "unsupported operation in plain-text fixture"
        ))
    ));
}

#[test]
fn fixture_context_checks_full_writer_epoch_sequence_and_timestamp_not_authentication() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let valid = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 1..1,
            inserted: "X",
        }],
    )
    .unwrap();
    assert!(matches!(
        fit::inspect(
            &fit::FixtureBase::seed([8; 32], "abc").unwrap(),
            &binding,
            1,
            &valid
        ),
        Err(FitError::Rejected(
            "fixture epoch or outer sequence mismatch"
        ))
    ));
    for expected_sequence in [0, 2] {
        assert!(fit::inspect(&base, &binding, expected_sequence, &valid).is_err());
    }
    for wrong_writer in [writer(2, 1, 1), writer(1, 2, 1), writer(1, 1, 2)] {
        let wrong_binding = EpochPeers::new([7; 32]).bind(wrong_writer, 2).unwrap();
        assert!(matches!(
            fit::inspect(&base, &wrong_binding, 1, &valid),
            Err(FitError::Rejected(
                "native writer, metadata or exact-cut mismatch"
            ))
        ));
    }
    for (peer, timestamp) in [(3, 0), (2, 1)] {
        let invalid = raw_fixture(&base, peer, &fit::message(&binding, 1), timestamp, |doc| {
            doc.get_text("text").insert(0, "X").unwrap();
        });
        assert!(matches!(
            fit::inspect(&base, &binding, 1, &invalid),
            Err(FitError::Rejected(
                "native span, timestamps or intent dependencies mismatch"
            ))
        ));
    }
}

#[test]
fn whole_blob_reconstruction_rejects_bytes_omitted_by_native_checksum_inspection() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let valid = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 1..2,
            inserted: "X",
        }],
    )
    .unwrap();
    let mut corrupted = valid.clone();
    corrupted.bytes[4] ^= 1;
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &corrupted),
        Err(FitError::Rejected("fixture artifact digest mismatch"))
    ));
    let altered = fit::CapturedUpdate::new(corrupted.bytes);
    // Fast native modes check only the last4 bytes of the16-byte checksum area.
    loro::LoroDoc::decode_import_blob_meta(&altered.bytes, true).unwrap();
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &altered),
        Err(FitError::Rejected(
            "fixture bytes differ from independently reconstructed native export"
        ))
    ));
    let mut checksum = valid.bytes;
    checksum[16] ^= 1;
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &fit::CapturedUpdate::new(checksum)),
        Err(FitError::Native(_))
    ));
}

#[test]
fn snapshot_overlap_empty_and_oversized_inputs_cannot_be_reauthored_as_one_new_change() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let snapshot = fit::CapturedUpdate::new(
        base.document()
            .unwrap()
            .export(loro::ExportMode::Snapshot)
            .unwrap(),
    );
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &snapshot),
        Err(FitError::Rejected(
            "expected exactly one native update change"
        ))
    ));
    let empty = fit::CapturedUpdate::new(
        base.document()
            .unwrap()
            .export(loro::ExportMode::updates_owned(
                base.document().unwrap().oplog_vv(),
            ))
            .unwrap(),
    );
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &empty),
        Err(FitError::Rejected(
            "expected exactly one native update change"
        ))
    ));
    let valid = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 1..1,
            inserted: "X",
        }],
    )
    .unwrap();
    let first = fit::inspect(&base, &binding, 1, &valid).unwrap();
    let newer = fit::FixtureBase::capture([7; 32], &first, first.oplog_frontiers()).unwrap();
    assert!(matches!(
        fit::inspect(&newer, &binding, 2, &valid),
        Err(FitError::Rejected(
            "native span, timestamps or intent dependencies mismatch"
        ))
    ));
    let huge = fit::CapturedUpdate::new(vec![0; 8 * 1024 * 1024 + 1]);
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &huge),
        Err(FitError::Rejected(
            "encoded fixture exceeds experimental limit"
        ))
    ));
}

#[test]
fn staged_primitive_byte_edits_share_one_bound_native_change() {
    let base = fit::FixtureBase::seed([7; 32], "abcdef").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let update = fit::author(
        &base,
        &binding,
        1,
        &[
            fit::ByteEdit {
                range: 1..3,
                inserted: "XYZ",
            },
            fit::ByteEdit {
                range: 4..5,
                inserted: "",
            },
            fit::ByteEdit {
                range: 6..6,
                inserted: "😀",
            },
        ],
    )
    .unwrap();
    let checked = fit::inspect(&base, &binding, 1, &update).unwrap();
    assert_eq!(checked.get_text("text").to_string(), "aXYZef😀");
    assert_eq!(
        loro::LoroDoc::decode_import_blob_meta(&update.bytes, true)
            .unwrap()
            .change_num,
        1
    );
}

#[test]
fn backward_adjacent_deletions_preserve_native_direction_and_target_references() {
    let base = fit::FixtureBase::seed([7; 32], "a😀bc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let update = fit::author(
        &base,
        &binding,
        1,
        &[
            fit::ByteEdit {
                range: 5..6,
                inserted: "",
            },
            fit::ByteEdit {
                range: 1..5,
                inserted: "",
            },
        ],
    )
    .unwrap();
    let checked = fit::inspect(&base, &binding, 1, &update).unwrap();
    assert_eq!(checked.get_text("text").to_string(), "ac");
    let decoded = checked
        .export_json_updates_without_peer_compression(
            &base.document().unwrap().oplog_vv(),
            &checked.oplog_vv(),
        )
        .unwrap();
    assert!(matches!(
        decoded.changes[0].ops[0].content,
        loro::JsonOpContent::Text(loro::JsonTextOp::Delete {
            pos: 2,
            len: -2,
            ..
        })
    ));
}

#[test]
fn capturing_a_trusted_base_rejects_pending_edits_without_committing_them() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let doc = base.document().unwrap();
    doc.set_peer_id(2).unwrap();
    let cut = doc.oplog_frontiers();
    let before = doc.oplog_vv();
    doc.get_text("text").insert(1, "X").unwrap();
    let pending_version = doc.oplog_vv();
    assert_eq!(doc.get_pending_txn_len(), 1);
    assert!(matches!(
        fit::FixtureBase::capture([7; 32], &doc, cut),
        Err(FitError::Rejected(
            "trusted fixture must have no pending local operations"
        ))
    ));
    assert_eq!(doc.get_pending_txn_len(), 1);
    assert_eq!(doc.oplog_vv(), pending_version);
    doc.commit_with(
        loro::CommitOptions::new()
            .timestamp(0)
            .commit_msg("explicit-after-rejected-capture"),
    );
    let changes = doc
        .export_json_updates_without_peer_compression(&before, &doc.oplog_vv())
        .unwrap();
    assert_eq!(changes.changes.len(), 1);
    assert_eq!(
        changes.changes[0].msg.as_deref(),
        Some("explicit-after-rejected-capture")
    );
    assert_eq!(doc.get_text("text").to_string(), "aXbc");
}

#[test]
fn implicit_empty_native_seed_needs_external_epoch_binding_not_a_hidden_dummy_op() {
    let first = fit::FixtureBase::seed([7; 32], "").unwrap();
    let second = fit::FixtureBase::seed([8; 32], "").unwrap();
    let a = first.document().unwrap();
    let b = second.document().unwrap();
    assert_eq!(a.len_changes(), 0);
    assert_eq!(b.len_changes(), 0);
    assert_eq!(
        a.export(loro::ExportMode::Snapshot).unwrap(),
        b.export(loro::ExportMode::Snapshot).unwrap()
    );
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let update = fit::author(
        &first,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 0..0,
            inserted: "😀",
        }],
    )
    .unwrap();
    assert_eq!(
        fit::inspect(&first, &binding, 1, &update)
            .unwrap()
            .get_text("text")
            .to_string(),
        "😀"
    );
    assert!(matches!(
        fit::inspect(&second, &binding, 1, &update),
        Err(FitError::Rejected(
            "fixture epoch or outer sequence mismatch"
        ))
    ));
    // Matching outer epoch/peer must still reject the other epoch's native
    // message, even though both empty native seed snapshots are identical.
    let second_binding = EpochPeers::new([8; 32]).bind(writer(1, 1, 1), 2).unwrap();
    assert!(matches!(
        fit::inspect(&second, &second_binding, 1, &update),
        Err(FitError::Rejected(
            "native writer, metadata or exact-cut mismatch"
        ))
    ));
}

#[test]
fn deletion_references_are_regenerated_not_trusted_from_decoded_history() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let valid = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 1..2,
            inserted: "",
        }],
    )
    .unwrap();
    let accepted = fit::inspect(&base, &binding, 1, &valid).unwrap();
    let from = base.document().unwrap().oplog_vv();
    let mut schema = accepted
        .export_json_updates_without_peer_compression(&from, &accepted.oplog_vv())
        .unwrap();
    for op in &mut schema.changes[0].ops {
        if let loro::JsonOpContent::Text(loro::JsonTextOp::Delete { start_id, .. }) =
            &mut op.content
        {
            assert_eq!(*start_id, loro::ID::new(1, 1));
            *start_id = loro::ID::new(1, 0); // Known, visible, but not the declared target.
        }
    }
    let forged = base.document().unwrap();
    forged.import_json_updates(schema).unwrap();
    let update = fit::CapturedUpdate::new(forged.export(loro::ExportMode::updates(&from)).unwrap());
    assert!(matches!(
        fit::inspect(&base, &binding, 1, &update),
        Err(FitError::Rejected(
            "fixture bytes differ from independently reconstructed native export"
        ))
    ));
}

#[test]
fn current_unrelated_history_cannot_broaden_the_declared_intent_cut() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let initial = base.document().unwrap().oplog_frontiers();
    let mut peers = EpochPeers::new([7; 32]);
    let a = peers.bind(writer(1, 1, 1), 2).unwrap();
    let b = peers.bind(writer(2, 2, 1), 3).unwrap();
    let other = fit::author(
        &base,
        &b,
        1,
        &[fit::ByteEdit {
            range: 1..1,
            inserted: "Y",
        }],
    )
    .unwrap();
    let current = fit::inspect(&base, &b, 1, &other).unwrap();
    assert_eq!(current.get_text("text").to_string(), "aYbc");
    let declared = fit::FixtureBase::capture([7; 32], &current, initial).unwrap();
    let isolated = fit::author(
        &declared,
        &a,
        1,
        &[fit::ByteEdit {
            range: 1..2,
            inserted: "X",
        }],
    )
    .unwrap();
    assert_eq!(
        fit::inspect(&declared, &a, 1, &isolated)
            .unwrap()
            .get_text("text")
            .to_string(),
        "aXc"
    );
    current.set_peer_id(2).unwrap();
    let from = current.oplog_vv();
    current.get_text("text").delete(1, 1).unwrap();
    current.get_text("text").insert(1, "X").unwrap();
    current.commit_with(
        loro::CommitOptions::new()
            .timestamp(0)
            .commit_msg(&fit::message(&a, 1)),
    );
    let broader =
        fit::CapturedUpdate::new(current.export(loro::ExportMode::updates(&from)).unwrap());
    assert!(matches!(
        fit::inspect(&declared, &a, 1, &broader),
        Err(FitError::Rejected(
            "native span, timestamps or intent dependencies mismatch"
        ))
    ));
}

#[test]
fn missing_predecessor_can_be_pending_and_is_not_preserved_by_snapshot_alone() {
    let base = fit::FixtureBase::seed([7; 32], "a").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let first = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 1..1,
            inserted: "b",
        }],
    )
    .unwrap();
    let doc = fit::inspect(&base, &binding, 1, &first).unwrap();
    let later = fit::FixtureBase::capture([7; 32], &doc, doc.oplog_frontiers()).unwrap();
    let second = fit::author(
        &later,
        &binding,
        2,
        &[fit::ByteEdit {
            range: 2..2,
            inserted: "c",
        }],
    )
    .unwrap();
    let receiver = base.document().unwrap();
    assert!(receiver.import(&second.bytes).unwrap().pending.is_some());
    assert_eq!(receiver.get_text("text").to_string(), "a");
    let snapshot = receiver.export(loro::ExportMode::Snapshot).unwrap();
    let sidecar = receiver.export_pending_updates().unwrap().unwrap();
    let without = fit::document(8).unwrap();
    without.import(&snapshot).unwrap();
    without.import(&first.bytes).unwrap();
    assert_eq!(without.get_text("text").to_string(), "ab");
    // An external original-byte archive can replay omitted pending input.
    without.import(&second.bytes).unwrap();
    assert_eq!(without.get_text("text").to_string(), "abc");
    let with = fit::document(9).unwrap();
    with.import(&snapshot).unwrap();
    assert!(with.import(&sidecar).unwrap().pending.is_some());
    with.import(&first.bytes).unwrap();
    assert_eq!(with.get_text("text").to_string(), "abc");
    assert!(matches!(
        fit::inspect(&base, &binding, 2, &second),
        Err(FitError::Rejected(
            "native span, timestamps or intent dependencies mismatch"
        ))
    ));
}

fn bare_update(text: &str) -> Vec<u8> {
    let doc = fit::document(7).unwrap();
    doc.get_text("text").insert(0, text).unwrap();
    doc.commit_with(loro::CommitOptions::new().timestamp(0));
    doc.export(loro::ExportMode::all_updates()).unwrap()
}

#[test]
fn fully_known_native_equivocation_is_a_noop_not_original_payload_verification() {
    let first = bare_update("x");
    let conflicting = bare_update("y");
    assert_ne!(first, conflicting);
    let receiver = fit::document(8).unwrap();
    receiver.import(&first).unwrap();
    let before = receiver.oplog_vv();
    let result = receiver.import(&conflicting).unwrap();
    assert!(result.pending.is_none());
    assert!(result.success.iter().next().is_none());
    assert_eq!(receiver.oplog_vv(), before);
    assert_eq!(receiver.get_text("text").to_string(), "x");
}

#[test]
fn conflicting_known_prefix_with_new_counters_is_rejected_by_pinned_loro() {
    let receiver = fit::document(8).unwrap();
    receiver.import(&bare_update("x")).unwrap();
    let conflicting = fit::document(7).unwrap();
    conflicting.get_text("text").insert(0, "y").unwrap();
    conflicting.commit_with(loro::CommitOptions::new().timestamp(0));
    conflicting.get_text("text").insert(1, "z").unwrap();
    conflicting.commit_with(loro::CommitOptions::new().timestamp(0));
    let update = conflicting.export(loro::ExportMode::all_updates()).unwrap();
    assert!(matches!(
        receiver.import(&update),
        Err(loro::LoroError::UsedOpID { id }) if id == loro::ID::new(7,0)
    ));
    assert_eq!(receiver.get_text("text").to_string(), "x");
    // Matching overlapping input has a different result: prefix checked,
    // ignored, suffix applied. It is not a copy of the original envelope.
    let matching = fit::document(7).unwrap();
    matching.get_text("text").insert(0, "x").unwrap();
    matching.commit_with(loro::CommitOptions::new().timestamp(0));
    matching.get_text("text").insert(1, "z").unwrap();
    matching.commit_with(loro::CommitOptions::new().timestamp(0));
    receiver
        .import(&matching.export(loro::ExportMode::all_updates()).unwrap())
        .unwrap();
    assert_eq!(receiver.get_text("text").to_string(), "xz");
}

#[test]
fn local_no_merge_does_not_make_exported_or_imported_change_boundaries_immutable() {
    for unique_messages in [false, true] {
        let source = fit::document(7).unwrap();
        let text = source.get_text("text");
        for (index, inserted) in ["x", "y"].into_iter().enumerate() {
            text.insert(index, inserted).unwrap();
            let options = loro::CommitOptions::new().timestamp(0);
            source.commit_with(if unique_messages {
                options.commit_msg(if index == 0 { "step-one" } else { "step-two" })
            } else {
                options
            });
        }
        assert_eq!(source.len_changes(), 2);
        let bytes = source.export(loro::ExportMode::all_updates()).unwrap();
        let expected = if unique_messages { 2 } else { 1 };
        assert_eq!(
            loro::LoroDoc::decode_import_blob_meta(&bytes, true)
                .unwrap()
                .change_num,
            expected
        );
        let loaded = fit::document(8).unwrap();
        loaded.import(&bytes).unwrap();
        assert_eq!(loaded.len_changes(), expected as usize);
        assert_eq!(loaded.get_text("text").to_string(), "xy");
    }
}

#[test]
fn native_full_history_does_not_retain_original_envelope_bytes() {
    let base = fit::FixtureBase::seed([7; 32], "a").unwrap();
    let binding = EpochPeers::new([7; 32]).bind(writer(1, 1, 1), 2).unwrap();
    let original = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 1..1,
            inserted: "b",
        }],
    )
    .unwrap();
    let mut alternative = original.bytes.clone();
    alternative[4] ^= 1; // Natively accepted, but outside our exact encoder profile.
    loro::LoroDoc::decode_import_blob_meta(&alternative, true).unwrap();
    let source = base.document().unwrap();
    source.import(&alternative).unwrap();
    assert_eq!(source.get_text("text").to_string(), "ab");
    let loaded = fit::document(9).unwrap();
    loaded
        .import(&source.export(loro::ExportMode::Snapshot).unwrap())
        .unwrap();
    let reexport = loaded
        .export(loro::ExportMode::updates_in_range(vec![loro::IdSpan::new(
            2, 0, 1,
        )]))
        .unwrap();
    assert_ne!(reexport, alternative);
    assert_eq!(reexport, original.bytes);
    // Exact known originals replay idempotently, but the caller's raw bytes
    // are separate evidence, not something that native snapshot recovered.
    let before = loaded.oplog_vv();
    assert!(
        loaded
            .import(&alternative)
            .unwrap()
            .success
            .iter()
            .next()
            .is_none()
    );
    assert_eq!(loaded.oplog_vv(), before);
}

#[test]
fn same_closed_cut_with_opposite_import_orders_accepts_original_update_bytes() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let mut peers = EpochPeers::new([7; 32]);
    let a = peers.bind(writer(1, 1, 1), 2).unwrap();
    let b = peers.bind(writer(2, 1, 1), 3).unwrap();
    let c = peers.bind(writer(3, 1, 1), 4).unwrap();
    let mut updates = Vec::new();
    for (binding, position, inserted) in [(&a, 1, "X"), (&b, 2, "Y")] {
        updates.push(
            fit::author(
                &base,
                binding,
                1,
                &[fit::ByteEdit {
                    range: position..position,
                    inserted,
                }],
            )
            .unwrap(),
        );
    }
    let mut cuts = Vec::new();
    for order in [[0, 1], [1, 0]] {
        let doc = base.document().unwrap();
        for index in order {
            doc.import(&updates[index].bytes).unwrap();
        }
        assert_eq!(doc.get_text("text").to_string(), "aXbYc");
        cuts.push(fit::FixtureBase::capture([7; 32], &doc, doc.oplog_frontiers()).unwrap());
    }
    let original = fit::author(
        &cuts[0],
        &c,
        1,
        &[fit::ByteEdit {
            range: 2..3,
            inserted: "Z",
        }],
    )
    .unwrap();
    for cut in cuts {
        assert_eq!(
            fit::inspect(&cut, &c, 1, &original)
                .unwrap()
                .get_text("text")
                .to_string(),
            "aXZYc"
        );
    }
}
