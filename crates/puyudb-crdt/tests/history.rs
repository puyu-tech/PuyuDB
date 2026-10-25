// SPDX-License-Identifier: Apache-2.0

//! Native-history evidence, not PuyuDB storage or replication tests.

mod common;

use automerge::{Automerge, Change, ChangeHash, ReadDoc};
use common::context;
use proptest::{prelude::*, test_runner::RngSeed};
use puyudb_crdt::experiment::{edit_at, inspect, seed};

fn schedules(changes: &[Change], known: &[ChangeHash]) -> Vec<Vec<usize>> {
    fn visit(
        changes: &[Change],
        known: &mut Vec<ChangeHash>,
        path: &mut Vec<usize>,
        output: &mut Vec<Vec<usize>>,
    ) {
        if path.len() == changes.len() {
            output.push(path.clone());
            return;
        }
        for (index, change) in changes.iter().enumerate() {
            if !path.contains(&index) && change.deps().iter().all(|d| known.contains(d)) {
                known.push(change.hash());
                path.push(index);
                visit(changes, known, path, output);
                path.pop();
                known.pop();
            }
        }
    }
    let mut output = Vec::new();
    visit(changes, &mut known.to_vec(), &mut Vec::new(), &mut output);
    output
}

#[test]
fn concurrent_first_edits_converge_in_every_order_with_duplicates_and_reload() {
    let seeded = seed(&[7; 32], "abcdef").unwrap();
    let changes: Vec<_> = ["A", "β", "😀"]
        .iter()
        .enumerate()
        .map(|(index, text)| {
            edit_at(
                &seeded.document,
                &seeded.document.get_heads(),
                &context(index as u8 + 1, 1),
                2..2,
                text,
            )
            .unwrap()
        })
        .collect();
    let orders = schedules(&changes, &seeded.document.get_heads());
    assert_eq!(orders.len(), 6);
    let mut expected = None;
    for order in orders {
        let mut replica = seed(&[7; 32], "abcdef").unwrap().document;
        for index in order {
            let change = &changes[index];
            replica
                .apply_changes([change.clone(), change.clone()])
                .unwrap();
            replica = Automerge::load(&replica.save()).unwrap();
            assert_eq!(
                replica
                    .text_at(&seeded.text, &seeded.document.get_heads())
                    .unwrap(),
                "abcdef"
            );
        }
        let mut heads = replica.get_heads();
        heads.sort();
        let result = (replica.text(&seeded.text).unwrap(), heads);
        assert_eq!(result.0.chars().count(), 9);
        if let Some(expected) = &expected {
            assert_eq!(&result, expected);
        } else {
            expected = Some(result);
        }
    }
}

#[test]
fn observed_delete_does_not_erase_an_unseen_concurrent_insertion() {
    let seeded = seed(&[7; 32], "abcdef").unwrap();
    let base = seeded.document.get_heads();
    let insert = edit_at(&seeded.document, &base, &context(1, 1), 2..2, "X").unwrap();
    let delete = edit_at(&seeded.document, &base, &context(2, 1), 1..3, "").unwrap();
    for changes in [[insert.clone(), delete.clone()], [delete, insert]] {
        let mut replica = seeded.document.clone();
        replica.apply_changes(changes).unwrap();
        assert_eq!(replica.text(&seeded.text).unwrap(), "aXdef");
        assert_eq!(replica.text_at(&seeded.text, &base).unwrap(), "abcdef");
    }
}

#[test]
fn stale_byte_ranges_are_interpreted_only_in_the_declared_intent_base() {
    let seeded = seed(&[7; 32], "abcd").unwrap();
    let base = seeded.document.get_heads();
    let newer = edit_at(&seeded.document, &base, &context(1, 1), 0..0, "W").unwrap();
    let mut current = seeded.document.clone();
    current.apply_changes([newer]).unwrap();
    let stale = edit_at(&current, &base, &context(2, 1), 1..2, "Z").unwrap();
    assert_eq!(stale.deps(), base);
    current.apply_changes([stale]).unwrap();
    assert_eq!(current.text(&seeded.text).unwrap(), "WaZcd");
}

#[test]
fn every_small_frontier_survives_native_full_and_incremental_exports() {
    let seeded = seed(&[7; 32], "abcdef").unwrap();
    let base = seeded.document.get_heads();
    let changes: Vec<_> = (1..=3)
        .map(|writer| {
            edit_at(
                &seeded.document,
                &base,
                &context(writer, 1),
                2..2,
                &writer.to_string(),
            )
            .unwrap()
        })
        .collect();
    let mut full = seeded.document.clone();
    full.apply_changes(changes.clone()).unwrap();
    for mask in 0..8 {
        let mut cut = seeded.document.clone();
        cut.apply_changes(
            changes
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, c)| c.clone()),
        )
        .unwrap();
        let expected = cut.text(&seeded.text).unwrap();
        for snapshot in [full.save(), full.save_nocompress()] {
            let loaded = Automerge::load(&snapshot).unwrap();
            assert_eq!(
                loaded.text_at(&seeded.text, &cut.get_heads()).unwrap(),
                expected
            );
            assert_eq!(
                loaded.fork_at(&cut.get_heads()).unwrap().get_heads(),
                cut.get_heads()
            );
            for original in full.get_changes(&[]) {
                assert_eq!(
                    loaded
                        .get_change_by_hash(&original.hash())
                        .unwrap()
                        .raw_bytes(),
                    original.raw_bytes()
                );
            }
        }
    }
    let mut replay = seed(&[7; 32], "abcdef").unwrap().document;
    replay.load_incremental(&full.save_after(&base)).unwrap();
    assert_eq!(replay.get_heads(), full.get_heads());
    assert_eq!(
        replay.text(&seeded.text).unwrap(),
        full.text(&seeded.text).unwrap()
    );
}

#[test]
fn history_forks_need_fresh_native_writer_epochs_not_actor_substitution() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let base = seeded.document.get_heads();
    let first = edit_at(&seeded.document, &base, &context(1, 1), 0..0, "A").unwrap();
    let unsafe_reuse = edit_at(&seeded.document, &base, &context(1, 2), 0..0, "B").unwrap();
    assert_eq!(first.actor_id(), unsafe_reuse.actor_id());
    assert_eq!(first.seq(), unsafe_reuse.seq());
    assert_ne!(first.hash(), unsafe_reuse.hash());
    let mut fresh = context(1, 2);
    fresh.writer_epoch = [99; 16];
    let safe = edit_at(&seeded.document, &base, &fresh, 0..0, "B").unwrap();
    assert_ne!(safe.actor_id(), first.actor_id());
    let mut replica = seeded.document.clone();
    replica.apply_changes([first, safe]).unwrap();
    assert!(replica.text(&seeded.text).unwrap().contains("abc"));
}

#[test]
fn native_change_sequences_are_gapless_but_operation_counters_follow_the_base() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let mut current = seeded.document.clone();
    let a = edit_at(&current, &current.get_heads(), &context(1, 1), 0..0, "A").unwrap();
    current.apply_changes([a.clone()]).unwrap();
    let b = edit_at(&current, &current.get_heads(), &context(2, 1), 0..0, "BBBB").unwrap();
    current.apply_changes([b.clone()]).unwrap();
    let a2 = edit_at(&current, &current.get_heads(), &context(1, 2), 0..0, "C").unwrap();
    assert_eq!(a2.seq(), a.seq() + 1);
    assert!(a2.start_op().get() > a.max_op() + 1);
    assert_eq!(a2.start_op().get(), b.max_op() + 1);
    inspect(&current, &context(1, 2), a2.raw_bytes()).unwrap();
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64, rng_seed: RngSeed::Fixed(0x50555955), failure_persistence: None,
        ..ProptestConfig::default()
    })]
    #[test]
    fn generated_unicode_histories_converge_across_all_topological_orders(
        choices in prop::collection::vec((0usize..12, any::<bool>(), 0usize..4), 6)
    ) {
        let seeded = seed(&[7; 32], "a😀e\u{301}\r\n").unwrap();
        let mut changes = Vec::new();
        for writer in 1..=3 {
            let mut local = seeded.document.clone();
            for round in 0..2 {
                let text = local.text(&seeded.text).unwrap();
                let boundaries: Vec<_> = text.char_indices().map(|(i, _)| i)
                    .chain(std::iter::once(text.len())).collect();
                let (position, delete, scalar) = choices[(writer as usize - 1) * 2 + round];
                let index = position % boundaries.len();
                let start = boundaries[index];
                let end = if delete && index + 1 < boundaries.len() { boundaries[index+1] } else { start };
                let inserted = ["x", "😀", "\u{301}", "\n"][scalar];
                let change = edit_at(&local, &local.get_heads(), &context(writer, round as u64 + 1),
                    start..end, inserted).unwrap();
                local.apply_changes([change.clone()]).unwrap();
                changes.push(change);
            }
        }
        let orders = schedules(&changes, &seeded.document.get_heads());
        prop_assert_eq!(orders.len(), 90);
        let mut expected = None;
        for order in orders {
            let mut replica = seeded.document.clone();
            for index in order {
                replica.apply_changes([changes[index].clone(), changes[index].clone()]).unwrap();
            }
            let loaded = Automerge::load(&replica.save()).unwrap();
            let mut heads = loaded.get_heads();
            heads.sort();
            let actual = (loaded.text(&seeded.text).unwrap(), heads);
            if let Some(expected) = &expected {
                prop_assert_eq!(&actual, expected);
            } else {
                expected = Some(actual);
            }
        }
    }
}
