// SPDX-License-Identifier: Apache-2.0

//! Independently authored text adaptations of published Automerge regressions
//! and a fresh paper-informed backward-insertion diagnostic.
//! Exact source/license pins and derivation are in docs/validation/I02-sources.md.

mod common;

use automerge::{Automerge, Change, ReadDoc};
use common::context;
use puyudb_crdt::experiment::{EditContext, edit_at, seed};

fn ordered_writers() -> [EditContext; 2] {
    let mut writers = [context(1, 1), context(2, 1)];
    writers.sort_by_key(EditContext::actor);
    assert!(writers[0].actor() < writers[1].actor());
    writers
}

fn apply(document: &Automerge, changes: impl IntoIterator<Item = Change> + Clone) -> Automerge {
    let mut result = document.clone();
    result.apply_changes(changes).unwrap();
    result
}

#[test]
fn upstream_derived_forward_runs_keep_the_pinned_native_order_and_contiguity() {
    let seeded = seed(&[7; 32], "").unwrap();
    let writers = ordered_writers();
    let base = seeded.document.get_heads();
    let a = edit_at(&seeded.document, &base, &writers[0], 0..0, "xy").unwrap();
    let b = edit_at(&seeded.document, &base, &writers[1], 0..0, "UV").unwrap();
    assert_eq!(a.start_op(), b.start_op());
    for order in [[a.clone(), b.clone()], [b.clone(), a.clone()]] {
        let full = apply(&seeded.document, order);
        let loaded = Automerge::load(&full.save()).unwrap();
        assert_eq!(loaded.text(&seeded.text).unwrap(), "UVxy");
        assert_eq!(loaded.text_at(&seeded.text, &[a.hash()]).unwrap(), "xy");
        assert_eq!(loaded.text_at(&seeded.text, &[b.hash()]).unwrap(), "UV");
        assert_eq!(loaded.text_at(&seeded.text, &base).unwrap(), "");
    }
}

#[test]
fn upstream_derived_deleted_origin_does_not_erase_an_unseen_descendant() {
    let seeded = seed(&[7; 32], "hjk").unwrap();
    let base = seeded.document.get_heads();
    let deletion = edit_at(&seeded.document, &base, &context(1, 1), 1..3, "").unwrap();
    let insertion = edit_at(&seeded.document, &base, &context(2, 1), 2..2, "Q").unwrap();
    for order in [
        [deletion.clone(), insertion.clone()],
        [insertion.clone(), deletion.clone()],
    ] {
        let full = apply(&seeded.document, order);
        let loaded = Automerge::load(&full.save()).unwrap();
        assert_eq!(loaded.text(&seeded.text).unwrap(), "hQ");
        assert_eq!(
            loaded.text_at(&seeded.text, &[deletion.hash()]).unwrap(),
            "h"
        );
        assert_eq!(
            loaded.text_at(&seeded.text, &[insertion.hash()]).unwrap(),
            "hjQk"
        );
        assert_eq!(loaded.text_at(&seeded.text, &base).unwrap(), "hjk");
    }
}

#[test]
fn paper_informed_backward_insertions_converge_but_are_not_maximally_noninterleaving() {
    let seeded = seed(&[7; 32], "").unwrap();
    let writers = ordered_writers();
    let base = seeded.document.get_heads();
    let a1 = edit_at(&seeded.document, &base, &writers[0], 0..0, "p").unwrap();
    let b1 = edit_at(&seeded.document, &base, &writers[1], 0..0, "q").unwrap();
    assert_eq!(a1.start_op(), b1.start_op());
    let a = apply(&seeded.document, [a1.clone()]);
    let b = apply(&seeded.document, [b1.clone()]);
    let mut a_context = writers[0].clone();
    let mut b_context = writers[1].clone();
    a_context.outer_sequence = 2;
    b_context.outer_sequence = 2;
    let a2 = edit_at(&a, &a.get_heads(), &a_context, 0..0, "m").unwrap();
    let b2 = edit_at(&b, &b.get_heads(), &b_context, 0..0, "n").unwrap();
    let changes = [a1, a2, b1, b2];
    for order in [
        [0, 1, 2, 3],
        [0, 2, 1, 3],
        [0, 2, 3, 1],
        [2, 0, 1, 3],
        [2, 0, 3, 1],
        [2, 3, 0, 1],
    ] {
        let mut replica = seeded.document.clone();
        for index in order {
            replica
                .apply_changes([changes[index].clone(), changes[index].clone()])
                .unwrap();
        }
        let loaded = Automerge::load(&replica.save()).unwrap();
        let text = loaded.text(&seeded.text).unwrap();
        assert_eq!(text, "nmqp");
        assert_ne!(text, "mpnq");
        assert_ne!(text, "nqmp");
        assert_eq!(
            loaded.text_at(&seeded.text, &[changes[1].hash()]).unwrap(),
            "mp"
        );
        assert_eq!(
            loaded.text_at(&seeded.text, &[changes[3].hash()]).unwrap(),
            "nq"
        );
        assert_eq!(loaded.text_at(&seeded.text, &base).unwrap(), "");
    }
}
