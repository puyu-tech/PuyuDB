// SPDX-License-Identifier: Apache-2.0
//! Bounded native strict-input spike; not a hostile-input security certificate.
//! Fixtures use the native encoder, or small literal chunk trees checked against it.
//! No admission decoder, byte canonicalization, or replacement CRDT integration.

use diamond_types::{
    HasLength,
    list::{
        ListOpLog,
        encoding::{EncodeOptions, ParseError},
        operation::{ListOpKind, TextOperation},
    },
};

// Independent CRC-32C fixture arithmetic, not a native decoder.
fn seal(mut prefix: Vec<u8>) -> Vec<u8> {
    let mut crc = u32::MAX;
    for &byte in &prefix {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 0 { 0 } else { 0x82f6_3b78 };
        }
    }
    prefix.extend([100, 4]);
    prefix.extend((!crc).to_le_bytes());
    prefix
}

fn options<'a>() -> EncodeOptions<'a> {
    EncodeOptions::patch().compress_content(false)
}

fn inserted(name: &str, text: &str) -> ListOpLog {
    let mut log = ListOpLog::new();
    let agent = log.get_or_create_agent_id(name);
    log.add_insert_at(agent, &[], 0, text);
    log
}

fn text(log: &ListOpLog) -> String {
    log.checkout_tip().content().to_string()
}

fn assert_rejected(base: &ListOpLog, data: &[u8]) -> ParseError {
    let snapshot = base.encode(&options());
    let operations: Vec<_> = base.iter_ops().collect();
    let history: Vec<_> = base.iter_history().collect();
    let frontier = base.local_frontier();
    let agents = base.num_agents();
    let error = base
        .decode_new_strict(data)
        .expect_err("strict input must reject");
    assert_eq!(base.encode(&options()), snapshot);
    assert_eq!(base.iter_ops().collect::<Vec<_>>(), operations);
    assert_eq!(base.iter_history().collect::<Vec<_>>(), history);
    assert_eq!(base.local_frontier(), frontier);
    assert_eq!(base.num_agents(), agents);
    error
}

#[test]
fn strict_rejects_trailing_input() {
    let original = inserted("writer", "A").encode(&options());
    let second = inserted("second", "B").encode(&options());
    for suffix in [b"unconsumed".as_slice(), second.as_slice()] {
        let mut extended = original.clone();
        extended.extend_from_slice(suffix);
        assert_eq!(text(&ListOpLog::load_from(&extended).unwrap()), "A");
        assert_eq!(
            assert_rejected(&ListOpLog::new(), &extended),
            ParseError::InvalidLength
        );
    }
}

// These are fixture constructors, not parsers. All fixture tags and lengths fit
// one byte. Unknown/nested mutations need not understand any native op stream.
fn chunk(tag: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 128);
    [
        vec![tag, u8::try_from(payload.len()).unwrap()],
        payload.to_vec(),
    ]
    .concat()
}

fn empty_patches() -> Vec<u8> {
    [chunk(21, &[]), chunk(22, &[]), chunk(23, &[])].concat()
}

fn envelope(info: &[u8], start: &[u8], patches: &[u8]) -> Vec<u8> {
    seal(
        [
            b"DMNDTYPS\0".to_vec(),
            chunk(1, info),
            chunk(10, start),
            chunk(20, patches),
        ]
        .concat(),
    )
}

#[test]
fn strict_checks_unknown_chunks_crc_presence_and_crc_payload_exhaustion() {
    let empty = envelope(&chunk(3, &[]), &[], &empty_patches());
    assert_eq!(empty, ListOpLog::new().encode(&options()));
    for offset in [9, 13, empty.len() - 6] {
        let mut prefix = empty[..empty.len() - 6].to_vec();
        prefix.splice(offset..offset, chunk(99, &[42]));
        let valid_crc = seal(prefix);
        assert!(ListOpLog::load_from(&valid_crc).is_ok());
        assert!(assert_rejected(&ListOpLog::new(), &valid_crc) != ParseError::ChecksumFailed);
        if offset == empty.len() - 6 {
            let mut bypass = valid_crc;
            *bypass.last_mut().unwrap() ^= 1;
            assert!(ListOpLog::load_from(&bypass).is_ok()); // Old before-CRC bypass.
            assert_eq!(
                assert_rejected(&ListOpLog::new(), &bypass),
                ParseError::UnknownChunk
            );
        }
    }
    let missing = &empty[..empty.len() - 6];
    assert!(ListOpLog::load_from(missing).is_ok());
    assert!(ListOpLog::new().decode_new_strict(missing).is_err());
    let mut invalid = empty.clone();
    *invalid.last_mut().unwrap() ^= 1;
    assert_eq!(
        assert_rejected(&ListOpLog::new(), &invalid),
        ParseError::ChecksumFailed
    );
    let mut extra_crc_payload = empty;
    let length_offset = extra_crc_payload.len() - 5;
    extra_crc_payload[length_offset] = 5;
    extra_crc_payload.push(0);
    assert!(ListOpLog::load_from(&extra_crc_payload).is_ok());
    assert_eq!(
        assert_rejected(&ListOpLog::new(), &extra_crc_payload),
        ParseError::InvalidLength
    );
}

#[test]
fn strict_rejects_ignored_fileinfo_startbranch_and_operation_payloads() {
    let names = chunk(3, &[]);
    let junk = chunk(99, &[42]);
    let plain_empty_content = chunk(13, &[4]);
    for data in [
        envelope(
            &[names.clone(), junk.clone()].concat(),
            &[],
            &empty_patches(),
        ),
        envelope(
            &names,
            &[plain_empty_content, junk.clone()].concat(),
            &empty_patches(),
        ),
        // No assignments/history, but an extra valid positional operation.
        envelope(
            &names,
            &[],
            &[chunk(21, &[]), chunk(22, &[0]), chunk(23, &[])].concat(),
        ),
    ] {
        assert!(ListOpLog::load_from(&data).is_ok());
        assert_rejected(&ListOpLog::new(), &data);
    }
    // Unknown and known-but-unsupported payloads cannot be skipped in any position.
    for tag in [99, 11, 27, 28, 5, 14] {
        let mut prefix = b"DMNDTYPS\0".to_vec();
        prefix.extend(chunk(tag, &[]));
        let data = ListOpLog::new().encode(&options());
        prefix.extend_from_slice(&data[9..data.len() - 6]);
        assert_rejected(&ListOpLog::new(), &seal(prefix));
        let nested = envelope(&names, &[], &[chunk(tag, &[]), empty_patches()].concat());
        assert_rejected(&ListOpLog::new(), &nested);
    }
}

fn insertion_content(extra: &[u8]) -> Vec<u8> {
    [
        vec![0],
        chunk(13, &[4, b'A']),
        chunk(25, &[3]),
        extra.to_vec(),
    ]
    .concat()
}

fn insertion_patches(content: &[u8], positions: &[u8]) -> Vec<u8> {
    [
        chunk(24, content),
        chunk(21, &[2, 1]),
        chunk(22, positions),
        chunk(23, &[1, 1]),
    ]
    .concat()
}

#[test]
fn strict_rejects_nested_content_leftovers_overwritten_content_and_buffered_operations() {
    let names = chunk(3, &[1, b'w']);
    let content = insertion_content(&[]);
    let plain = envelope(&names, &[], &insertion_patches(&content, &[0]));
    assert_eq!(plain, inserted("w", "A").encode(&options()));
    assert_eq!(
        text(&ListOpLog::new().decode_new_strict(&plain).unwrap().0),
        "A"
    );
    let nested_junk = insertion_content(&chunk(99, &[42]));
    let mut duplicate = chunk(24, &content);
    duplicate.extend(insertion_patches(&content, &[0]));
    for data in [
        envelope(&names, &[], &insertion_patches(&nested_junk, &[0])),
        envelope(&names, &[], &duplicate),
        envelope(&names, &[], &insertion_patches(&content, &[0, 0])),
        // One length-two insertion with only one ID: remainder sits in BufferedIter.
        envelope(&names, &[], &insertion_patches(&content, &[17])),
    ] {
        assert!(ListOpLog::load_from(&data).is_ok());
        assert_rejected(&ListOpLog::new(), &data);
    }
}

#[test]
fn strict_rejects_missing_or_leftover_insert_content_before_checkout() {
    let source = inserted("writer", "A");
    let missing = source.encode(&options().store_inserted_content(false));
    assert!(ListOpLog::load_from(&missing).is_ok()); // Existing contentless dialect.
    assert_eq!(
        assert_rejected(&ListOpLog::new(), &missing),
        ParseError::InvalidContent
    );
    let names = chunk(3, &[1, b'w']);
    for content in [
        [vec![0], chunk(13, &[4, b'A', b'B']), chunk(25, &[3])].concat(),
        [vec![0], chunk(13, &[4, b'A']), chunk(25, &[3, 2])].concat(),
        [vec![0], chunk(13, &[4]), chunk(25, &[2])].concat(),
    ] {
        let data = envelope(&names, &[], &insertion_patches(&content, &[0]));
        assert_rejected(&ListOpLog::new(), &data);
    }
}

#[test]
fn strict_exposes_original_envelope_metadata_including_empty_vs_absent() {
    for user_data in [
        None,
        Some(b"".as_slice()),
        Some(b"external epoch descriptor\0".as_slice()),
    ] {
        let info = [
            chunk(2, &[4, b'd', b'o', b'c']),
            chunk(3, &[]),
            user_data.map(|data| chunk(4, data)).unwrap_or_default(),
        ]
        .concat();
        let data = envelope(&info, &chunk(13, &[4]), &empty_patches());
        let (stage, report) = ListOpLog::new().decode_new_strict(&data).unwrap();
        assert_eq!(report.doc_id.as_deref(), Some("doc"));
        assert_eq!(report.user_data.as_deref(), user_data);
        assert_eq!(report.start_content.as_deref(), Some(""));
        assert!(report.start_frontier.is_empty());
        assert!(report.end_frontier.is_empty());
        assert!(report.submitted.is_empty());
        assert!(stage.is_empty());
    }
}

#[test]
fn strict_exposes_even_unused_and_duplicate_agent_declarations() {
    let data = envelope(
        &chunk(3, &[1, b'x', 1, b'x', 1, b'y']),
        &[],
        &empty_patches(),
    );
    let (stage, report) = ListOpLog::new().decode_new_strict(&data).unwrap();
    assert_eq!(report.agent_names, ["x", "x", "y"]);
    assert_eq!(stage.num_agents(), 2);
    assert!(report.submitted.is_empty());
    // Understood optional declarations are not canonicalized or hidden; a
    // caller may disallow them. Native re-encoding alone loses these values.
    assert_ne!(stage.encode(&options()), data);
}

#[test]
fn strict_report_outlives_original_input_and_stage() {
    let report = {
        let info = [
            chunk(2, &[4, b'd', b'o', b'c']),
            chunk(3, &[1, b'w']),
            chunk(4, b"original metadata"),
        ]
        .concat();
        let data = envelope(&info, &chunk(13, &[4]), &empty_patches());
        let (_, report) = ListOpLog::new().decode_new_strict(&data).unwrap();
        report
    };
    assert_eq!(report.doc_id.as_deref(), Some("doc"));
    assert_eq!(
        report.user_data.as_deref(),
        Some(b"original metadata".as_slice())
    );
    assert_eq!(report.agent_names, ["w"]);
    assert_eq!(report.start_content.as_deref(), Some(""));
}

#[test]
fn strict_rejects_every_known_span_including_exact_replay_and_conflicting_prefixes() {
    let base = inserted("writer", "A");
    for submitted in ["A", "B", "AX", "BX"] {
        let data = inserted("writer", submitted).encode(&options());
        let mut legacy = base.clone();
        legacy.decode_and_add(&data).unwrap();
        assert_eq!(text(&legacy), if submitted.len() == 1 { "A" } else { "AX" });
        assert_eq!(assert_rejected(&base, &data), ParseError::KnownOperation);
    }
    // Even when the advertised start frontier equals the receiver tip, strict
    // inspection must not take native replay's "non-overlap" fast path.
    let names = chunk(3, &[6, b'w', b'r', b'i', b't', b'e', b'r']);
    let data = envelope(
        &names,
        &chunk(12, &[2, 0]),
        &insertion_patches(&insertion_content(&[]), &[0]),
    );
    assert_eq!(assert_rejected(&base, &data), ParseError::KnownOperation);
}

#[test]
fn known_foreign_prefix_and_known_suffix_cannot_hide_behind_new_operations() {
    let base = inserted("foreign", "A");
    let mut incoming = inserted("foreign", "B");
    let author = incoming.get_or_create_agent_id("third");
    incoming.add_insert(author, 1, "X");
    let data = incoming.encode(&options());
    let mut legacy = base.clone();
    legacy.decode_and_add(&data).unwrap();
    assert_eq!(text(&legacy), "AX");
    assert_eq!(assert_rejected(&base, &data), ParseError::KnownOperation);

    let mut sparse_base = ListOpLog::new();
    let author = sparse_base.get_or_create_agent_id("writer");
    sparse_base.add_operations_remote(author, &[], 1, &[TextOperation::new_insert(0, "A")]);
    let known_suffix = inserted("writer", "BX").encode(&options());
    assert_eq!(
        assert_rejected(&sparse_base, &known_suffix),
        ParseError::KnownOperation
    );
}

#[test]
fn repeated_ids_inside_one_submission_are_rejected() {
    let names = chunk(3, &[1, b'w']);
    // Second agent span jumps back from sequence 1 to 0 (old zigzag(-1) = 3).
    // Strict rejects before reading/filtering the duplicate operation's body.
    let patches = [
        chunk(24, &insertion_content(&[])),
        chunk(21, &[2, 1, 3, 1, 3]),
        chunk(22, &[0, 0]),
        chunk(23, &[1, 1, 1, 1]),
    ]
    .concat();
    let data = envelope(&names, &[], &patches);
    assert_eq!(
        assert_rejected(&ListOpLog::new(), &data),
        ParseError::KnownOperation
    );
}

fn remote_frontier(log: &ListOpLog, cut: &[usize]) -> Vec<(String, usize)> {
    let mut ids: Vec<_> = log
        .cg
        .agent_assignment
        .local_to_remote_frontier_owned(cut)
        .iter()
        .map(|id| (id.0.to_string(), id.1))
        .collect();
    ids.sort();
    ids
}

// External synthetic admission policy. Strict decode establishes full inspection,
// not this writer's authorization, counter continuity or exact parent chain.
fn inspect_policy(
    base: &ListOpLog,
    data: &[u8],
    metadata: &[u8],
) -> Result<ListOpLog, &'static str> {
    let (stage, report) = base.decode_new_strict(data).map_err(|_| "native")?;
    if report.user_data.as_deref() != Some(metadata) || report.doc_id.is_some() {
        return Err("metadata");
    }
    if report
        .agent_names
        .iter()
        .any(|name| name != "third" && base.cg.agent_assignment.get_agent_id(name).is_none())
    {
        return Err("declarations");
    }
    if remote_frontier(&stage, report.start_frontier.as_ref())
        != remote_frontier(base, base.local_frontier_ref())
    {
        return Err("start frontier");
    }
    if let Some(content) = report.start_content
        && content != text(base)
    {
        return Err("start content");
    }
    if report.submitted.start != base.len()
        || report.submitted.end != stage.len()
        || report.submitted.is_empty()
    {
        return Err("new range");
    }
    let mut expected_parents = remote_frontier(base, base.local_frontier_ref());
    let mut next = 0;
    let mut length = base.checkout_tip().len();
    for (op, history, ids) in stage.iter_full_range(report.submitted) {
        if ids.0 != "third" || ids.1.start != next || ids.1.len() != op.len() {
            return Err("ownership");
        }
        if remote_frontier(&stage, history.parents.as_ref()) != expected_parents {
            return Err("parents");
        }
        match op.kind {
            ListOpKind::Ins => {
                if !op.loc.fwd
                    || op.start() > length
                    || op.content_as_str().unwrap().chars().count() != op.len()
                {
                    return Err("insert");
                }
                length += op.len();
            }
            ListOpKind::Del => {
                if op.end() > length || op.content.is_some() {
                    return Err("delete");
                }
                length -= op.len();
            }
        }
        next += op.len();
        expected_parents = remote_frontier(&stage, &[history.span.end - 1]);
    }
    if next != report.submitted.len() {
        return Err("coverage");
    }
    Ok(stage)
}

fn replacement(base: &ListOpLog, author: &str, parents: &[usize]) -> Vec<u8> {
    let mut stage = base.clone();
    let agent = stage.get_or_create_agent_id(author);
    stage.add_operations_at(
        agent,
        parents,
        &[
            TextOperation::new_delete(2..3),
            TextOperation::new_insert(2, "Z"),
        ],
    );
    stage.encode_from(
        &options().user_data(b"expected epoch"),
        base.local_frontier_ref(),
    )
}

#[test]
fn both_original_third_writer_updates_pass_on_both_opposite_order_bases() {
    let seed = inserted("seed", "abc");
    let mut updates = Vec::new();
    for (name, position, content) in [("left", 1, "X"), ("right", 2, "Y")] {
        let mut stage = seed.clone();
        let agent = stage.get_or_create_agent_id(name);
        stage.add_insert_at(agent, seed.local_frontier_ref(), position, content);
        updates.push(stage.encode_from(&options(), seed.local_frontier_ref()));
    }
    let mut bases = Vec::new();
    for order in [[0, 1], [1, 0]] {
        let mut base = seed.clone();
        for index in order {
            base.decode_and_add(&updates[index]).unwrap();
        }
        assert_eq!(text(&base), "aXbYc");
        bases.push(base);
    }
    let originals: Vec<_> = bases
        .iter()
        .map(|base| replacement(base, "third", base.local_frontier_ref()))
        .collect();
    assert_ne!(originals[0], originals[1]); // Old whole-byte guard false-rejects.
    for base in &bases {
        for data in &originals {
            let before = base.encode(&options());
            let accepted = inspect_policy(base, data, b"expected epoch").unwrap();
            assert_eq!(text(&accepted), "aXZYc");
            assert_eq!(base.encode(&options()), before);
            assert_eq!(
                inspect_policy(base, data, b"wrong epoch").err(),
                Some("metadata")
            );
            let (stage, report) = base.decode_new_strict(data).unwrap();
            assert_eq!(report.submitted.len(), 2);
            assert_eq!(
                report.user_data.as_deref(),
                Some(b"expected epoch".as_slice())
            );
            // Reconstruction on this local cut can differ, but neither original
            // needs re-encoding or weakening input inspection to be admitted.
            assert_eq!(text(&stage), text(&accepted));
        }
    }
}

#[test]
fn exact_puyu_461_byte_same_cut_witness_accepts_both_original_artifacts() {
    // Independently computed with Python hashlib.sha256 + urlsafe_b64encode,
    // without padding. Seed domain: b"puyudb/diamond-fit/seed/i02\0" + [7; 32].
    // Writer domain: b"puyudb/diamond-fit/writer/i02\0" + [key; 32]
    // + [1; 16] branch + [1; 16] writer epoch + [7; 32] content epoch.
    const SEED: &str = "s:Cn_Mblxo0Bft4I3VrhacvkETIiWPrscRjxiQUfGRM2g";
    const THIRD: &str = "w:nsrXCNn8XQ-x3AwxUYn_QF4IeWHLXz3fzysrMHcQE3c";
    const LEFT: &str = "w:2fDnlACJvVYwKjTwpgRKGvTz8rtVh6MINzQS_6BeGwg";
    const RIGHT: &str = "w:O53AMlGvFmJoR8MdxMEq0YbqzCq1ew5sWIOZkobsP5g";
    for name in [SEED, THIRD, LEFT, RIGHT] {
        assert_eq!(name.len(), 45);
    }
    let context = |key: u8| {
        format!(
            "puyudb/diamond-fit/i02 key={} branch={} writer_epoch={} epoch={} seq=1 ordinal=0",
            format!("{key:02x}").repeat(32),
            "01".repeat(16),
            "01".repeat(16),
            "07".repeat(32),
        )
    };
    let metadata = context(1);
    assert_eq!(metadata.len(), 264);
    let seed = inserted(SEED, "abc");
    let mut concurrent = Vec::new();
    for (name, key, position, content) in [(LEFT, 2, 1, "X"), (RIGHT, 3, 2, "Y")] {
        let mut stage = seed.clone();
        let agent = stage.get_or_create_agent_id(name);
        let mut view = stage.checkout_tip();
        view.insert(&mut stage, agent, position, content);
        concurrent.push(stage.encode_from(
            &options().user_data(context(key).as_bytes()),
            seed.local_frontier_ref(),
        ));
    }
    let mut bases = Vec::new();
    let mut originals = Vec::new();
    for order in [[0, 1], [1, 0]] {
        let mut base = seed.clone();
        for index in order {
            base.decode_and_add(&concurrent[index]).unwrap();
        }
        assert_eq!(text(&base), "aXbYc");
        assert_eq!(base.len(), 5);
        let mut authored = base.clone();
        let agent = authored.get_or_create_agent_id(THIRD);
        let mut view = authored.checkout_tip();
        view.delete(&mut authored, agent, 2..3);
        view.insert(&mut authored, agent, 2, "Z");
        originals.push(authored.encode_from(
            &options().user_data(metadata.as_bytes()),
            base.local_frontier_ref(),
        ));
        bases.push(base);
    }
    let cut = remote_frontier(&bases[0], bases[0].local_frontier_ref());
    let mut expected_cut = vec![(LEFT.to_owned(), 0), (RIGHT.to_owned(), 0)];
    expected_cut.sort();
    assert_eq!(cut, expected_cut);
    assert_eq!(
        remote_frontier(&bases[1], bases[1].local_frontier_ref()),
        cut
    );
    assert_ne!(originals[0], originals[1]);
    for (index, original) in originals.iter().enumerate() {
        assert_eq!(original.len(), 461);
        // The preserved assessment's exact differing-name offsets, not just an
        // analogous short-name fixture. Remaining differences are only the CRC.
        let names = if index == 0 {
            [LEFT, RIGHT]
        } else {
            [RIGHT, LEFT]
        };
        assert_eq!(&original[16..61], THIRD.as_bytes());
        assert_eq!(&original[62..107], names[0].as_bytes());
        assert_eq!(&original[108..153], names[1].as_bytes());
        for base in &bases {
            let before = base.encode(&options());
            let (stage, report) = base.decode_new_strict(original).unwrap();
            assert_eq!(base.encode(&options()), before);
            assert_eq!(report.user_data.as_deref(), Some(metadata.as_bytes()));
            assert_eq!(report.agent_names, [THIRD, names[0], names[1]]);
            assert_eq!(report.doc_id, None);
            assert_eq!(report.start_content, None);
            assert_eq!(remote_frontier(&stage, report.start_frontier.as_ref()), cut);
            assert_eq!(report.submitted, (5..7).into());
            assert_eq!(report.submitted.end, stage.len());
            assert_eq!(
                remote_frontier(&stage, report.end_frontier.as_ref()),
                [(THIRD.to_owned(), 1)]
            );
            let submitted: Vec<_> = stage.iter_full_range(report.submitted).collect();
            assert_eq!(submitted.len(), 2);
            for (sequence, (op, history, ids)) in submitted.iter().enumerate() {
                assert_eq!(ids.0, THIRD);
                assert_eq!(ids.1, (sequence..sequence + 1).into());
                assert_eq!(history.span, (5 + sequence..6 + sequence).into());
                let expected_parents = if sequence == 0 {
                    cut.clone()
                } else {
                    vec![(THIRD.to_owned(), 0)]
                };
                assert_eq!(
                    remote_frontier(&stage, history.parents.as_ref()),
                    expected_parents
                );
                assert_eq!(op.start(), 2);
                assert_eq!(op.len(), 1);
                assert!(op.loc.fwd);
                assert_eq!(
                    op.kind,
                    if sequence == 0 {
                        ListOpKind::Del
                    } else {
                        ListOpKind::Ins
                    }
                );
                assert_eq!(
                    op.content_as_str(),
                    if sequence == 0 { None } else { Some("Z") }
                );
            }
            assert_eq!(text(&stage), "aXZYc");
        }
    }
    for (offset, (&left, &right)) in originals[0].iter().zip(&originals[1]).enumerate() {
        if !(62..107).contains(&offset) && !(108..153).contains(&offset) && offset < 457 {
            assert_eq!(left, right);
        }
    }
}

#[test]
fn foreign_ownership_and_wrong_original_parents_are_externally_rejected() {
    let base = inserted("seed", "abc");
    let foreign = replacement(&base, "foreign", base.local_frontier_ref());
    assert!(base.decode_new_strict(&foreign).is_ok()); // Generic parsing is not authorization.
    assert_eq!(
        inspect_policy(&base, &foreign, b"expected epoch").err(),
        Some("declarations")
    );
    let impersonated_seed = replacement(&base, "seed", base.local_frontier_ref());
    assert!(base.decode_new_strict(&impersonated_seed).is_ok());
    assert_eq!(
        inspect_policy(&base, &impersonated_seed, b"expected epoch").err(),
        Some("ownership")
    );
    let root_authored = replacement(&base, "third", &[]);
    assert!(base.decode_new_strict(&root_authored).is_ok());
    assert_eq!(
        inspect_policy(&base, &root_authored, b"expected epoch").err(),
        Some("parents")
    );
}

#[test]
fn original_start_content_and_document_id_are_external_policy_not_silent_data() {
    let base = inserted("seed", "abc");
    let update = replacement(&base, "third", base.local_frontier_ref());
    let (stage, _) = base.decode_new_strict(&update).unwrap();
    let full_start = stage.encode_from(
        &options()
            .store_start_branch_content(true)
            .user_data(b"expected epoch"),
        base.local_frontier_ref(),
    );
    assert!(inspect_policy(&base, &full_start, b"expected epoch").is_ok());
    let end = full_start.len() - 6;
    let mut prefix = full_start[..end].to_vec();
    let offset = prefix.windows(3).position(|w| w == b"abc").unwrap();
    prefix[offset + 1] = b'q';
    let altered = seal(prefix);
    let (_, report) = base.decode_new_strict(&altered).unwrap();
    assert_eq!(report.start_content.as_deref(), Some("aqc"));
    assert_eq!(
        inspect_policy(&base, &altered, b"expected epoch").err(),
        Some("start content")
    );

    let mut prefix = update[..update.len() - 6].to_vec();
    assert_eq!(prefix[9], 1); // Short FileInfo fixture.
    let doc_id = chunk(2, &[4, b'd', b'o', b'c']);
    prefix[10] += u8::try_from(doc_id.len()).unwrap();
    prefix.splice(11..11, doc_id);
    let named = seal(prefix);
    let (_, report) = base.decode_new_strict(&named).unwrap();
    assert_eq!(report.doc_id.as_deref(), Some("doc"));
    assert_eq!(
        inspect_policy(&base, &named, b"expected epoch").err(),
        Some("metadata")
    );
}

#[test]
fn late_crc_and_missing_base_errors_do_not_mutate_the_caller() {
    let base = inserted("seed", "abc");
    let valid = replacement(&base, "third", base.local_frontier_ref());
    let mut invalid = valid.clone();
    *invalid.last_mut().unwrap() ^= 1;
    assert_eq!(assert_rejected(&base, &invalid), ParseError::ChecksumFailed);
    assert_eq!(text(&base), "abc");
    assert_eq!(
        assert_rejected(&ListOpLog::new(), &valid),
        ParseError::BaseVersionUnknown
    );
    let (stage, report) = base.decode_new_strict(&valid).unwrap();
    let mut default = base.clone();
    let frontier = default.decode_and_add(&valid).unwrap();
    assert_eq!(report.end_frontier, frontier);
    assert_eq!(stage.encode(&options()), default.encode(&options()));
}
