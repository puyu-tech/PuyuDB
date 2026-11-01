// SPDX-License-Identifier: Apache-2.0

//! Native deterministic seed gate.

use automerge::{Automerge, Change, ROOT, ReadDoc};
use puyudb_crdt::experiment::{seed, seed_actor};
use std::process::Command;

#[test]
fn seed_retains_exact_baseline_and_has_only_one_zero_time_change() {
    let epoch = [7; 32];
    let baseline = "A😀e\u{301}\r\n日本語\0";
    let seed = seed(&epoch, baseline).expect("seed");

    assert_eq!(seed.document.text(&seed.text).unwrap(), baseline);
    assert_eq!(seed.document.keys(ROOT).collect::<Vec<_>>(), ["text"]);
    assert_eq!(seed.document.get_changes(&[]).len(), 1);
    assert_eq!(seed.change.actor_id(), &seed_actor(&epoch));
    assert_eq!(seed.change.seq(), 1);
    assert_eq!(seed.change.start_op().get(), 1);
    assert_eq!(seed.change.timestamp(), 0);
    assert_eq!(seed.change.message(), Some("puyudb/am-text-seed/i02"));
    assert!(seed.change.deps().is_empty());
    assert!(seed.change.extra_bytes().is_empty());
    assert!(seed.change.author().is_none());
}

#[test]
fn independent_processes_match_retained_native_byte_and_hash_fixtures() {
    for (baseline, expected) in [
        ("", include_str!("fixtures/seed-empty.txt")),
        ("A😀e\u{301}", include_str!("fixtures/seed-unicode.txt")),
    ] {
        for _ in 0..3 {
            let output = Command::new(env!("CARGO_BIN_EXE_native_seed_probe"))
                .args(["7", baseline])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty());
            assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        }
    }
}

#[test]
fn original_seed_bytes_survive_reload_and_epoch_namespaces_are_distinct() {
    for baseline in ["", "abc", "😀\r\n\0", "e\u{301}", "é", "日本語"] {
        let seeded = seed(&[7; 32], baseline).unwrap();
        let loaded = Automerge::load(&seeded.document.save()).unwrap();
        let retained = loaded.get_change_by_hash(&seeded.change.hash()).unwrap();
        assert_eq!(retained.raw_bytes(), seeded.change.raw_bytes());
        let parsed = Change::from_bytes(retained.raw_bytes().to_vec()).unwrap();
        assert_eq!(parsed.hash(), seeded.change.hash());
        assert_eq!(loaded.text(&seeded.text).unwrap(), baseline);
        assert_ne!(
            seed(&[8; 32], baseline).unwrap().change.hash(),
            parsed.hash()
        );
    }
    assert_eq!(
        seed_actor(&[7; 32]).to_string(),
        "00ea31681718b2da3c5d2e0e2da5748068ea5d0c2e4ca2b5f83213fdeca55e1dd4"
    );
}
