// SPDX-License-Identifier: Apache-2.0

//! Shared workload semantics, not production engine adapters.

#[path = "../examples/support/candidates.rs"]
mod candidates;

use candidates::{AutomergeText, DiamondText, Edit, LoroText, NativeText, YrsText};
use std::ops::Range;

fn check_history<T: NativeText>() {
    let mut source = T::seed("A😀e\u{301}\r\n").unwrap();
    let base = source.cut();
    source
        .splice(&Edit::at("A😀e\u{301}\r\n", 1..5, "日本").unwrap())
        .unwrap();
    assert_eq!(source.text().unwrap(), "A日本e\u{301}\r\n", "{}", T::NAME);
    let replaced = source.cut();
    source
        .splice(&Edit::at("A日本e\u{301}\r\n", 1..7, "").unwrap())
        .unwrap();
    assert_eq!(source.text().unwrap(), "Ae\u{301}\r\n", "{}", T::NAME);
    let bytes = source.save().unwrap();
    for (cut, expected) in [(&base, "A😀e\u{301}\r\n"), (&replaced, "A日本e\u{301}\r\n")] {
        let mut restored = T::load(&bytes).unwrap();
        assert_eq!(restored.text().unwrap(), "Ae\u{301}\r\n", "{}", T::NAME);
        assert_eq!(restored.text_at(cut).unwrap(), expected, "{}", T::NAME);
    }
}

#[test]
fn all_candidates_reconstruct_deleted_unicode_from_full_native_history() {
    check_history::<AutomergeText>();
    check_history::<YrsText>();
    check_history::<LoroText>();
    check_history::<DiamondText>();
}

fn check_delete_only<T: NativeText>() {
    let mut source = T::seed("abcdef").unwrap();
    let base = source.cut();
    source
        .splice(&Edit::at("abcdef", 1..3, "").unwrap())
        .unwrap();
    let first_delete = source.cut();
    source.splice(&Edit::at("adef", 1..3, "").unwrap()).unwrap();
    let bytes = source.save().unwrap();
    for (cut, expected) in [(&base, "abcdef"), (&first_delete, "adef")] {
        let mut restored = T::load(&bytes).unwrap();
        assert_eq!(restored.text().unwrap(), "af", "{}", T::NAME);
        assert_eq!(restored.text_at(cut).unwrap(), expected, "{}", T::NAME);
    }
}

#[test]
fn delete_only_cuts_retain_deletion_boundaries_not_just_insert_clocks() {
    check_delete_only::<AutomergeText>();
    check_delete_only::<YrsText>();
    check_delete_only::<LoroText>();
    check_delete_only::<DiamondText>();
}

#[test]
fn shared_byte_translation_rejects_split_scalars_and_invalid_ranges() {
    let base = "A😀b";
    for range in [2..5, 1..3, 0..7, Range { start: 5, end: 1 }] {
        assert!(Edit::at(base, range, "x").is_err());
    }
    assert!(Edit::at(base, 1..1, "").is_err());
    assert!(Edit::at(base, 1..5, "日本").is_ok());
}

fn check_distinct_starts<T: NativeText>() {
    let mut candidate = T::seed("A😀b").unwrap();
    let before = candidate.cut();
    // Byte start 5, scalar start 2 and UTF-16 start 3 must not be conflated.
    let mut expected = "A😀b".to_owned();
    expected.replace_range(5..6, "日本");
    candidate
        .splice(&Edit::at("A😀b", 5..6, "日本").unwrap())
        .unwrap();
    assert_eq!(candidate.text().unwrap(), expected, "{}", T::NAME);
    let mut loaded = T::load(&candidate.save().unwrap()).unwrap();
    assert_eq!(loaded.text().unwrap(), expected, "{}", T::NAME);
    assert_eq!(loaded.text_at(&before).unwrap(), "A😀b", "{}", T::NAME);
}

#[test]
fn edit_starts_after_supplementary_characters_use_candidate_native_units() {
    check_distinct_starts::<AutomergeText>();
    check_distinct_starts::<YrsText>();
    check_distinct_starts::<LoroText>();
    check_distinct_starts::<DiamondText>();
}
