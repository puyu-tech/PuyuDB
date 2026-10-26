// SPDX-License-Identifier: Apache-2.0
//! Bounded native-ordering spike, not a production anchor or wire format.
//! Original test logic; four historical native byte vectors are quoted with
//! attribution in the compatibility test, not claimed as newly authored data.

use diamond_types::{
    DTRange, Frontier, LV,
    causalgraph::agent_assignment::remote_ids::{RemoteVersion, RemoteVersionOwned},
    list::{
        ListOpLog,
        encoding::EncodeOptions,
        operation::{ListOpKind, TextOperation},
    },
};

type Cut = Vec<RemoteVersionOwned>;
type Scalar = (RemoteVersionOwned, char);

#[test]
fn resumed_agent_and_native_span_crossing_assignments_keep_all_remote_ids() {
    let mut log = ListOpLog::new();
    let a = log.get_or_create_agent_id("a");
    log.add_insert(a, 0, "a");
    let mut other = ListOpLog::load_from(&save(&log)).unwrap();
    let b = other.get_or_create_agent_id("b");
    other.add_insert(b, 1, "b");
    log.decode_and_add(&save(&other)).unwrap();
    // The original local handle resumes only after importing b's operation.
    log.add_insert(a, 2, "c");
    let expected = [(id("a", 0), true), (id("b", 0), true), (id("a", 1), true)];
    for log in [&log, &ListOpLog::load_from(&save(&log)).unwrap()] {
        let cut = tip(log);
        assert_eq!(ordered_ids(log, &cut), expected);
        assert_eq!(log.checkout_tip().content().to_string(), "abc");
        let spans = log.insertion_spans_at(local_cut(log, &cut).as_ref());
        assert_eq!(spans, [((0..3).into(), true)]);
        // One native span crosses agent assignments: this converter deliberately
        // returns only its first portion. Never extrapolate that agent to 0..3.
        let first = log
            .cg
            .agent_assignment
            .local_to_remote_version_span(spans[0].0);
        assert_eq!((first.0, first.1.end - first.1.start), ("a", 1));
        assert_eq!(
            Anchor::Item(id("a", 1), Side::Before).resolve(log, &cut),
            Some(2)
        );
        assert_eq!(
            Anchor::Item(id("a", 1), Side::After).resolve(log, &cut),
            Some(3)
        );
    }
}

const ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

fn id(agent: &str, seq: usize) -> RemoteVersionOwned {
    (agent, seq).into()
}

fn local_cut(log: &ListOpLog, cut: &Cut) -> Frontier {
    log.cg
        .agent_assignment
        .try_remote_to_local_frontier(cut.iter())
        .unwrap()
}

fn remote_cut(log: &ListOpLog, cut: &[LV]) -> Cut {
    log.cg
        .agent_assignment
        .local_to_remote_frontier_owned(cut)
        .into_iter()
        .collect()
}

fn tip(log: &ListOpLog) -> Cut {
    remote_cut(log, log.local_frontier_ref())
}

fn save(log: &ListOpLog) -> Vec<u8> {
    log.encode(&EncodeOptions::full().compress_content(false))
}

fn seed(text: &str) -> ListOpLog {
    let mut log = ListOpLog::new();
    if !text.is_empty() {
        let agent = log.get_or_create_agent_id("seed");
        log.add_insert_at(agent, &[], 0, text);
    }
    log
}

struct Artifact {
    bytes: Vec<u8>,
    cut: Cut,
}

fn author(base: &[u8], agent: &str, ops: &[TextOperation]) -> Artifact {
    let mut log = ListOpLog::load_from(base).unwrap();
    let parents = log.local_frontier();
    let agent = log.get_or_create_agent_id(agent);
    let head = log.add_operations_at(agent, parents.as_ref(), ops);
    Artifact {
        bytes: save(&log),
        cut: remote_cut(&log, &[head]),
    }
}

fn remote_range(log: &ListOpLog, range: DTRange) -> Vec<RemoteVersionOwned> {
    (range.start..range.end)
        .map(|lv| {
            log.cg
                .agent_assignment
                .local_to_remote_version(lv)
                .to_owned()
        })
        .collect()
}

// Independent public-patch projection checks visible IDs, never tombstone order.
fn project(log: &ListOpLog, cut: &[LV]) -> Vec<Scalar> {
    let mut visible = Vec::new();
    for (range, patch) in log.iter_xf_operations_from(&[], cut) {
        let Some(op) = patch else { continue };
        match op.kind {
            ListOpKind::Ins => {
                let chars = op.content_as_str().unwrap().chars();
                let mut inserted: Vec<_> =
                    remote_range(log, range).into_iter().zip(chars).collect();
                if !op.loc.fwd {
                    inserted.reverse();
                }
                visible.splice(op.start()..op.start(), inserted);
            }
            ListOpKind::Del => {
                visible.drain(op.start()..op.end());
            }
        }
    }
    assert_eq!(
        visible.iter().map(|(_, ch)| ch).collect::<String>(),
        log.checkout(cut).content().to_string()
    );
    visible
}

fn ordered_ids(log: &ListOpLog, cut: &Cut) -> Vec<(RemoteVersionOwned, bool)> {
    let frontier = local_cut(log, cut);
    log.insertion_spans_at(frontier.as_ref())
        .into_iter()
        .flat_map(|(ids, visible)| {
            remote_range(log, ids)
                .into_iter()
                .map(move |id| (id, visible))
        })
        .collect()
}

fn check_order(log: &ListOpLog, cut: &Cut) -> Vec<(RemoteVersionOwned, bool)> {
    let ordered = ordered_ids(log, cut);
    let frontier = local_cut(log, cut);
    assert_eq!(
        ordered
            .iter()
            .filter(|(_, visible)| *visible)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>(),
        project(log, frontier.as_ref())
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        "visible IDs at {cut:?}"
    );
    // Check the seam includes exactly the insertion IDs in this causal closure,
    // not only the visible IDs, and never includes deletion operation IDs.
    let mut expected = Vec::new();
    for (op, _, remote) in log.iter_full() {
        if op.kind == ListOpKind::Ins {
            for seq in remote.1.start..remote.1.end {
                let lv = log
                    .cg
                    .agent_assignment
                    .remote_to_local_version(RemoteVersion(remote.0, seq));
                if log
                    .cg
                    .graph
                    .frontier_contains_frontier(frontier.as_ref(), &[lv])
                {
                    expected.push(id(remote.0, seq));
                }
            }
        }
    }
    let sort_ids = |ids: &mut Vec<RemoteVersionOwned>| {
        ids.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    };
    let mut actual: Vec<_> = ordered.iter().map(|(id, _)| id.clone()).collect();
    sort_ids(&mut actual);
    sort_ids(&mut expected);
    assert_eq!(actual, expected, "all insertion IDs at {cut:?}");
    ordered
}

// The disproven numeric transform, specialized to Before(seed:0), Right.
fn old_numeric_cursor(log: &ListOpLog) -> usize {
    let lv = log
        .cg
        .agent_assignment
        .remote_to_local_version(RemoteVersion("seed", 0));
    let mut pos = 0;
    for (_, patch) in log.iter_xf_operations_from(&[lv], log.local_frontier_ref()) {
        let Some(op) = patch else { continue };
        match op.kind {
            ListOpKind::Ins => {
                if op.start() <= pos {
                    pos += op.end() - op.start();
                }
            }
            ListOpKind::Del => {
                if pos > op.start() {
                    pos -= pos.min(op.end()) - op.start();
                }
            }
        }
    }
    pos
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Affinity {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    Before,
    After,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Anchor {
    Start,
    End,
    Item(RemoteVersionOwned, Side),
}

impl Anchor {
    fn create(log: &ListOpLog, cut: &Cut, pos: usize, affinity: Affinity) -> Self {
        let frontier = local_cut(log, cut);
        let visible: Vec<_> = log
            .insertion_spans_at(frontier.as_ref())
            .into_iter()
            .filter(|(_, visible)| *visible)
            .flat_map(|(ids, _)| remote_range(log, ids))
            .collect();
        assert!(pos <= visible.len());
        match affinity {
            Affinity::Left if pos == 0 => Self::Start,
            Affinity::Left => Self::Item(visible[pos - 1].clone(), Side::After),
            Affinity::Right if pos == visible.len() => Self::End,
            Affinity::Right => Self::Item(visible[pos].clone(), Side::Before),
        }
    }

    fn resolve(&self, log: &ListOpLog, cut: &Cut) -> Option<usize> {
        let frontier = local_cut(log, cut);
        let spans = log.insertion_spans_at(frontier.as_ref());
        let (primary, side) = match self {
            Self::Start => return Some(0),
            Self::End => {
                return Some(
                    spans
                        .iter()
                        .filter(|(_, visible)| *visible)
                        .map(|(ids, _)| ids.end - ids.start)
                        .sum(),
                );
            }
            Self::Item(primary, side) => (primary, side),
        };
        let lv = log
            .cg
            .agent_assignment
            .try_remote_to_local_version(primary.into())
            .ok()?;
        let mut pos = 0;
        for (ids, visible) in spans {
            if (ids.start..ids.end).contains(&lv) {
                return Some(
                    pos + if visible {
                        lv - ids.start + usize::from(*side == Side::After)
                    } else {
                        0
                    },
                );
            }
            if visible {
                pos += ids.end - ids.start;
            }
        }
        None // Unknown, unborn at this cut, or not an insertion ID.
    }
}

#[test]
fn deleted_start_cursor_resolves_to_literal_gap_one() {
    let original = seed("ab");
    let base = tip(&original);
    let bytes = save(&original);
    let left = author(&bytes, "left", &[TextOperation::new_insert(0, "😀")]);
    let right = author(&bytes, "right", &[TextOperation::new_insert(2, "e\u{301}")]);
    let deleted = author(&bytes, "delete", &[TextOperation::new_delete(0..2)]);
    let artifacts = [&left, &right, &deleted];
    let mut old_positions = Vec::new();
    for order in ORDERS {
        let mut log = ListOpLog::load_from(&bytes).unwrap();
        for index in order {
            log.decode_and_add(&artifacts[index].bytes).unwrap();
        }
        old_positions.push(old_numeric_cursor(&log));
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        for log in [&log, &loaded] {
            let unchanged = save(log);
            assert_eq!(
                project(log, log.local_frontier_ref()),
                [
                    (id("left", 0), '😀'),
                    (id("right", 0), 'e'),
                    (id("right", 1), '\u{301}')
                ]
            );
            assert_eq!(
                check_order(log, &tip(log)),
                [
                    (id("left", 0), true),
                    (id("seed", 0), false),
                    (id("seed", 1), false),
                    (id("right", 0), true),
                    (id("right", 1), true)
                ]
            );
            for (offset, affinity, intended) in [
                (0, Affinity::Left, 0),
                (0, Affinity::Right, 1),
                (1, Affinity::Left, 1),
                (1, Affinity::Right, 1),
                (2, Affinity::Left, 1),
                (2, Affinity::Right, 3),
            ] {
                let anchor = Anchor::create(&original, &base, offset, affinity);
                assert_eq!(anchor.resolve(log, &tip(log)), Some(intended));
                assert_eq!(anchor.resolve(log, &base), Some(offset));
            }
            let anchor = Anchor::create(&original, &base, 0, Affinity::Right);
            assert_eq!(anchor, Anchor::Item(id("seed", 0), Side::Before));
            // The suffix is strictly to the right of this deleted identity.
            assert_eq!(anchor.resolve(log, &tip(log)), Some(1));
            assert_eq!(anchor.resolve(log, &vec![]), None);
            for cut in [&base, &left.cut, &right.cut, &deleted.cut] {
                check_order(log, cut);
            }
            assert_eq!(save(log), unchanged, "queries must not mutate the oplog");
        }
    }
    assert_eq!(old_positions[0], 3);
    assert_eq!(old_positions[5], 1);
}

#[test]
fn historical_sibling_cuts_missing_births_and_root_edges_are_explicit() {
    let original = seed("abcd");
    let base = tip(&original);
    let bytes = save(&original);
    let unrelated = author(&bytes, "unrelated", &[TextOperation::new_insert(4, "!")]);
    let deleted = author(&bytes, "delete", &[TextOperation::new_delete(1..3)]);
    let inserted = author(&bytes, "insert", &[TextOperation::new_insert(2, "😀")]);
    let creation = ListOpLog::load_from(&unrelated.bytes).unwrap();
    let artifacts = [&unrelated, &deleted, &inserted];
    for order in ORDERS {
        let mut log = ListOpLog::load_from(&bytes).unwrap();
        for index in order {
            log.decode_and_add(&artifacts[index].bytes).unwrap();
        }
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        for log in [&log, &loaded] {
            for affinity in [Affinity::Left, Affinity::Right] {
                let anchor = Anchor::create(&creation, &unrelated.cut, 2, affinity);
                assert_eq!(anchor, Anchor::create(&original, &base, 2, affinity));
                assert_eq!(anchor.resolve(log, &deleted.cut), Some(1));
                assert_eq!(
                    anchor.resolve(log, &tip(log)),
                    Some(if affinity == Affinity::Left { 1 } else { 2 })
                );
            }
            for side in [Side::Before, Side::After] {
                let anchor = Anchor::Item(id("insert", 0), side);
                assert_eq!(anchor.resolve(log, &vec![]), None);
                assert_eq!(anchor.resolve(log, &base), None);
                assert_eq!(anchor.resolve(&original, &base), None);
                assert_eq!(
                    anchor.resolve(log, &inserted.cut),
                    Some(2 + usize::from(side == Side::After))
                );
                assert_eq!(
                    Anchor::Item(id("unknown", 0), side).resolve(log, &tip(log)),
                    None
                );
                assert_eq!(
                    Anchor::Item(id("seed", 99), side).resolve(log, &tip(log)),
                    None
                );
                assert_eq!(
                    Anchor::Item(id("delete", 0), side).resolve(log, &tip(log)),
                    None
                );
            }
            // Root has no insertion birth requirement, including when created
            // at a later empty historical cut, not just at the empty oplog.
            let empty_cut = vec![];
            assert_eq!(
                Anchor::create(log, &empty_cut, 0, Affinity::Left),
                Anchor::Start
            );
            assert_eq!(
                Anchor::create(log, &empty_cut, 0, Affinity::Right),
                Anchor::End
            );
            assert_eq!(Anchor::Start.resolve(log, &tip(log)), Some(0));
            assert_eq!(Anchor::End.resolve(log, &tip(log)), Some(4));
            assert_eq!(Anchor::End.resolve(log, &empty_cut), Some(0));
            for cut in [
                &base,
                &unrelated.cut,
                &deleted.cut,
                &inserted.cut,
                &empty_cut,
            ] {
                let visible = project(log, local_cut(log, cut).as_ref());
                for pos in 0..=visible.len() {
                    for affinity in [Affinity::Left, Affinity::Right] {
                        assert_eq!(
                            Anchor::create(log, cut, pos, affinity).resolve(log, cut),
                            Some(pos)
                        );
                    }
                }
            }
        }
    }
    let all_deleted = author(&bytes, "all", &[TextOperation::new_delete(0..4)]);
    let log = ListOpLog::load_from(&all_deleted.bytes).unwrap();
    assert_eq!(
        Anchor::create(&log, &all_deleted.cut, 0, Affinity::Left),
        Anchor::Start
    );
    assert_eq!(
        Anchor::create(&log, &all_deleted.cut, 0, Affinity::Right),
        Anchor::End
    );
    assert_eq!(Anchor::End.resolve(&original, &base), Some(4));
}

#[test]
fn repeated_unicode_overlapping_reverse_deletes_and_later_edits_keep_identity() {
    let original = seed("a😀a\u{301}a😀a");
    let base = tip(&original);
    let bytes = save(&original);
    let a = author(&bytes, "a", &[TextOperation::new_insert(3, "😀a")]);
    let b = author(&bytes, "b", &[TextOperation::new_insert(3, "a😀")]);
    let mut reverse = TextOperation::new_delete(2..6);
    reverse.loc.fwd = false;
    let d = author(
        &bytes,
        "delete",
        &[reverse, TextOperation::new_delete(1..2)],
    );
    let artifacts = [&a, &b, &d];
    for order in ORDERS {
        let mut log = ListOpLog::load_from(&bytes).unwrap();
        for index in order {
            log.decode_and_add(&artifacts[index].bytes).unwrap();
        }
        assert_eq!(
            project(&log, log.local_frontier_ref()),
            [
                (id("seed", 0), 'a'),
                (id("a", 0), '😀'),
                (id("a", 1), 'a'),
                (id("b", 0), 'a'),
                (id("b", 1), '😀'),
                (id("seed", 6), 'a')
            ]
        );
        let merged = tip(&log);
        let later = author(
            &save(&log),
            "later",
            &[
                TextOperation::new_delete(1..5),
                TextOperation::new_insert(1, "a😀"),
            ],
        );
        log.decode_and_add(&later.bytes).unwrap();
        let overlapping = author(&bytes, "overlap", &[TextOperation::new_delete(1..4)]);
        log.decode_and_add(&overlapping.bytes).unwrap();
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        for log in [&log, &loaded] {
            assert_eq!(log.checkout_tip().content().to_string(), "aa😀a");
            for (affinity, expected) in [(Affinity::Left, 1), (Affinity::Right, 5)] {
                let anchor = Anchor::create(&original, &base, 3, affinity);
                assert_eq!(anchor.resolve(log, &merged), Some(expected));
                assert_eq!(anchor.resolve(log, &d.cut), Some(1));
            }
            for side in [Side::Before, Side::After] {
                let anchor = Anchor::Item(id("a", 0), side);
                assert_eq!(anchor.resolve(log, &base), None);
                // The later insertion is natively ordered before the deleted
                // run (its right origin), so both sides of a:0 follow it.
                // This is an identity boundary, not a saved numeric gap.
                assert_eq!(
                    anchor.resolve(log, &later.cut),
                    Some(3),
                    "native order at later cut: {:?}",
                    ordered_ids(log, &later.cut)
                );
            }
            for cut in [
                &base,
                &a.cut,
                &b.cut,
                &d.cut,
                &merged,
                &later.cut,
                &tip(log),
            ] {
                let visible = project(log, local_cut(log, cut).as_ref());
                check_order(log, cut);
                for pos in 0..=visible.len() {
                    for affinity in [Affinity::Left, Affinity::Right] {
                        assert_eq!(
                            Anchor::create(log, cut, pos, affinity).resolve(log, cut),
                            Some(pos)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn exhaustive_small_concurrent_gaps_match_literal_scalar_oracles() {
    let original = seed("a😀a\u{301}a");
    let base = tip(&original);
    let bytes = save(&original);
    let len = 5;
    let mut cases = 0;
    // 540 histories: two insert gaps and every nonempty observed seed deletion.
    // The oracle counts only surviving seed scalars and independent inserts at
    // known original gaps; it does not implement any conflict/tombstone merger.
    for a_gap in 0..=len {
        for b_gap in 0..=len {
            for start in 0..len {
                for end in start + 1..=len {
                    let a = author(&bytes, "a", &[TextOperation::new_insert(a_gap, "😀a")]);
                    let b = author(&bytes, "z", &[TextOperation::new_insert(b_gap, "a\u{301}")]);
                    let mut delete = TextOperation::new_delete(start..end);
                    delete.loc.fwd = cases % 2 == 0;
                    let d = author(&bytes, "delete", &[delete]);
                    let artifacts = [&a, &b, &d];
                    let mut reference = None;
                    for order in ORDERS {
                        let mut log = ListOpLog::load_from(&bytes).unwrap();
                        for index in order {
                            log.decode_and_add(&artifacts[index].bytes).unwrap();
                        }
                        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
                        for log in [&log, &loaded] {
                            let cut = tip(log);
                            let ordered = check_order(log, &cut);
                            if let Some(reference) = &reference {
                                assert_eq!(&ordered, reference);
                            } else {
                                reference = Some(ordered);
                            }
                            for pos in 0..=len {
                                for affinity in [Affinity::Left, Affinity::Right] {
                                    let surviving_before =
                                        (0..pos).filter(|i| !(start..end).contains(i)).count();
                                    let expected = match (pos, affinity) {
                                        (0, Affinity::Left) => 0,
                                        (p, Affinity::Right) if p == len => len - (end - start) + 4,
                                        (_, Affinity::Left) => {
                                            surviving_before
                                                + 2 * usize::from(a_gap < pos)
                                                + 2 * usize::from(b_gap < pos)
                                        }
                                        (_, Affinity::Right) => {
                                            surviving_before
                                                + 2 * usize::from(a_gap <= pos)
                                                + 2 * usize::from(b_gap <= pos)
                                        }
                                    };
                                    let anchor = Anchor::create(&original, &base, pos, affinity);
                                    assert_eq!(
                                        anchor.resolve(log, &cut),
                                        Some(expected),
                                        "gaps={a_gap},{b_gap}, delete={start}..{end}, pos={pos}, \
                                         affinity={affinity:?}, order={order:?}"
                                    );
                                }
                            }
                        }
                    }
                    cases += 1;
                }
            }
        }
    }
    assert_eq!(cases, 540);
}

fn draw(state: &mut u64, bound: usize) -> usize {
    *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
    usize::try_from(*state >> 32).unwrap() % bound
}

#[test]
fn generated_branching_histories_keep_tombstone_order_at_partial_and_joined_cuts() {
    for history in 0..24 {
        let mut random = history + 19;
        let original = seed("a😀a\u{301}");
        let mut artifacts = vec![Artifact {
            bytes: save(&original),
            cut: tip(&original),
        }];
        // Every artifact contains only its own ancestor closure. Importing
        // snapshots in reverse order must still leave genuinely concurrent
        // branches arriving differently, not one final all-history snapshot.
        for batch in 0..12 {
            let p = draw(&mut random, artifacts.len());
            let mut staged = ListOpLog::load_from(&artifacts[p].bytes).unwrap();
            if draw(&mut random, 3) == 0 {
                let q = draw(&mut random, artifacts.len());
                staged.decode_and_add(&artifacts[q].bytes).unwrap();
            }
            let len = staged.checkout_tip().content().len_chars();
            let mode = draw(&mut random, 3);
            let mut ops = Vec::new();
            let pos = if len > 0 && mode != 0 {
                let start = draw(&mut random, len);
                let end = start + 1 + draw(&mut random, (len - start).min(3));
                let mut delete = TextOperation::new_delete(start..end);
                delete.loc.fwd = draw(&mut random, 2) == 0;
                ops.push(delete);
                start
            } else {
                draw(&mut random, len + 1)
            };
            if mode != 1 || ops.is_empty() {
                let content = ["a", "😀", "a\u{301}", "😀a", "aa"][draw(&mut random, 5)];
                ops.push(TextOperation::new_insert(pos, content));
            }
            artifacts.push(author(&save(&staged), &format!("w{batch:02}"), &ops));
        }

        let mut reference = ListOpLog::load_from(&artifacts[0].bytes).unwrap();
        for artifact in &artifacts[1..] {
            reference.decode_and_add(&artifact.bytes).unwrap();
        }
        let mut cuts = vec![vec![], tip(&reference)];
        cuts.extend(artifacts.iter().map(|artifact| artifact.cut.clone()));
        // Every native atom is also a retained cut, including partial runs and
        // reverse deletions. Add joins which are not any author's saved tip.
        for lv in 0..reference.len() {
            cuts.push(remote_cut(&reference, &[lv]));
        }
        for _ in 0..4 {
            let a = local_cut(&reference, &cuts[draw(&mut random, cuts.len())]);
            let b = local_cut(&reference, &cuts[draw(&mut random, cuts.len())]);
            let joined = reference.version_union(a.as_ref(), b.as_ref());
            cuts.push(remote_cut(&reference, joined.as_ref()));
        }
        let reference_orders: Vec<_> = cuts
            .iter()
            .map(|cut| check_order(&reference, cut))
            .collect();
        let reference_texts: Vec<_> = cuts
            .iter()
            .map(|cut| {
                reference
                    .checkout(local_cut(&reference, cut).as_ref())
                    .content()
                    .to_string()
            })
            .collect();
        let mut anchors = Vec::new();
        for cut in &cuts {
            let len = project(&reference, local_cut(&reference, cut).as_ref()).len();
            for pos in 0..=len {
                for affinity in [Affinity::Left, Affinity::Right] {
                    let anchor = Anchor::create(&reference, cut, pos, affinity);
                    let at_tip = anchor.resolve(&reference, &tip(&reference));
                    anchors.push((cut, pos, affinity, anchor, at_tip));
                }
            }
        }

        let forward: Vec<_> = (1..artifacts.len()).collect();
        let mut shuffled = forward.clone();
        for i in (1..shuffled.len()).rev() {
            let j = draw(&mut random, i + 1);
            shuffled.swap(i, j);
        }
        for order in [
            forward.clone(),
            forward.iter().copied().rev().collect(),
            shuffled,
        ] {
            let mut log = ListOpLog::load_from(&artifacts[0].bytes).unwrap();
            for index in &order {
                log.decode_and_add(&artifacts[*index].bytes).unwrap();
            }
            let loaded = ListOpLog::load_from(&save(&log)).unwrap();
            for log in [&log, &loaded] {
                let unchanged = save(log);
                for (index, cut) in cuts.iter().enumerate() {
                    assert_eq!(
                        check_order(log, cut),
                        reference_orders[index],
                        "history={history}, cut={cut:?}, order={order:?}"
                    );
                    assert_eq!(
                        log.checkout(local_cut(log, cut).as_ref())
                            .content()
                            .to_string(),
                        reference_texts[index]
                    );
                }
                for (cut, pos, affinity, anchor, at_tip) in &anchors {
                    assert_eq!(Anchor::create(log, cut, *pos, *affinity), *anchor);
                    assert_eq!(anchor.resolve(log, cut), Some(*pos));
                    assert_eq!(anchor.resolve(log, &tip(log)), *at_tip);
                }
                assert_eq!(save(log), unchanged);
            }
        }
    }
}

#[test]
fn historical_native_encoding_fixtures_and_reencoding_are_unchanged_by_queries() {
    // Uncompressed upstream fixtures from src/list/encoding/tests.rs:347-389
    // at 89ae3a0, including the older explicit ROOT StartBranch chunk.
    let empty_old = [
        0x44, 0x4d, 0x4e, 0x44, 0x54, 0x59, 0x50, 0x53, 0x00, 0x01, 0x02, 0x03, 0x00, 0x0a, 0x07,
        0x0c, 0x02, 0x00, 0x00, 0x0d, 0x01, 0x04, 0x14, 0x06, 0x15, 0x00, 0x16, 0x00, 0x17, 0x00,
        0x64, 0x04, 0x6c, 0xce, 0x6b, 0x00,
    ];
    let empty_new = [
        0x44, 0x4d, 0x4e, 0x44, 0x54, 0x59, 0x50, 0x53, 0x00, 0x01, 0x02, 0x03, 0x00, 0x0a, 0x00,
        0x14, 0x06, 0x15, 0x00, 0x16, 0x00, 0x17, 0x00, 0x64, 0x04, 0x86, 0x77, 0x4d, 0x6a,
    ];
    let simple_old = [
        68, 77, 78, 68, 84, 89, 80, 83, 0, 1, 7, 3, 5, 4, 115, 101, 112, 104, 10, 7, 12, 2, 0, 0,
        13, 1, 4, 20, 32, 24, 16, 0, 13, 10, 4, 104, 105, 32, 116, 104, 101, 114, 101, 109, 25, 1,
        19, 21, 2, 2, 13, 22, 4, 65, 79, 11, 0, 23, 2, 13, 1, 100, 4, 162, 205, 138, 38,
    ];
    let simple_new = [
        68, 77, 78, 68, 84, 89, 80, 83, 0, 1, 7, 3, 5, 4, 115, 101, 112, 104, 10, 0, 20, 32, 24,
        16, 0, 13, 10, 4, 104, 105, 32, 116, 104, 101, 114, 101, 109, 25, 1, 19, 21, 2, 2, 13, 22,
        4, 65, 79, 11, 0, 23, 2, 13, 1, 100, 4, 151, 117, 95, 151,
    ];
    for bytes in [&empty_old[..], &empty_new] {
        let log = ListOpLog::load_from(bytes).unwrap();
        let before = save(&log);
        assert_eq!(log, ListOpLog::new());
        assert!(check_order(&log, &tip(&log)).is_empty());
        assert_eq!(save(&log), before);
    }
    for bytes in [&simple_old[..], &simple_new] {
        let log = ListOpLog::load_from(bytes).unwrap();
        let before = save(&log);
        assert_eq!(
            project(&log, log.local_frontier_ref()),
            [
                (id("seph", 0), 'h'),
                (id("seph", 1), 'i'),
                (id("seph", 2), ' '),
                (id("seph", 12), 'm'),
                (id("seph", 7), 'e')
            ]
        );
        check_order(&log, &tip(&log));
        for pos in 0..=5 {
            for affinity in [Affinity::Left, Affinity::Right] {
                let anchor = Anchor::create(&log, &tip(&log), pos, affinity);
                assert_eq!(anchor.resolve(&log, &tip(&log)), Some(pos));
            }
        }
        assert_eq!(save(&log), before);
    }
}
