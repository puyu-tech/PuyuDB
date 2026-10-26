// SPDX-License-Identifier: Apache-2.0

//! Bounded public-API experiment at DT 89ae3a0, not a production anchor adapter.
//!
//! Identity projection: apply ONLY native transformed patches to a visible scalar
//! vector. Insert IDs are native remote (agent name, sequence), never local LVs.
//! Reconstruct each original delete's targets at its own native parent cut; a
//! transformed `None` means "already deleted", not "no original target".
//!
//! Experimental gap policy: store ONE primary remote insertion ID and before/after
//! side. Left affinity chooses the left neighbor, after; right affinity chooses
//! the right neighbor, before. At a nonempty document edge, use the only neighbor
//! with the appropriate side, retaining the requested affinity. Empty documents
//! use a root sentinel: left stays at 0, right at the target's scalar length,
//! available at every cut, even if created after a deletion made the text empty.
//!
//! Resolve the primary's native LV afresh, then project its OWN birth frontier
//! [LV] to find its before/after scalar point. Transform that point with native
//! patches from [LV] to the target, even when the primary has been deleted.
//! Insertions exactly at the cursor stay after left affinity / before right
//! affinity; deletions collapse the cursor to the deleted interval's start.
//! The target must contain the primary's birth, otherwise return Unavailable.
//! A target containing that ID necessarily contains its birth cut: no whole
//! comment-creation frontier, unrelated tail, saved numeric fallback or local ID
//! is stored in the anchor. Complete retained history and insert content are
//! prerequisites. `iter_xf_operations_from` merges cuts, not arbitrary checkouts.
//! No tombstone ordering, conflict ordering or other CRDT merge is implemented.
//!
//! Verdict: insertion/deletion identity projection works in these fixtures. The
//! minimal birth cut also fixes the previous whole-comment-cut causal restriction:
//! the former sibling-cut limitation is now a positive regression. BUT the cursor
//! adapter is not a stable native anchor: deleting the primary can collapse
//! distinct gaps, and transformed patch traversal order can change its resolved
//! position across import orders even though text AND visible IDs converge.
//! The counterexample below preserves observed positions 3 and 1 against literal
//! intended gap 1. Interior-gap probes pass; they do not erase that counterexample.
//! General anchors remain unsupported by this adapter, not disproved for DT.
//! A maintained native remote-ID anchor resolver, or a public identity-aware
//! deleted-item/insertion-origin ordering seam, is needed for a direct remedy.
//! No actor sorting, canonical replay or tombstone CRDT workaround is attempted.
//! Replaying every delete's parent cut is deliberately unoptimized; no performance
//! claim is made. Fixtures author forward insert runs and forward/reverse deletes;
//! manually flipped reverse-insert runs are outside this experiment's profile.
//!
//! Source seams (all at the above pin): src/list/merge.rs:24-46 (patches),
//! src/list/op_iter.rs:179-193 (original ops and parents),
//! src/causalgraph/agent_assignment/remote_ids.rs:109-208 (portable IDs),
//! src/causalgraph/graph/tools.rs:176-186 (cut containment).

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

fn id(agent: &str, seq: usize) -> RemoteVersionOwned {
    (agent, seq).into()
}

fn local_cut(log: &ListOpLog, cut: &Cut) -> Frontier {
    log.cg
        .agent_assignment
        .try_remote_to_local_frontier(cut.iter())
        .expect("fixture cut must exist in this replica")
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

fn project(log: &ListOpLog, cut: &[LV]) -> Vec<Scalar> {
    let mut visible = Vec::new();
    for (range, patch) in log.iter_xf_operations_from(&[], cut) {
        let Some(op) = patch else { continue };
        match op.kind {
            ListOpKind::Ins => {
                let chars: Vec<_> = op
                    .content_as_str()
                    .expect("experiment requires retained insert content")
                    .chars()
                    .collect();
                let ids = remote_range(log, range);
                assert_eq!(ids.len(), chars.len());
                let mut inserted: Vec<_> = ids.into_iter().zip(chars).collect();
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
    // This is a consistency check, not the identity oracle used in the tests.
    assert_eq!(
        visible.iter().map(|(_, ch)| ch).collect::<String>(),
        log.checkout(cut).content().to_string()
    );
    visible
}

fn assert_projection(log: &ListOpLog, cut: &Cut, expected: &[(&str, usize, char)]) {
    let expected: Vec<_> = expected
        .iter()
        .map(|&(agent, seq, ch)| (id(agent, seq), ch))
        .collect();
    assert_eq!(project(log, local_cut(log, cut).as_ref()), expected);
}

fn deletion_targets(log: &ListOpLog) -> Vec<(RemoteVersionOwned, RemoteVersionOwned)> {
    let mut result = Vec::new();
    for (op, history, remote) in log.iter_full() {
        if op.kind != ListOpKind::Del {
            continue;
        }
        // Replaying parents delegates all conflict decisions to DT, including
        // parents in which another equal character has already been deleted.
        let parent = project(log, history.parents.as_ref());
        let mut targets: Vec<_> = parent[op.start()..op.end()]
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        if !op.loc.fwd {
            targets.reverse();
        }
        assert_eq!(targets.len(), remote.1.end - remote.1.start);
        result.extend(
            (remote.1.start..remote.1.end)
                .map(|seq| id(remote.0, seq))
                .zip(targets),
        );
    }
    result.sort_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.cmp(&b.0.1)));
    result
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

#[derive(Debug, PartialEq)]
enum Resolution {
    Position(usize),
    UnavailableAtCut,
}

#[derive(Debug, Eq, PartialEq)]
struct Anchor {
    primary: Option<(RemoteVersionOwned, Side)>,
    affinity: Affinity,
}

impl Anchor {
    fn create(log: &ListOpLog, base: &Cut, offset: usize, affinity: Affinity) -> Self {
        let visible = project(log, local_cut(log, base).as_ref());
        assert!(offset <= visible.len());
        let primary = if visible.is_empty() {
            None
        } else {
            let (index, side) = match affinity {
                Affinity::Left if offset > 0 => (offset - 1, Side::After),
                Affinity::Left => (0, Side::Before),
                Affinity::Right if offset < visible.len() => (offset, Side::Before),
                Affinity::Right => (visible.len() - 1, Side::After),
            };
            Some((visible[index].0.clone(), side))
        };
        Self { primary, affinity }
    }

    fn resolve(&self, log: &ListOpLog, target: &Cut) -> Resolution {
        let target = local_cut(log, target);
        let visible = project(log, target.as_ref());
        let Some((primary, side)) = &self.primary else {
            return Resolution::Position(match self.affinity {
                Affinity::Left => 0,
                Affinity::Right => visible.len(),
            });
        };
        let Ok(lv) = log
            .cg
            .agent_assignment
            .try_remote_to_local_version(RemoteVersion::from(primary))
        else {
            return Resolution::UnavailableAtCut;
        };
        // The graph API correctly treats the root as missing every insertion.
        if !log
            .cg
            .graph
            .frontier_contains_frontier(target.as_ref(), &[lv])
        {
            return Resolution::UnavailableAtCut;
        }
        let birth = project(log, &[lv]);
        let mut pos = birth
            .iter()
            .position(|(id, _)| id == primary)
            .expect("primary insertion must be visible at its own birth")
            + usize::from(*side == Side::After);
        for (_, patch) in log.iter_xf_operations_from(&[lv], target.as_ref()) {
            let Some(op) = patch else { continue };
            match op.kind {
                ListOpKind::Ins => {
                    if op.start() < pos
                        || (op.start() == pos && matches!(self.affinity, Affinity::Right))
                    {
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
        assert!(pos <= visible.len());
        Resolution::Position(pos)
    }
}

#[test]
fn unicode_identities_and_all_original_delete_targets_survive_orders_reload_and_history() {
    // Equal 'a', astral scalars and a combining mark cannot be matched by text.
    let original = seed("a😀a\u{301}a😀a");
    let base = tip(&original);
    let bytes = save(&original);
    let a = author(&bytes, "a", &[TextOperation::new_insert(3, "😀a")]);
    let b = author(&bytes, "b", &[TextOperation::new_insert(3, "a😀")]);
    let d = author(&bytes, "d", &[TextOperation::new_delete(1..4)]);
    let e = author(&bytes, "e", &[TextOperation::new_delete(2..6)]);
    let left = Anchor::create(&original, &base, 3, Affinity::Left);
    let right = Anchor::create(&original, &base, 3, Affinity::Right);
    let mut a_local_ids = Vec::new();

    for order in [[&a, &b, &d, &e], [&e, &d, &b, &a]] {
        let mut log = ListOpLog::load_from(&bytes).unwrap();
        for artifact in order {
            log.decode_and_add(&artifact.bytes).unwrap();
        }
        a_local_ids.push(
            log.cg
                .agent_assignment
                .try_remote_to_local_version(RemoteVersion("a", 0))
                .unwrap(),
        );
        // Keep only remote-valued anchors/cuts across a full native reload.
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        for log in [&log, &loaded] {
            assert_projection(
                log,
                &tip(log),
                &[
                    ("seed", 0, 'a'),
                    ("a", 0, '😀'),
                    ("a", 1, 'a'),
                    ("b", 0, 'a'),
                    ("b", 1, '😀'),
                    ("seed", 6, 'a'),
                ],
            );
            assert_projection(
                log,
                &base,
                &[
                    ("seed", 0, 'a'),
                    ("seed", 1, '😀'),
                    ("seed", 2, 'a'),
                    ("seed", 3, '\u{301}'),
                    ("seed", 4, 'a'),
                    ("seed", 5, '😀'),
                    ("seed", 6, 'a'),
                ],
            );
            assert_projection(
                log,
                &d.cut,
                &[
                    ("seed", 0, 'a'),
                    ("seed", 4, 'a'),
                    ("seed", 5, '😀'),
                    ("seed", 6, 'a'),
                ],
            );
            assert_projection(
                log,
                &a.cut,
                &[
                    ("seed", 0, 'a'),
                    ("seed", 1, '😀'),
                    ("seed", 2, 'a'),
                    ("a", 0, '😀'),
                    ("a", 1, 'a'),
                    ("seed", 3, '\u{301}'),
                    ("seed", 4, 'a'),
                    ("seed", 5, '😀'),
                    ("seed", 6, 'a'),
                ],
            );
            assert_projection(
                log,
                &e.cut,
                &[("seed", 0, 'a'), ("seed", 1, '😀'), ("seed", 6, 'a')],
            );
            assert_eq!(
                deletion_targets(log),
                vec![
                    (id("d", 0), id("seed", 1)),
                    (id("d", 1), id("seed", 2)),
                    (id("d", 2), id("seed", 3)),
                    (id("e", 0), id("seed", 2)),
                    (id("e", 1), id("seed", 3)),
                    (id("e", 2), id("seed", 4)),
                    (id("e", 3), id("seed", 5)),
                ]
            );
            assert!(log.iter_xf_operations().any(|(_, op)| op.is_none()));
            assert_eq!(left.resolve(log, &base), Resolution::Position(3));
            assert_eq!(right.resolve(log, &base), Resolution::Position(3));
            assert_eq!(left.resolve(log, &tip(log)), Resolution::Position(1));
            assert_eq!(right.resolve(log, &tip(log)), Resolution::Position(5));
            for affinity in [Affinity::Left, Affinity::Right] {
                let new_anchor = Anchor::create(log, &a.cut, 4, affinity);
                assert_eq!(new_anchor.resolve(log, &a.cut), Resolution::Position(4));
                assert_eq!(new_anchor.resolve(log, &base), Resolution::UnavailableAtCut);
                assert_eq!(new_anchor.resolve(log, &tip(log)), Resolution::Position(2));
            }
        }
    }
    assert_ne!(
        a_local_ids[0], a_local_ids[1],
        "orders must really remap local IDs"
    );
}

#[test]
fn deletes_authored_after_a_native_merge_target_insert_ids_not_equal_seed_scalars() {
    let original = seed("a😀a\u{301}a");
    let bytes = save(&original);
    let inserted = author(&bytes, "insert", &[TextOperation::new_insert(2, "a😀")]);
    let deleted = author(&bytes, "delete", &[TextOperation::new_delete(1..3)]);
    for order in [[&inserted, &deleted], [&deleted, &inserted]] {
        let mut merged = ListOpLog::load_from(&bytes).unwrap();
        for artifact in order {
            merged.decode_and_add(&artifact.bytes).unwrap();
        }
        assert_projection(
            &merged,
            &tip(&merged),
            &[
                ("seed", 0, 'a'),
                ("insert", 0, 'a'),
                ("insert", 1, '😀'),
                ("seed", 3, '\u{301}'),
                ("seed", 4, 'a'),
            ],
        );
        // Two sequential deletes by one writer can be coalesced by iter_full.
        let final_edit = author(
            &save(&merged),
            "later",
            &[
                TextOperation::new_delete(1..3),
                TextOperation::new_delete(1..2),
            ],
        );
        let log = ListOpLog::load_from(&final_edit.bytes).unwrap();
        assert_projection(&log, &final_edit.cut, &[("seed", 0, 'a'), ("seed", 4, 'a')]);
        assert_eq!(
            deletion_targets(&log),
            vec![
                (id("delete", 0), id("seed", 1)),
                (id("delete", 1), id("seed", 2)),
                (id("later", 0), id("insert", 0)),
                (id("later", 1), id("insert", 1)),
                (id("later", 2), id("seed", 3)),
            ]
        );
    }
}

#[test]
fn reverse_delete_runs_map_target_sequences_in_operation_order() {
    let original = seed("x😀a\u{301}a");
    let mut delete = TextOperation::new_delete(1..4);
    delete.loc.fwd = false;
    let deleted = author(&save(&original), "delete", &[delete]);
    let insert = TextOperation::new_insert(1, "😀e\u{301}");
    let inserted = author(&deleted.bytes, "insert", &[insert]);
    let log = ListOpLog::load_from(&inserted.bytes).unwrap();
    assert_projection(
        &log,
        &inserted.cut,
        &[
            ("seed", 0, 'x'),
            ("insert", 0, '😀'),
            ("insert", 1, 'e'),
            ("insert", 2, '\u{301}'),
            ("seed", 4, 'a'),
        ],
    );
    assert_eq!(
        deletion_targets(&log),
        vec![
            (id("delete", 0), id("seed", 3)),
            (id("delete", 1), id("seed", 2)),
            (id("delete", 2), id("seed", 1)),
        ]
    );
}

#[test]
fn empty_edges_and_unborn_neighbors_have_explicit_historical_semantics() {
    let empty = seed("");
    let start = Anchor::create(&empty, &tip(&empty), 0, Affinity::Left);
    let end = Anchor::create(&empty, &tip(&empty), 0, Affinity::Right);
    let inserted = author(
        &save(&empty),
        "new",
        &[TextOperation::new_insert(0, "a😀e\u{301}")],
    );
    let log = ListOpLog::load_from(&inserted.bytes).unwrap();
    assert_eq!(start.resolve(&log, &inserted.cut), Resolution::Position(0));
    assert_eq!(end.resolve(&log, &inserted.cut), Resolution::Position(4));
    for affinity in [Affinity::Left, Affinity::Right] {
        let anchor = Anchor::create(&log, &inserted.cut, 2, affinity);
        assert_eq!(anchor.resolve(&log, &vec![]), Resolution::UnavailableAtCut);
        // Also unavailable when the native log does not even know these IDs.
        assert_eq!(
            anchor.resolve(&empty, &vec![]),
            Resolution::UnavailableAtCut
        );
    }
}

#[test]
fn primary_birth_availability_depends_on_affinity_not_the_other_neighbor() {
    let original = seed("ab");
    let base = tip(&original);
    let inserted = author(
        &save(&original),
        "new",
        &[TextOperation::new_insert(1, "😀")],
    );
    let log = ListOpLog::load_from(&inserted.bytes).unwrap();
    for (offset, affinity, primary, side, older) in [
        (
            1,
            Affinity::Left,
            id("seed", 0),
            Side::After,
            Resolution::Position(1),
        ),
        (
            1,
            Affinity::Right,
            id("new", 0),
            Side::Before,
            Resolution::UnavailableAtCut,
        ),
        (
            2,
            Affinity::Left,
            id("new", 0),
            Side::After,
            Resolution::UnavailableAtCut,
        ),
        (
            2,
            Affinity::Right,
            id("seed", 1),
            Side::Before,
            Resolution::Position(1),
        ),
    ] {
        let anchor = Anchor::create(&log, &inserted.cut, offset, affinity);
        assert_eq!(anchor.primary, Some((primary, side)));
        assert_eq!(
            anchor.resolve(&log, &inserted.cut),
            Resolution::Position(offset)
        );
        // This includes a log which has never imported the new insertion.
        for historical in [&original, &log] {
            assert_eq!(anchor.resolve(historical, &base), older);
        }
    }
}

#[test]
fn birth_cut_cursor_counterexample_keeps_converged_ids_but_divergent_anchor_positions() {
    let original = seed("ab");
    let base = tip(&original);
    let bytes = save(&original);
    let left = author(&bytes, "left", &[TextOperation::new_insert(0, "😀")]);
    let right = author(&bytes, "right", &[TextOperation::new_insert(2, "e\u{301}")]);
    let deleted = author(&bytes, "delete", &[TextOperation::new_delete(0..2)]);
    let mut right_start_positions = Vec::new();
    for (order, observed_right_start) in [
        ([&left, &right, &deleted], 3),
        ([&deleted, &right, &left], 1),
    ] {
        let mut log = ListOpLog::load_from(&bytes).unwrap();
        for artifact in order {
            log.decode_and_add(&artifact.bytes).unwrap();
        }
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        for log in [&log, &loaded] {
            assert_projection(
                log,
                &tip(log),
                &[
                    ("left", 0, '😀'),
                    ("right", 0, 'e'),
                    ("right", 1, '\u{301}'),
                ],
            );
            for (offset, affinity, intended, observed) in [
                (0, Affinity::Left, 0, 0),
                (0, Affinity::Right, 1, observed_right_start),
                (2, Affinity::Left, 1, 1),
                (2, Affinity::Right, 3, 3),
            ] {
                let anchor = Anchor::create(&original, &base, offset, affinity);
                let actual = anchor.resolve(log, &tip(log));
                assert_eq!(actual, Resolution::Position(observed));
                if offset == 0 && affinity == Affinity::Right {
                    assert_eq!(anchor.primary, Some((id("seed", 0), Side::Before)));
                    right_start_positions.push(observed);
                    if observed == 3 {
                        // Literal oracle: 😀 is at the original start gap, but
                        // e+mark are strictly to its right. Right affinity must
                        // be after 😀 and BEFORE e+mark: scalar gap 1, not 3.
                        assert_eq!(intended, 1);
                        assert_ne!(actual, Resolution::Position(intended));
                        // In this native plan: insert b, insert 😀, delete ab,
                        // then insert e+mark at the now-collapsed cursor. The
                        // numeric Right-affinity transform absorbs that suffix.
                        let primary_lv = log
                            .cg
                            .agent_assignment
                            .remote_to_local_version(RemoteVersion("seed", 0));
                        let trace: Vec<_> = log
                            .iter_xf_operations_from(&[primary_lv], log.local_frontier_ref())
                            .map(|(range, patch)| {
                                let op = patch.expect("fixture has no redundant delete");
                                (remote_range(log, range), op.kind, op.start(), op.end())
                            })
                            .collect();
                        assert_eq!(
                            trace,
                            [
                                (vec![id("seed", 1)], ListOpKind::Ins, 1, 2),
                                (vec![id("left", 0)], ListOpKind::Ins, 0, 1),
                                (
                                    vec![id("delete", 0), id("delete", 1)],
                                    ListOpKind::Del,
                                    1,
                                    3
                                ),
                                (vec![id("right", 0), id("right", 1)], ListOpKind::Ins, 1, 3),
                            ]
                        );
                    } else {
                        assert_eq!(actual, Resolution::Position(intended));
                    }
                } else {
                    assert_eq!(actual, Resolution::Position(intended));
                }
                assert_eq!(anchor.resolve(log, &vec![]), Resolution::UnavailableAtCut);
            }
            // An empty view uses the root sentinel even with nonempty history.
            // Such an anchor intentionally has no comment-creation/birth bound.
            for (affinity, at_base, at_tip) in [(Affinity::Left, 0, 0), (Affinity::Right, 2, 3)] {
                let anchor = Anchor::create(log, &deleted.cut, 0, affinity);
                assert_eq!(anchor.primary, None);
                assert_eq!(anchor.resolve(log, &base), Resolution::Position(at_base));
                assert_eq!(anchor.resolve(log, &tip(log)), Resolution::Position(at_tip));
                assert_eq!(anchor.resolve(log, &vec![]), Resolution::Position(0));
            }
        }
    }
    // Each order is checked both before and after a full native reload.
    assert_eq!(right_start_positions, [3, 3, 1, 1]);
}

#[test]
fn interior_birth_cut_cursor_gaps_pass_the_bounded_two_insertion_probe() {
    let original = seed("abcd");
    let base = tip(&original);
    let bytes = save(&original);
    let left = author(&bytes, "left", &[TextOperation::new_insert(1, "X")]);
    let right = author(&bytes, "right", &[TextOperation::new_insert(3, "Y")]);
    let deleted = author(&bytes, "delete", &[TextOperation::new_delete(1..3)]);
    let mut results = Vec::new();
    for order in [[&left, &right, &deleted], [&deleted, &right, &left]] {
        let mut log = ListOpLog::load_from(&bytes).unwrap();
        for artifact in order {
            log.decode_and_add(&artifact.bytes).unwrap();
        }
        let log = ListOpLog::load_from(&save(&log)).unwrap();
        assert_projection(
            &log,
            &tip(&log),
            &[
                ("seed", 0, 'a'),
                ("left", 0, 'X'),
                ("right", 0, 'Y'),
                ("seed", 3, 'd'),
            ],
        );
        for offset in [1, 2, 3] {
            for affinity in [Affinity::Left, Affinity::Right] {
                let anchor = Anchor::create(&original, &base, offset, affinity);
                results.push(anchor.resolve(&log, &tip(&log)));
            }
        }
    }
    // Literal gaps a|b, b|c, c|d respectively bracket X, fall between X,Y,
    // and bracket Y. These are interior gaps, not document-edge policies.
    assert_eq!(
        results,
        [
            Resolution::Position(1),
            Resolution::Position(2),
            Resolution::Position(2),
            Resolution::Position(2),
            Resolution::Position(2),
            Resolution::Position(3),
            Resolution::Position(1),
            Resolution::Position(2),
            Resolution::Position(2),
            Resolution::Position(2),
            Resolution::Position(2),
            Resolution::Position(3),
        ]
    );
}

#[test]
fn primary_birth_transform_reproduces_creation_gaps_after_merges_and_edits() {
    let original = seed("a😀a\u{301}a");
    let base = tip(&original);
    let bytes = save(&original);
    let inserted = author(&bytes, "insert", &[TextOperation::new_insert(2, "a😀")]);
    let deleted = author(&bytes, "delete", &[TextOperation::new_delete(1..3)]);
    for order in [[&inserted, &deleted], [&deleted, &inserted]] {
        let mut merged = ListOpLog::load_from(&bytes).unwrap();
        for artifact in order {
            merged.decode_and_add(&artifact.bytes).unwrap();
        }
        let merged_cut = tip(&merged);
        let later = author(
            &save(&merged),
            "later",
            &[
                TextOperation::new_delete(1..3),
                TextOperation::new_insert(1, "a😀"),
            ],
        );
        let log = ListOpLog::load_from(&later.bytes).unwrap();
        for (cut, scalar_len) in [
            (&base, 5),
            (&inserted.cut, 7),
            (&deleted.cut, 3),
            (&merged_cut, 5),
            (&later.cut, 5),
        ] {
            assert_eq!(
                project(&log, local_cut(&log, cut).as_ref()).len(),
                scalar_len
            );
            for offset in 0..=scalar_len {
                for affinity in [Affinity::Left, Affinity::Right] {
                    let anchor = Anchor::create(&log, cut, offset, affinity);
                    assert_eq!(
                        anchor.resolve(&log, cut),
                        Resolution::Position(offset),
                        "creation gap {offset}, affinity {affinity:?}, cut {cut:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn primary_birth_cut_resolves_deleted_neighbors_without_unrelated_creation_history() {
    let original = seed("abcd");
    let base = tip(&original);
    let bytes = save(&original);
    let unrelated = author(&bytes, "unrelated", &[TextOperation::new_insert(4, "!")]);
    let deleted = author(&bytes, "delete", &[TextOperation::new_delete(1..3)]);
    let creation_log = ListOpLog::load_from(&unrelated.bytes).unwrap();
    for order in [[&deleted, &unrelated], [&unrelated, &deleted]] {
        let mut log = ListOpLog::load_from(&bytes).unwrap();
        for artifact in order {
            log.decode_and_add(&artifact.bytes).unwrap();
        }
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        for log in [&log, &loaded] {
            assert_projection(log, &deleted.cut, &[("seed", 0, 'a'), ("seed", 3, 'd')]);
            for affinity in [Affinity::Left, Affinity::Right] {
                let anchor = Anchor::create(&creation_log, &unrelated.cut, 2, affinity);
                // No comment-creation frontier is retained in the anchor.
                assert_eq!(anchor, Anchor::create(&original, &base, 2, affinity));
                assert_eq!(anchor.resolve(log, &base), Resolution::Position(2));
                // Literal oracle: deleting b,c leaves exactly a|d at 1.
                assert_eq!(anchor.resolve(log, &deleted.cut), Resolution::Position(1));
                assert_eq!(anchor.resolve(log, &tip(log)), Resolution::Position(1));
            }
        }
    }
}

#[test]
fn collapsed_gap_cursor_probe_does_not_assume_patch_order_preserves_anchor_intent() {
    let original = seed("abcd");
    let base = tip(&original);
    let bytes = save(&original);
    // Literal oracles distinguish X left of, AT, and right of b|c. All three
    // produce "aXd", but must NOT resolve both affinities to the same positions.
    for (insert_at, expected_left, expected_right) in [(1, 2, 2), (2, 1, 2), (3, 1, 1)] {
        for (deleter, inserter) in [("a", "z"), ("z", "a")] {
            let deleted = author(&bytes, deleter, &[TextOperation::new_delete(1..3)]);
            let inserted = author(
                &bytes,
                inserter,
                &[TextOperation::new_insert(insert_at, "X")],
            );
            for order in [[&deleted, &inserted], [&inserted, &deleted]] {
                let mut log = ListOpLog::load_from(&bytes).unwrap();
                for artifact in order {
                    log.decode_and_add(&artifact.bytes).unwrap();
                }
                let log = ListOpLog::load_from(&save(&log)).unwrap();
                assert_projection(
                    &log,
                    &tip(&log),
                    &[("seed", 0, 'a'), (inserter, 0, 'X'), ("seed", 3, 'd')],
                );
                for (affinity, expected) in [
                    (Affinity::Left, expected_left),
                    (Affinity::Right, expected_right),
                ] {
                    let anchor = Anchor::create(&original, &base, 2, affinity);
                    assert_eq!(
                        anchor.resolve(&log, &tip(&log)),
                        Resolution::Position(expected),
                        "insert_at={insert_at}, deleter={deleter}, affinity={affinity:?}"
                    );
                }
            }
        }
    }
}
