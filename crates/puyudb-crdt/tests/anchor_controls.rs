// SPDX-License-Identifier: Apache-2.0

//! Loro native-cursor controls for DT's deleted-anchor workload.
//! Native Side values are not relabeled as the DT projection's gap-affinity policy.

use loro::{
    CommitOptions, ExportMode, LoroDoc,
    cursor::{Cursor, Side},
};

fn doc(peer: u64) -> LoroDoc {
    let doc = LoroDoc::new();
    doc.set_peer_id(peer).unwrap();
    doc.set_record_timestamp(false);
    doc.set_change_merge_interval(-1);
    doc
}

fn deleted_neighbors(
    baseline: &str,
    point: usize,
    left_at: usize,
    right_at: usize,
    deleted: std::ops::Range<usize>,
    expected: &str,
    expected_point: usize,
) {
    let base = doc(1);
    base.get_text("text").insert(0, baseline).unwrap();
    base.commit_with(CommitOptions::new().timestamp(0).commit_msg("seed"));
    let cut = base.oplog_frontiers();
    let vv = base.oplog_vv();
    let cursors: Vec<_> = [Side::Left, Side::Right]
        .into_iter()
        .map(|side| {
            let c = base.get_text("text").get_cursor(point, side).unwrap();
            Cursor::decode(&c.encode()).unwrap()
        })
        .collect();
    let mut updates = Vec::new();
    for peer in 2..=4 {
        let source = base.fork_at(&cut).unwrap();
        source.set_peer_id(peer).unwrap();
        let text = source.get_text("text");
        match peer {
            2 => text.insert(left_at, "😀").unwrap(),
            3 => text.insert(right_at, "e\u{301}").unwrap(),
            4 => text.delete(deleted.start, deleted.len()).unwrap(),
            _ => unreachable!(),
        }
        source.commit_with(CommitOptions::new().timestamp(0).commit_msg("concurrent"));
        updates.push(source.export(ExportMode::updates(&vv)).unwrap());
    }
    let mut observed = Vec::new();
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let receiver = base.fork_at(&cut).unwrap();
        for index in order {
            receiver.import(&updates[index]).unwrap();
        }
        assert_eq!(receiver.get_text("text").to_string(), expected);
        let positions: Vec<_> = cursors
            .iter()
            .map(|c| receiver.get_cursor_pos(c).unwrap().current)
            .collect();
        let loaded = doc(9);
        loaded
            .import(&receiver.export(ExportMode::Snapshot).unwrap())
            .unwrap();
        let after: Vec<_> = cursors
            .iter()
            .map(|c| loaded.get_cursor_pos(c).unwrap().current)
            .collect();
        assert_eq!(positions, after);
        let historical = loaded.fork_at(&cut).unwrap();
        for c in &cursors {
            assert_eq!(historical.get_cursor_pos(c).unwrap().current.pos, point);
        }
        observed.push(positions);
    }
    assert!(
        observed.windows(2).all(|pair| pair[0] == pair[1]),
        "{observed:?}"
    );
    for positions in observed {
        for position in positions {
            assert_eq!(position.pos, expected_point);
            assert_eq!(position.side, Side::Left);
        }
    }
}

#[test]
fn native_loro_cursor_control_survives_deleted_edge_neighbors_and_import_orders() {
    deleted_neighbors("ab", 0, 0, 2, 0..2, "😀e\u{301}", 1);
}

#[test]
fn native_loro_cursor_control_survives_deleted_interior_neighbors_and_import_orders() {
    deleted_neighbors("abcd", 2, 1, 3, 1..3, "a😀e\u{301}d", 2);
}
