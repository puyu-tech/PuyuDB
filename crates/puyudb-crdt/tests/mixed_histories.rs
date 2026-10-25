// SPDX-License-Identifier: Apache-2.0

//! Seeded native-only delivery experiments, not the future replication protocol.

mod common;

use automerge::{Automerge, Change, ChangeHash, ObjId, ReadDoc};
use common::context;
use proptest::prelude::*;
use puyudb_crdt::experiment::{EditContext, edit_at, inspect, seed};
use rand::{RngExt, SeedableRng, rngs::StdRng, seq::SliceRandom};
use std::collections::HashSet;

const BASELINE: &str = "start😀e\u{301}\r\n日本語 end";

struct Record {
    change: Change,
    base: Vec<ChangeHash>,
    context: EditContext,
    expected: String,
}

struct History {
    records: Vec<Record>,
    expected: Automerge,
    text: ObjId,
}

fn history(seed_value: u64) -> History {
    let seeded = seed(&[7; 32], BASELINE).unwrap();
    let mut replicas: Vec<_> = (0..4).map(|_| seeded.document.clone()).collect();
    let mut rng = StdRng::seed_from_u64(seed_value);
    let mut sequences = [0; 4];
    let mut records = Vec::new();
    let mut expected = seeded.document;
    for step in 0..48 {
        let writer = step % replicas.len();
        // First isolated writers, then two partitions, then asymmetric reconnects.
        if step >= 16 && rng.random_ratio(2, 3) {
            let peer = if step < 32 {
                writer ^ 1
            } else {
                (writer + rng.random_range(1..replicas.len())) % replicas.len()
            };
            let learned = replicas[peer].get_changes(&[]);
            replicas[writer].apply_changes(learned).unwrap();
        }
        let original = replicas[writer].text(&seeded.text).unwrap();
        let boundaries: Vec<_> = original
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(original.len()))
            .collect();
        let position = rng.random_range(0..boundaries.len());
        let delete = rng.random_range(0..=3).min(boundaries.len() - position - 1);
        let range = boundaries[position]..boundaries[position + delete];
        let inserted = ["x", "😀", "e\u{301}", "\r\n", "日本"][rng.random_range(0..5)];
        let mut result = original;
        result.replace_range(range.clone(), inserted);
        sequences[writer] += 1;
        let context = context(writer as u8 + 1, sequences[writer]);
        let base = replicas[writer].get_heads();
        let change = edit_at(&replicas[writer], &base, &context, range, inserted).unwrap();
        replicas[writer].apply_changes([change.clone()]).unwrap();
        assert_eq!(replicas[writer].text(&seeded.text).unwrap(), result);
        expected.apply_changes([change.clone()]).unwrap();
        records.push(Record {
            change,
            base,
            context,
            expected: result,
        });
    }
    assert!(records.iter().any(|r| r.change.deps().len() > 1));
    History {
        records,
        expected,
        text: seeded.text,
    }
}

struct Receiver {
    visible: Automerge,
    pending: Vec<usize>,
}

impl Receiver {
    fn new() -> Self {
        Self {
            visible: seed(&[7; 32], BASELINE).unwrap().document,
            pending: Vec::new(),
        }
    }

    fn deliver(&mut self, history: &History, index: usize) {
        let record = &history.records[index];
        if self
            .visible
            .get_change_by_hash(&record.change.hash())
            .is_none()
            && !self.pending.contains(&index)
        {
            self.pending.push(index);
        }
        loop {
            let ready = self.pending.iter().position(|index| {
                history.records[*index]
                    .base
                    .iter()
                    .all(|hash| self.visible.get_change_by_hash(hash).is_some())
            });
            let Some(ready) = ready else { break };
            let record = &history.records[self.pending.swap_remove(ready)];
            let base = self.visible.fork_at(&record.base).unwrap();
            let checked = inspect(&base, &record.context, record.change.raw_bytes()).unwrap();
            self.visible.apply_changes([checked]).unwrap();
            assert!(
                self.visible
                    .get_change_by_hash(&record.change.hash())
                    .is_some()
            );
        }
    }
}

fn run_schedules(history: &History, seed_value: u64, count: usize) {
    let mut rng = StdRng::seed_from_u64(seed_value);
    let mut expected_heads = history.expected.get_heads();
    expected_heads.sort();
    let mut distinct_orders = HashSet::new();
    for schedule in 0..count {
        let mut order: Vec<_> = (0..history.records.len()).collect();
        if schedule == 0 {
            order.reverse();
        } else {
            order.shuffle(&mut rng);
        }
        assert!(
            distinct_orders.insert(order.clone()),
            "duplicate sampled delivery order"
        );
        let mut receiver = Receiver::new();
        for &index in order.iter().take(order.len() / 2) {
            // A dropped first transmission is retried after reconnect.
            if rng.random_ratio(2, 3) {
                receiver.deliver(history, index);
                receiver.deliver(history, index);
            }
        }
        let acknowledged: Vec<_> = receiver
            .visible
            .get_changes(&[])
            .into_iter()
            .map(|change| change.hash())
            .collect();
        receiver.visible = Automerge::load(&receiver.visible.save()).unwrap();
        receiver.pending.clear(); // Unpublished, volatile input is retried, not acknowledged.
        for hash in acknowledged {
            assert!(receiver.visible.get_change_by_hash(&hash).is_some());
        }
        for &index in &order {
            receiver.deliver(history, index);
            if rng.random_ratio(1, 4) {
                receiver.deliver(history, index);
            }
        }
        assert!(
            receiver.pending.is_empty(),
            "seed={seed_value} schedule={schedule}"
        );
        let loaded = Automerge::load(&receiver.visible.save()).unwrap();
        let mut heads = loaded.get_heads();
        heads.sort();
        assert_eq!(
            heads, expected_heads,
            "seed={seed_value} schedule={schedule}"
        );
        assert_eq!(
            loaded.text(&history.text).unwrap(),
            history.expected.text(&history.text).unwrap()
        );
        assert_eq!(loaded.get_changes(&[]).len(), history.records.len() + 1);
        for record in history
            .records
            .iter()
            .step_by(8)
            .chain(history.records.last())
        {
            assert_eq!(
                loaded
                    .text_at(&history.text, &[record.change.hash()])
                    .unwrap(),
                record.expected
            );
            assert_eq!(
                loaded
                    .get_change_by_hash(&record.change.hash())
                    .unwrap()
                    .raw_bytes(),
                record.change.raw_bytes()
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 4, failure_persistence: None,
        rng_seed: proptest::test_runner::RngSeed::Fixed(0x505559554c415247),
        ..ProptestConfig::default()
    })]
    #[test]
    fn larger_mixed_observation_histories_keep_closed_historical_cuts(seed_value in any::<u64>()) {
        let history = history(seed_value);
        run_schedules(&history, seed_value ^ 0x524553554d45, 8);
    }
}

#[test]
#[ignore = "long native convergence gate; run explicitly and in scheduled CI"]
fn thousands_of_seeded_partition_duplicate_restart_schedules() {
    for seed_value in [0x505559554d495831, 0x505559554d495832] {
        let history = history(seed_value);
        run_schedules(&history, seed_value ^ 0x524553554d45, 1024);
    }
}
