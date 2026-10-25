// SPDX-License-Identifier: Apache-2.0

//! Independently constructed pinned native-format negatives, not a public codec.
//! Layout source: Automerge 0.12.0 storage/change.rs, chunk.rs and column specs.

mod common;

use automerge::{Author, Change, ReadDoc, transaction::Transactable};
use common::context;
use puyudb_crdt::experiment::{Error, edit_at, inspect, seed};
use sha2::{Digest, Sha256};
use std::num::NonZeroU64;

fn integer(bytes: &[u8], offset: &mut usize) -> u64 {
    let mut value = 0;
    for shift in (0..70).step_by(7) {
        let byte = bytes[*offset];
        *offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return value;
        }
    }
    panic!("test fixture integer exceeds u64");
}

fn encoded(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        bytes.push(byte | if value == 0 { 0 } else { 0x80 });
        if value == 0 {
            return bytes;
        }
    }
}

fn chunk(body: &[u8]) -> Vec<u8> {
    let mut hashed = vec![1]; // Uncompressed native change.
    hashed.extend(encoded(body.len() as u64));
    hashed.extend(body);
    let hash = Sha256::digest(&hashed);
    let mut bytes = vec![0x85, 0x6f, 0x4a, 0x83];
    bytes.extend_from_slice(&hash[..4]);
    bytes.extend(hashed);
    bytes
}

fn body(raw: &[u8]) -> &[u8] {
    let mut offset = 9;
    let len = integer(raw, &mut offset) as usize;
    assert_eq!(offset + len, raw.len());
    &raw[offset..]
}

fn column_offset(body: &[u8]) -> usize {
    let mut offset = 0;
    let deps = integer(body, &mut offset) as usize;
    offset += deps * 32;
    let actor = integer(body, &mut offset) as usize;
    offset += actor;
    for _ in 0..3 {
        integer(body, &mut offset);
    } // Sequence, start op, zero timestamp.
    let message = integer(body, &mut offset) as usize;
    offset += message;
    let actors = integer(body, &mut offset);
    for _ in 0..actors {
        let len = integer(body, &mut offset) as usize;
        offset += len;
    }
    offset
}

#[test]
fn zero_and_out_of_width_native_start_counters_are_rejected_before_application() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let context = context(1, 1);
    let valid = edit_at(
        &seeded.document,
        &seeded.document.get_heads(),
        &context,
        1..1,
        "X",
    )
    .unwrap();
    let original = body(valid.raw_bytes());
    let mut start = 0;
    let deps = integer(original, &mut start) as usize;
    start += deps * 32;
    let actor = integer(original, &mut start) as usize;
    start += actor;
    integer(original, &mut start);
    let mut end = start;
    integer(original, &mut end);
    for value in [0, u64::from(u32::MAX) + 1, u64::MAX] {
        let mut changed = original.to_vec();
        changed.splice(start..end, encoded(value));
        let raw = chunk(&changed);
        assert!(
            matches!(
                inspect(&seeded.document, &context, &raw),
                Err(Error::Decode(_))
            ),
            "counter={value}"
        );
    }
}

#[test]
fn malformed_native_footer_cannot_hide_behind_absent_author_metadata() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let context = context(1, 1);
    let valid = edit_at(
        &seeded.document,
        &seeded.document.get_heads(),
        &context,
        1..1,
        "X",
    )
    .unwrap();
    let mut expanded = valid.decode();
    expanded.extra_bytes = vec![0xff, 0x00, 0xfe, 0x01];
    let native = Change::from(expanded);
    assert!(native.author().is_none());
    assert!(!native.extra_bytes().is_empty());
    assert!(inspect(&seeded.document, &context, native.raw_bytes()).is_err());
}

#[test]
fn valid_checksum_unknown_column_is_not_lost_silently_by_expanded_inspection() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let context = context(1, 1);
    let valid = edit_at(
        &seeded.document,
        &seeded.document.get_heads(),
        &context,
        1..1,
        "X",
    )
    .unwrap();
    let original = body(valid.raw_bytes());
    let count_offset = column_offset(original);
    let mut offset = count_offset;
    let count = integer(original, &mut offset);
    let descriptor_start = offset;
    let mut column_bytes = 0;
    for _ in 0..count {
        integer(original, &mut offset);
        column_bytes += integer(original, &mut offset) as usize;
    }
    let mut modified = original[..count_offset].to_vec();
    modified.extend(encoded(count + 1));
    modified.extend_from_slice(&original[descriptor_start..offset]);
    modified.extend(encoded((127 << 4) | 2)); // Unassigned high column ID, integer type.
    modified.extend(encoded(2));
    modified.extend_from_slice(&original[offset..offset + column_bytes]);
    modified.extend([1, 42]); // One RLE integer value in the extra column.
    modified.extend_from_slice(&original[offset + column_bytes..]);
    let raw = chunk(&modified);
    assert_eq!(&raw[4..8], &Sha256::digest(&raw[8..])[..4]);
    let parsed =
        Change::from_bytes(raw.clone()).expect("unknown column remains natively decodable");
    assert_eq!(parsed.decode().operations, valid.decode().operations);
    assert_eq!(parsed.actor_id(), valid.actor_id());
    assert_ne!(parsed.hash(), valid.hash());
    assert!(matches!(
        inspect(&seeded.document, &context, &raw),
        Err(Error::Invalid(
            "native bytes do not roundtrip through pinned encoder"
        ))
    ));
}

#[test]
fn native_author_footer_is_not_confused_with_authenticated_writer_or_outer_authorship() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let context = context(1, 1);
    let mut authoring = seeded
        .document
        .clone()
        .with_author(Some(Author::from(vec![1, 2, 3])))
        .with_actor(context.actor());
    let mut tx = authoring.transaction();
    tx.splice_text(&seeded.text, 1, 0, "X").unwrap();
    tx.commit_with(context.options());
    let change = authoring.get_last_local_change().unwrap();
    assert_eq!(change.actor_id(), &context.actor());
    assert!(change.author().is_some());
    assert!(!change.extra_bytes().is_empty());
    assert!(inspect(&seeded.document, &context, change.raw_bytes()).is_err());
}

#[test]
fn counter_range_crossing_u32_and_oversized_inputs_are_rejected_without_application() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let context = context(1, 1);
    let valid = edit_at(
        &seeded.document,
        &seeded.document.get_heads(),
        &context,
        1..1,
        "XY",
    )
    .unwrap();
    let mut expanded = valid.decode();
    expanded.start_op = NonZeroU64::new(u64::from(u32::MAX)).unwrap();
    let crossing = Change::from(expanded);
    assert!(matches!(
        inspect(&seeded.document, &context, crossing.raw_bytes()),
        Err(Error::Invalid(
            "native operation counter exceeds checked profile"
        ))
    ));
    let mut oversized = vec![0; 8 * 1024 * 1024 + 1];
    oversized[8] = 1;
    assert!(matches!(
        inspect(&seeded.document, &context, &oversized),
        Err(Error::Invalid(
            "expected a bounded uncompressed native change"
        ))
    ));
    assert!(matches!(
        edit_at(
            &seeded.document,
            &seeded.document.get_heads(),
            &context,
            0..0,
            &"x".repeat(1024 * 1024 + 1)
        ),
        Err(Error::Invalid(
            "empty edit or insertion exceeds experimental limit"
        ))
    ));
    assert_eq!(seeded.document.text(&seeded.text).unwrap(), "abc");
}

#[test]
fn known_elsewhere_is_not_available_in_the_declared_intent_cut() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let cut = seeded.document.get_heads();
    let unseen = edit_at(&seeded.document, &cut, &context(1, 1), 0..0, "A").unwrap();
    let original = edit_at(&seeded.document, &cut, &context(2, 1), 1..1, "B").unwrap();
    let mut current = seeded.document.clone();
    current.apply_changes([unseen.clone()]).unwrap();
    assert!(current.get_change_by_hash(&unseen.hash()).is_some());
    let mut expanded = original.decode();
    expanded.deps = vec![unseen.hash()];
    let invalid = Change::from(expanded);
    let exact = current.fork_at(&cut).unwrap();
    assert!(matches!(
        inspect(&exact, &context(2, 1), invalid.raw_bytes()),
        Err(Error::Invalid(
            "native dependencies do not represent the exact base cut"
        ))
    ));
}

#[test]
fn already_deleted_element_cannot_be_reauthored_as_an_observed_delete() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let cut = seeded.document.get_heads();
    let first = edit_at(&seeded.document, &cut, &context(1, 1), 1..2, "").unwrap();
    let again = edit_at(&seeded.document, &cut, &context(2, 1), 1..2, "").unwrap();
    let mut current = seeded.document.clone();
    current.apply_changes([first.clone()]).unwrap();
    let mut expanded = again.decode();
    expanded.deps = current.get_heads();
    expanded.start_op = NonZeroU64::new(first.max_op() + 1).unwrap();
    let invalid = Change::from(expanded);
    assert!(matches!(
        inspect(&current, &context(2, 1), invalid.raw_bytes()),
        Err(Error::Invalid(
            "deletion must target a visible text element"
        ))
    ));
}
