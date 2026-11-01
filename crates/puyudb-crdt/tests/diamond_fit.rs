// SPDX-License-Identifier: Apache-2.0

//! Bounded candidate-fit experiments, not a selected engine or hardened decoder.

#[path = "support/diamond_fit.rs"]
mod fit;

#[path = "support/diamond_bytes.rs"]
mod bytes;

fn writer() -> fit::FixtureWriter {
    fit::FixtureWriter {
        key: [1; 32],
        branch: [1; 16],
        writer_epoch: [1; 16],
    }
}

#[test]
fn full_context_digest_fits_native_name_without_truncation() {
    let original = writer();
    let name = fit::writer_name(&[7; 32], &original);
    // Independently calculated with Python hashlib + urlsafe_b64encode.
    assert_eq!(name, "w:nsrXCNn8XQ-x3AwxUYn_QF4IeWHLXz3fzysrMHcQE3c");
    assert_eq!(name.len(), 45);
    for changed in [
        fit::FixtureWriter {
            key: [2; 32],
            ..original
        },
        fit::FixtureWriter {
            branch: [2; 16],
            ..original
        },
        fit::FixtureWriter {
            writer_epoch: [2; 16],
            ..original
        },
    ] {
        assert_ne!(fit::writer_name(&[7; 32], &changed), name);
    }
    assert_ne!(fit::writer_name(&[8; 32], &original), name);
}

#[test]
fn exact_cut_staged_unicode_edits_roundtrip_without_mutating_base() {
    let base = fit::FixtureBase::seed([7; 32], "A😀bc").unwrap();
    let original = base
        .document()
        .unwrap()
        .encode(&diamond_types::list::encoding::EncodeOptions::full().compress_content(false));
    let update = fit::author(
        &base,
        &[7; 32],
        &writer(),
        1,
        &[
            fit::ByteEdit {
                range: 5..6,
                inserted: "日本",
            },
            fit::ByteEdit {
                range: 11..12,
                inserted: "!",
            },
        ],
    )
    .unwrap();
    let accepted = fit::inspect(&base, &[7; 32], &writer(), 1, &update).unwrap();
    assert_eq!(accepted.checkout_tip().content().to_string(), "A😀日本!");
    assert_eq!(
        base.document()
            .unwrap()
            .encode(&diamond_types::list::encoding::EncodeOptions::full().compress_content(false)),
        original
    );
}

fn update(base: &fit::FixtureBase, sequence: u64) -> fit::CapturedUpdate {
    fit::author(
        base,
        &[7; 32],
        &writer(),
        sequence,
        &[fit::ByteEdit {
            range: 1..2,
            inserted: "X",
        }],
    )
    .unwrap()
}

#[test]
fn empty_native_seed_needs_external_epoch_descriptor_without_dummy_operations() {
    let a = fit::FixtureBase::seed([7; 32], "").unwrap();
    let b = fit::FixtureBase::seed([8; 32], "").unwrap();
    let options = diamond_types::list::encoding::EncodeOptions::full().compress_content(false);
    assert_eq!(a.document().unwrap().len(), 0);
    assert_eq!(
        a.document().unwrap().encode(&options),
        b.document().unwrap().encode(&options)
    );
    assert_ne!(fit::seed_name(&[7; 32]), fit::seed_name(&[8; 32]));
    assert_ne!(
        fit::seed_name(&[7; 32]),
        fit::writer_name(&[7; 32], &writer())
    );
    let artifact = fit::author(
        &a,
        &[7; 32],
        &writer(),
        1,
        &[fit::ByteEdit {
            range: 0..0,
            inserted: "😀",
        }],
    )
    .unwrap();
    assert_eq!(
        fit::inspect(&a, &[7; 32], &writer(), 1, &artifact)
            .unwrap()
            .checkout_tip()
            .content()
            .to_string(),
        "😀"
    );
    assert!(matches!(
        fit::inspect(&b, &[8; 32], &writer(), 1, &artifact),
        Err(fit::FitError::Rejected(
            "native fixture writer or counter span mismatch"
        ))
    ));
}

#[test]
fn full_writer_context_and_raw_sequence_metadata_are_checked_not_authenticated() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let artifact = update(&base, 1);
    for changed in [
        fit::FixtureWriter {
            key: [2; 32],
            ..writer()
        },
        fit::FixtureWriter {
            branch: [2; 16],
            ..writer()
        },
        fit::FixtureWriter {
            writer_epoch: [2; 16],
            ..writer()
        },
    ] {
        assert!(matches!(
            fit::inspect(&base, &[7; 32], &changed, 1, &artifact),
            Err(fit::FitError::Rejected(
                "native fixture writer or counter span mismatch"
            ))
        ));
    }
    assert!(matches!(
        fit::inspect(&base, &[7; 32], &writer(), 2, &artifact),
        Err(fit::FitError::Rejected(
            "fixture bytes differ from independent native reconstruction"
        ))
    ));
    assert!(matches!(
        fit::inspect(&base, &[8; 32], &writer(), 1, &artifact),
        Err(fit::FitError::Rejected(
            "fixture epoch or outer sequence mismatch"
        ))
    ));
    assert!(matches!(
        fit::inspect(&base, &[7; 32], &writer(), 0, &artifact),
        Err(fit::FitError::Rejected(
            "fixture epoch or outer sequence mismatch"
        ))
    ));
}

#[test]
fn whole_original_bytes_reject_native_ignored_trailing_data_and_known_prefix() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let original = update(&base, 1);
    let mut trailing = original.bytes.clone();
    trailing.extend_from_slice(b"ignored trailing bytes");
    assert!(matches!(
        fit::inspect(
            &base,
            &[7; 32],
            &writer(),
            1,
            &fit::CapturedUpdate::new(trailing)
        ),
        Err(fit::FitError::Rejected(
            "fixture bytes differ from independent native reconstruction"
        ))
    ));
    let doc = fit::inspect(&base, &[7; 32], &writer(), 1, &original).unwrap();
    let context = fit::message(&[7; 32], &writer(), 1);
    let full = doc.encode(
        &diamond_types::list::encoding::EncodeOptions::full()
            .compress_content(false)
            .user_data(context.as_bytes()),
    );
    assert!(matches!(
        fit::inspect(
            &base,
            &[7; 32],
            &writer(),
            1,
            &fit::CapturedUpdate::new(full)
        ),
        Err(fit::FitError::Rejected(
            "fixture bytes differ from independent native reconstruction"
        ))
    ));
}

#[test]
fn digest_size_and_already_known_span_guards_are_specific() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let mut artifact = update(&base, 1);
    artifact.sha256[0] ^= 1;
    assert!(matches!(
        fit::inspect(&base, &[7; 32], &writer(), 1, &artifact),
        Err(fit::FitError::Rejected("fixture artifact digest mismatch"))
    ));
    let huge = fit::CapturedUpdate::new(vec![0; 8 * 1024 * 1024 + 1]);
    assert!(matches!(
        fit::inspect(&base, &[7; 32], &writer(), 1, &huge),
        Err(fit::FitError::Rejected(
            "encoded fixture exceeds experimental limit"
        ))
    ));
    let artifact = update(&base, 1);
    let doc = fit::inspect(&base, &[7; 32], &writer(), 1, &artifact).unwrap();
    let later = fit::FixtureBase::capture([7; 32], &doc, fit::frontier(&doc)).unwrap();
    assert!(matches!(
        fit::inspect(&later, &[7; 32], &writer(), 2, &artifact),
        Err(fit::FitError::Rejected(
            "expected a bounded new fixture span"
        ))
    ));
}

#[test]
fn malformed_byte_ranges_and_empty_authorship_are_rejected() {
    let base = fit::FixtureBase::seed([7; 32], "A😀bc").unwrap();
    for range in [2..5, 1..4, 8..8, std::ops::Range { start: 6, end: 5 }] {
        assert!(matches!(
            fit::author(
                &base,
                &[7; 32],
                &writer(),
                1,
                &[fit::ByteEdit {
                    range,
                    inserted: "x"
                },]
            ),
            Err(fit::FitError::Rejected("invalid fixture UTF-8 byte range"))
        ));
    }
    assert!(matches!(
        fit::author(&base, &[7; 32], &writer(), 1, &[]),
        Err(fit::FitError::Rejected("empty authored fixture"))
    ));
}

#[test]
fn unrelated_newer_history_does_not_broaden_intent_and_stale_writer_is_refused() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let initial = fit::frontier(&base.document().unwrap());
    let newer = fit::inspect(&base, &[7; 32], &writer(), 1, &update(&base, 1)).unwrap();
    let declared = fit::FixtureBase::capture([7; 32], &newer, initial).unwrap();
    assert_eq!(
        declared
            .document()
            .unwrap()
            .checkout_tip()
            .content()
            .to_string(),
        "abc"
    );
    assert!(matches!(
        fit::author(
            &declared,
            &[7; 32],
            &writer(),
            2,
            &[fit::ByteEdit {
                range: 1..2,
                inserted: "Y"
            },]
        ),
        Err(fit::FitError::Rejected(
            "historical fixture edit requires a fresh writer epoch"
        ))
    ));
    let fresh = fit::FixtureWriter {
        writer_epoch: [2; 16],
        ..writer()
    };
    let artifact = fit::author(
        &declared,
        &[7; 32],
        &fresh,
        1,
        &[fit::ByteEdit {
            range: 1..2,
            inserted: "Y",
        }],
    )
    .unwrap();
    assert_eq!(
        fit::inspect(&declared, &[7; 32], &fresh, 1, &artifact)
            .unwrap()
            .checkout_tip()
            .content()
            .to_string(),
        "aYc"
    );
}

#[test]
fn prefix_copy_handles_mid_run_native_cuts_without_transferring_local_indexes() {
    use diamond_types::causalgraph::agent_assignment::remote_ids::RemoteVersionOwned;
    let base = fit::FixtureBase::seed([7; 32], "A😀bc").unwrap();
    let full = base.document().unwrap();
    // Native diagnostic only: a PuyuDB outer version must respect seed/batch atomicity.
    let cut = [RemoteVersionOwned::from((fit::seed_name(&[7; 32]), 1))]
        .into_iter()
        .collect();
    let prefix = fit::FixtureBase::capture([7; 32], &full, cut).unwrap();
    assert_eq!(
        prefix
            .document()
            .unwrap()
            .checkout_tip()
            .content()
            .to_string(),
        "A😀"
    );
    let deletion = fit::author(
        &base,
        &[7; 32],
        &writer(),
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
    let accepted = fit::inspect(&base, &[7; 32], &writer(), 1, &deletion).unwrap();
    assert_eq!(accepted.checkout_tip().content().to_string(), "Ac");
    let cut = [RemoteVersionOwned::from((
        fit::writer_name(&[7; 32], &writer()),
        0,
    ))]
    .into_iter()
    .collect();
    let prefix = fit::FixtureBase::capture([7; 32], &accepted, cut).unwrap();
    assert_eq!(
        prefix
            .document()
            .unwrap()
            .checkout_tip()
            .content()
            .to_string(),
        "A😀c"
    );
}

#[test]
fn root_unknown_duplicate_and_redundant_cuts_are_not_silently_normalized() {
    use diamond_types::causalgraph::agent_assignment::remote_ids::RemoteVersionOwned;
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let doc = base.document().unwrap();
    let root = fit::FixtureBase::capture([7; 32], &doc, Default::default()).unwrap();
    assert_eq!(root.document().unwrap().len(), 0);
    let unknown = [RemoteVersionOwned::from(("unknown", 0))]
        .into_iter()
        .collect();
    assert!(matches!(
        fit::FixtureBase::capture([7; 32], &doc, unknown),
        Err(fit::FitError::Rejected("unavailable fixture frontier"))
    ));
    let mut duplicate = fit::frontier(&doc);
    duplicate.push(duplicate[0].clone());
    assert!(matches!(
        fit::FixtureBase::capture([7; 32], &doc, duplicate),
        Err(fit::FitError::Rejected("noncanonical fixture frontier"))
    ));
    let name = fit::seed_name(&[7; 32]);
    let redundant = [
        RemoteVersionOwned::from((name.clone(), 0)),
        RemoteVersionOwned::from((name, 2)),
    ]
    .into_iter()
    .collect();
    assert!(matches!(
        fit::FixtureBase::capture([7; 32], &doc, redundant),
        Err(fit::FitError::Rejected("redundant fixture frontier"))
    ));
}

fn raw(
    base: &fit::FixtureBase,
    agent_name: &str,
    op: diamond_types::list::operation::TextOperation,
) -> fit::CapturedUpdate {
    let mut log = base.document().unwrap();
    let from = log.local_frontier();
    let agent = log.get_or_create_agent_id(agent_name);
    log.add_operations_at(agent, from.as_ref(), &[op]);
    let context = fit::message(&[7; 32], &writer(), 1);
    fit::CapturedUpdate::new(
        log.encode_from(
            &diamond_types::list::encoding::EncodeOptions::patch()
                .compress_content(false)
                .user_data(context.as_bytes()),
            from.as_ref(),
        ),
    )
}

#[test]
fn foreign_seed_impersonation_and_out_of_cut_positions_are_rejected_before_checkout() {
    use diamond_types::list::operation::TextOperation;
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    for name in ["foreign-agent".to_owned(), fit::seed_name(&[7; 32])] {
        let artifact = raw(&base, &name, TextOperation::new_insert(0, "x"));
        assert!(matches!(
            fit::inspect(&base, &[7; 32], &writer(), 1, &artifact),
            Err(fit::FitError::Rejected(
                "native fixture writer or counter span mismatch"
            ))
        ));
    }
    let name = fit::writer_name(&[7; 32], &writer());
    let artifact = raw(&base, &name, TextOperation::new_delete(3..4));
    assert!(matches!(
        fit::inspect(&base, &[7; 32], &writer(), 1, &artifact),
        Err(fit::FitError::Rejected(
            "unsupported native fixture deletion"
        ))
    ));
}

#[test]
fn same_closed_cut_can_false_reject_original_bytes_under_opposite_import_orders() {
    let seed = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let mut artifacts = Vec::new();
    for (key, position, inserted) in [(2, 1, "X"), (3, 2, "Y")] {
        let other = fit::FixtureWriter {
            key: [key; 32],
            ..writer()
        };
        artifacts.push(
            fit::author(
                &seed,
                &[7; 32],
                &other,
                1,
                &[fit::ByteEdit {
                    range: position..position,
                    inserted,
                }],
            )
            .unwrap(),
        );
    }
    let mut bases = Vec::new();
    for order in [[0, 1], [1, 0]] {
        let mut log = seed.document().unwrap();
        for i in order {
            log.decode_and_add(&artifacts[i].bytes).unwrap();
        }
        assert_eq!(log.checkout_tip().content().to_string(), "aXbYc");
        bases.push(fit::FixtureBase::capture([7; 32], &log, fit::frontier(&log)).unwrap());
    }
    assert_eq!(
        fit::frontier(&bases[0].document().unwrap()),
        fit::frontier(&bases[1].document().unwrap())
    );
    let mut originals = Vec::new();
    for base in &bases {
        let artifact = fit::author(
            base,
            &[7; 32],
            &writer(),
            1,
            &[fit::ByteEdit {
                range: 2..3,
                inserted: "Z",
            }],
        )
        .unwrap();
        let accepted = fit::inspect(base, &[7; 32], &writer(), 1, &artifact).unwrap();
        assert_eq!(accepted.checkout_tip().content().to_string(), "aXZYc");
        originals.push(artifact);
    }
    assert_ne!(originals[0].bytes, originals[1].bytes);
    for (base, artifact) in [(&bases[1], &originals[0]), (&bases[0], &originals[1])] {
        let mut native = base.document().unwrap();
        native.decode_and_add(&artifact.bytes).unwrap();
        assert_eq!(native.checkout_tip().content().to_string(), "aXZYc");
        // Same logical cut/intent, but native encoding inherits local arrival
        // order. Keep the byte guard: this is an inspector-profile LIMITATION,
        // not permission to accept ignored fields or a native merge failure.
        assert!(matches!(
            fit::inspect(base, &[7; 32], &writer(), 1, artifact),
            Err(fit::FitError::Rejected(
                "fixture bytes differ from independent native reconstruction"
            ))
        ));
    }
}

#[test]
fn missing_insert_content_is_rejected_before_the_native_checkout_panic_path() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let original = update(&base, 1);
    let doc = fit::inspect(&base, &[7; 32], &writer(), 1, &original).unwrap();
    let from = base.document().unwrap().local_frontier();
    let context = fit::message(&[7; 32], &writer(), 1);
    let bytes = doc.encode_from(
        &diamond_types::list::encoding::EncodeOptions::patch()
            .compress_content(false)
            .store_inserted_content(false)
            .user_data(context.as_bytes()),
        from.as_ref(),
    );
    assert!(matches!(
        fit::inspect(
            &base,
            &[7; 32],
            &writer(),
            1,
            &fit::CapturedUpdate::new(bytes)
        ),
        Err(fit::FitError::Rejected("missing native fixture content"))
    ));
}

#[test]
fn valid_crc_unknown_chunks_cannot_hide_in_original_input() {
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let original = update(&base, 1);
    let end = original.bytes.len() - 6;
    assert_eq!(&original.bytes[end..end + 2], &[100, 4]);
    assert_eq!(bytes::seal(original.bytes[..end].to_vec()), original.bytes);
    for offset in [9, end] {
        let mut prefix = original.bytes[..end].to_vec();
        prefix.splice(offset..offset, [99, 1, 42]); // Before FileInfo or CRC.
        let mut altered = bytes::seal(prefix);
        if offset == end {
            // This placement also makes native CRC probing stop prematurely.
            *altered.last_mut().unwrap() ^= 1;
        }
        let mut native = base.document().unwrap();
        native.decode_and_add(&altered).unwrap();
        assert_eq!(native.checkout_tip().content().to_string(), "aXc");
        assert!(matches!(
            fit::inspect(
                &base,
                &[7; 32],
                &writer(),
                1,
                &fit::CapturedUpdate::new(altered)
            ),
            Err(fit::FitError::Rejected(
                "fixture bytes differ from independent native reconstruction"
            ))
        ));
    }
}

#[test]
fn conflicting_known_prefix_with_valid_new_suffix_is_not_hidden_by_delta_inspection() {
    let base = fit::FixtureBase::seed([7; 32], "a").unwrap();
    let first = fit::author(
        &base,
        &[7; 32],
        &writer(),
        1,
        &[fit::ByteEdit {
            range: 1..1,
            inserted: "x",
        }],
    )
    .unwrap();
    let known = fit::inspect(&base, &[7; 32], &writer(), 1, &first).unwrap();
    let current = fit::FixtureBase::capture([7; 32], &known, fit::frontier(&known)).unwrap();
    let mut conflicting = base.document().unwrap();
    let from = conflicting.local_frontier();
    let agent = conflicting.get_or_create_agent_id(&fit::writer_name(&[7; 32], &writer()));
    conflicting.add_insert_at(agent, from.as_ref(), 1, "yz");
    let context = fit::message(&[7; 32], &writer(), 2);
    let artifact = conflicting.encode_from(
        &diamond_types::list::encoding::EncodeOptions::patch()
            .compress_content(false)
            .user_data(context.as_bytes()),
        from.as_ref(),
    );
    let mut native = known;
    native.decode_and_add(&artifact).unwrap();
    assert_eq!(native.checkout_tip().content().to_string(), "axz");
    assert!(matches!(
        fit::inspect(
            &current,
            &[7; 32],
            &writer(),
            2,
            &fit::CapturedUpdate::new(artifact)
        ),
        Err(fit::FitError::Rejected(
            "fixture bytes differ from independent native reconstruction"
        ))
    ));
}

#[test]
fn root_authored_operation_cannot_claim_a_nonroot_intent_cut() {
    use diamond_types::list::{encoding::EncodeOptions, operation::TextOperation};
    let base = fit::FixtureBase::seed([7; 32], "abc").unwrap();
    let mut log = base.document().unwrap();
    let from = log.local_frontier();
    let agent = log.get_or_create_agent_id(&fit::writer_name(&[7; 32], &writer()));
    log.add_operations_at(agent, &[], &[TextOperation::new_insert(0, "X")]);
    let context = fit::message(&[7; 32], &writer(), 1);
    let artifact = log.encode_from(
        &EncodeOptions::patch()
            .compress_content(false)
            .user_data(context.as_bytes()),
        from.as_ref(),
    );
    assert!(matches!(
        fit::inspect(
            &base,
            &[7; 32],
            &writer(),
            1,
            &fit::CapturedUpdate::new(artifact)
        ),
        Err(fit::FitError::Rejected(
            "native fixture intent dependencies mismatch"
        ))
    ));
}
