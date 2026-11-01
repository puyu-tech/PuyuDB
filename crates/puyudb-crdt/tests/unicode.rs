// SPDX-License-Identifier: Apache-2.0

//! Position units and native anchor evidence.

mod common;

use automerge::{Automerge, Cursor, CursorPosition, MoveCursor, ReadDoc, TextEncoding};
use common::context;
use puyudb_crdt::experiment::{edit_at, seed, utf16_offset_to_byte};

#[test]
fn byte_ranges_reject_split_scalars_reversed_ranges_and_out_of_bounds() {
    let seeded = seed(&[7; 32], "A😀e\u{301}\r\n").unwrap();
    let before = seeded.document.get_heads();
    for (start, end) in [(2, 5), (1, 2), (4, 3), (0, 100)] {
        assert!(edit_at(&seeded.document, &before, &context(1, 1), start..end, "X").is_err());
    }
    let good = edit_at(&seeded.document, &before, &context(1, 1), 1..5, "Z").unwrap();
    let mut document = seeded.document.clone();
    document.apply_changes([good]).unwrap();
    assert_eq!(document.text(&seeded.text).unwrap(), "AZe\u{301}\r\n");
    assert_eq!(seeded.document.get_heads(), before);
}

#[test]
fn experimental_scalar_profile_rejects_an_implicitly_different_native_unit() {
    let seeded = seed(&[7; 32], "A😀bc").unwrap();
    let mut bytes = Automerge::new_with_encoding(TextEncoding::Utf8CodeUnit);
    bytes.apply_changes([seeded.change]).unwrap();
    assert!(edit_at(&bytes, &bytes.get_heads(), &context(1, 1), 5..5, "X").is_err());
}

#[test]
fn utf16_conversion_rejects_a_split_surrogate_and_preserves_combining_scalars() {
    let text = "A😀e\u{301}";
    for (units, byte) in [(0, 0), (1, 1), (3, 5), (4, 6), (5, 8)] {
        assert_eq!(utf16_offset_to_byte(text, units).unwrap(), byte);
    }
    assert!(utf16_offset_to_byte(text, 2).is_err());
    assert!(utf16_offset_to_byte(text, 6).is_err());
    assert_ne!(
        seed(&[7; 32], "é").unwrap().change.hash(),
        seed(&[7; 32], "e\u{301}").unwrap().change.hash()
    );
}

#[test]
fn native_cursors_survive_insert_delete_serialization_and_historical_reload() {
    let seeded = seed(&[7; 32], "abcdef").unwrap();
    let base = seeded.document.get_heads();
    let cursor = seeded
        .document
        .get_cursor_moving(&seeded.text, 3, None, MoveCursor::After)
        .unwrap();
    let cursor = Cursor::try_from(cursor.to_bytes().as_slice()).unwrap();
    let change = edit_at(&seeded.document, &base, &context(1, 1), 2..5, "").unwrap();
    let mut document = seeded.document.clone();
    document.apply_changes([change]).unwrap();
    let loaded = Automerge::load(&document.save()).unwrap();
    assert_eq!(loaded.text(&seeded.text).unwrap(), "abf");
    assert_eq!(
        loaded
            .get_cursor_position(&seeded.text, &cursor, None)
            .unwrap(),
        2
    );
    assert_eq!(
        loaded
            .get_cursor_position(&seeded.text, &cursor, Some(&base))
            .unwrap(),
        3
    );
    let inserted = edit_at(&loaded, &loaded.get_heads(), &context(2, 1), 0..0, "😀").unwrap();
    document = loaded;
    document.apply_changes([inserted]).unwrap();
    assert_eq!(
        document
            .get_cursor_position(&seeded.text, &cursor, None)
            .unwrap(),
        3
    );
    let other_epoch = seed(&[8; 32], "abcdef").unwrap();
    assert!(
        other_epoch
            .document
            .get_cursor_position(&seeded.text, &cursor, None)
            .is_err()
    );
}

#[test]
fn both_anchor_affinities_and_new_anchor_availability_match_every_small_frontier() {
    let seeded = seed(&[7; 32], "abcdef").unwrap();
    let base = seeded.document.get_heads();
    let changes = [
        edit_at(&seeded.document, &base, &context(1, 1), 2..2, "XY").unwrap(),
        edit_at(&seeded.document, &base, &context(2, 1), 2..4, "").unwrap(),
        edit_at(&seeded.document, &base, &context(3, 1), 3..5, "Z").unwrap(),
    ];
    let mut cursors = Vec::new();
    for position in 0..6 {
        for affinity in [MoveCursor::Before, MoveCursor::After] {
            let cursor = seeded
                .document
                .get_cursor_moving(&seeded.text, position, None, affinity)
                .unwrap();
            cursors.push(Cursor::try_from(cursor.to_bytes().as_slice()).unwrap());
        }
    }
    for position in [CursorPosition::Start, CursorPosition::End] {
        cursors.push(
            seeded
                .document
                .get_cursor(&seeded.text, position, None)
                .unwrap(),
        );
    }
    let mut with_insert = seeded.document.clone();
    with_insert.apply_changes([changes[0].clone()]).unwrap();
    let new_cursor = with_insert.get_cursor(&seeded.text, 2, None).unwrap();
    let mut full = seeded.document.clone();
    full.apply_changes(changes.clone()).unwrap();
    for mask in 0..8 {
        let mut exact = seeded.document.clone();
        exact
            .apply_changes(
                changes
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, change)| change.clone()),
            )
            .unwrap();
        let heads = exact.get_heads();
        let text = exact.text(&seeded.text).unwrap();
        for bytes in [full.save(), full.save_nocompress()] {
            let loaded = Automerge::load(&bytes).unwrap();
            let fork = loaded.fork_at(&heads).unwrap();
            assert_eq!(loaded.text_at(&seeded.text, &heads).unwrap(), text);
            for cursor in &cursors {
                let expected = exact
                    .get_cursor_position(&seeded.text, cursor, None)
                    .unwrap();
                assert_eq!(
                    loaded
                        .get_cursor_position(&seeded.text, cursor, Some(&heads))
                        .unwrap(),
                    expected
                );
                assert_eq!(
                    fork.get_cursor_position(&seeded.text, cursor, None)
                        .unwrap(),
                    expected
                );
            }
            if mask & 1 == 0 {
                assert!(
                    loaded
                        .get_cursor_position(&seeded.text, &new_cursor, Some(&heads))
                        .is_err()
                );
            } else {
                let expected = exact
                    .get_cursor_position(&seeded.text, &new_cursor, None)
                    .unwrap();
                assert_eq!(
                    loaded
                        .get_cursor_position(&seeded.text, &new_cursor, Some(&heads))
                        .unwrap(),
                    expected
                );
                assert_eq!(
                    fork.get_cursor_position(&seeded.text, &new_cursor, None)
                        .unwrap(),
                    expected
                );
            }
        }
    }
}
