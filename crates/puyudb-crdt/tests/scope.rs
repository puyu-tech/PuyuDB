// SPDX-License-Identifier: Apache-2.0

//! Experimental operation-scope inspection, including hidden non-text operations.

mod common;

use automerge::{
    Change, ChangeHash, ExpandedChange, ScalarValue,
    legacy::{ElementId, Key, ObjectId, OpId, OpType},
    transaction::Transactable,
};
use common::context;
use puyudb_crdt::experiment::{edit_at, inspect, seed};
use std::num::NonZeroU64;

type Mutation = (&'static str, fn(&mut ExpandedChange));

fn mutate(change: &Change, mutation: impl FnOnce(&mut ExpandedChange)) -> Vec<u8> {
    let mut expanded = change.decode();
    mutation(&mut expanded);
    Change::from(expanded).raw_bytes().to_vec()
}

#[test]
fn staged_primitive_edits_share_one_native_change_and_earlier_operation_scope() {
    use automerge::ReadDoc;
    let seeded = seed(&[7; 32], "A😀bc").unwrap();
    let context = context(1, 1);
    let mut staged = seeded.document.clone().with_actor(context.actor());
    let mut tx = staged.transaction();
    tx.splice_text(&seeded.text, 1, 0, "Z").unwrap();
    tx.splice_text(&seeded.text, 1, 1, "Y").unwrap();
    tx.commit_with(context.options());
    assert_eq!(staged.text(&seeded.text).unwrap(), "AY😀bc");
    let changes = staged.get_changes(&seeded.document.get_heads());
    assert_eq!(changes.len(), 1);
    inspect(&seeded.document, &context, changes[0].raw_bytes()).unwrap();
}

#[test]
fn rejects_non_text_operations_even_if_the_visible_text_could_look_valid() {
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
    let mutations: Vec<Mutation> = vec![
        ("root target", |c| c.operations[0].obj = ObjectId::Root),
        ("foreign object", |c| {
            c.operations[0].obj = ObjectId::Id(OpId::new(99, &c.actor_id))
        }),
        ("extra map", |c| {
            c.operations[0].action = OpType::Make(automerge::ObjType::Map)
        }),
        ("counter", |c| c.operations[0].action = OpType::Increment(1)),
        ("numeric value", |c| {
            c.operations[0].action = OpType::Put(ScalarValue::Int(1))
        }),
        ("rich text mark", |c| {
            c.operations[0].action = OpType::MarkEnd(false)
        }),
        ("map key", |c| {
            c.operations[0].key = Key::Map("hidden".into())
        }),
        ("non-inserting update", |c| c.operations[0].insert = false),
        ("multi-scalar value", |c| {
            c.operations[0].action = OpType::Put(ScalarValue::Str("XY".into()))
        }),
        ("foreign anchor", |c| {
            c.operations[0].key = Key::Seq(ElementId::Id(OpId::new(999, &c.actor_id)))
        }),
    ];
    for (name, mutation) in mutations {
        assert!(
            inspect(&seeded.document, &context, &mutate(&valid, mutation)).is_err(),
            "{name}"
        );
    }
    assert_eq!(
        seeded.document.text_encoding(),
        automerge::TextEncoding::UnicodeCodePoint
    );
}

#[test]
fn rejects_actor_metadata_sequence_counter_and_dependency_scope_mismatches() {
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
    let mutations: Vec<Mutation> = vec![
        ("actor", |c| {
            c.actor_id = automerge::ActorId::from(vec![42; 33])
        }),
        ("sequence", |c| c.seq = 2),
        ("counter", |c| c.start_op = NonZeroU64::new(999).unwrap()),
        ("timestamp", |c| c.time = 1),
        ("message", |c| {
            c.message = Some("wrong outer context".into())
        }),
        ("extra bytes", |c| c.extra_bytes = vec![1]),
        ("missing base", |c| c.deps.clear()),
        ("unknown dependency", |c| {
            c.deps = vec![ChangeHash([99; 32])]
        }),
    ];
    for (name, mutation) in mutations {
        assert!(
            inspect(&seeded.document, &context, &mutate(&valid, mutation)).is_err(),
            "{name}"
        );
    }
    let mut other_epoch = context.clone();
    other_epoch.content_epoch = [8; 32];
    assert!(inspect(&seeded.document, &other_epoch, valid.raw_bytes()).is_err());
}

#[test]
fn rejects_wrong_delete_predecessors_and_deleting_unobserved_elements() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let context = context(1, 1);
    let valid = edit_at(
        &seeded.document,
        &seeded.document.get_heads(),
        &context,
        1..2,
        "",
    )
    .unwrap();
    assert!(
        inspect(
            &seeded.document,
            &context,
            &mutate(&valid, |c| {
                c.operations[0].pred = Vec::new().into();
            })
        )
        .is_err()
    );
    assert!(
        inspect(
            &seeded.document,
            &context,
            &mutate(&valid, |c| {
                c.operations[0].key = Key::Seq(ElementId::Head);
            })
        )
        .is_err()
    );
}

#[test]
fn parse_success_does_not_replace_checksum_or_exact_native_byte_validation() {
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
    let mut bad_checksum = valid.raw_bytes().to_vec();
    bad_checksum[4] ^= 1;
    assert!(
        Change::from_bytes(bad_checksum.clone()).is_ok(),
        "pin-specific observation: this native parsing path does not verify checksum"
    );
    assert!(inspect(&seeded.document, &context, &bad_checksum).is_err());
    let mut compressed_tag = valid.raw_bytes().to_vec();
    compressed_tag[8] = 2;
    assert!(inspect(&seeded.document, &context, &compressed_tag).is_err());
    for raw in [&[][..], &valid.raw_bytes()[..8], &[0; 20][..]] {
        assert!(inspect(&seeded.document, &context, raw).is_err());
    }
}

#[test]
fn native_queued_changes_are_not_mistaken_for_applied_history() {
    let seeded = seed(&[7; 32], "abc").unwrap();
    let a = edit_at(
        &seeded.document,
        &seeded.document.get_heads(),
        &context(1, 1),
        0..0,
        "A",
    )
    .unwrap();
    let mut branch = seeded.document.clone();
    branch.apply_changes([a.clone()]).unwrap();
    let a2 = edit_at(&branch, &branch.get_heads(), &context(1, 2), 0..0, "B").unwrap();
    assert!(inspect(&seeded.document, &context(1, 2), a2.raw_bytes()).is_err());
    let mut receiver = seeded.document.clone();
    receiver.apply_changes([a2.clone()]).unwrap();
    assert_eq!(receiver.get_heads(), seeded.document.get_heads());
    receiver.apply_changes([a]).unwrap();
    assert_eq!(receiver.get_heads(), [a2.hash()]);
}
