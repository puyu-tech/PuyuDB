// SPDX-License-Identifier: Apache-2.0

//! Default native merge/byte observations, identical before and after the patches.

use diamond_types::list::{ListOpLog, encoding::EncodeOptions};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn save(log: &ListOpLog) -> Vec<u8> {
    log.encode(&EncodeOptions::full().compress_content(false))
}

fn show(case: &str, log: &ListOpLog, expected: &str) {
    let text = log.checkout_tip().content().to_string();
    assert_eq!(text, expected);
    println!("{case},{},{}", hex(&save(log)), hex(text.as_bytes()));
}

fn main() {
    println!("case,native_hex,text_hex");
    for (i, text) in ["", "A😀e\u{301}\r\n日本", "baseline"]
        .into_iter()
        .enumerate()
    {
        let mut log = ListOpLog::new();
        if !text.is_empty() {
            let seed = log.get_or_create_agent_id("seed");
            log.add_insert(seed, 0, text);
        }
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        show(&format!("seed-{i}"), &loaded, text);
    }
    let mut seed = ListOpLog::new();
    let agent = seed.get_or_create_agent_id("seed");
    seed.add_insert(agent, 0, "ab");
    let base = save(&seed);
    let mut updates = Vec::new();
    for name in ["left", "right", "delete"] {
        let mut log = ListOpLog::load_from(&base).unwrap();
        let parents = log.local_frontier();
        let agent = log.get_or_create_agent_id(name);
        match name {
            "left" => {
                log.add_insert_at(agent, parents.as_ref(), 0, "😀");
            }
            "right" => {
                log.add_insert_at(agent, parents.as_ref(), 2, "e\u{301}");
            }
            "delete" => {
                log.add_delete_at(agent, parents.as_ref(), 0..2);
            }
            _ => unreachable!(),
        }
        updates.push(save(&log));
    }
    for (i, order) in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ]
    .into_iter()
    .enumerate()
    {
        let mut log = ListOpLog::load_from(&base).unwrap();
        for index in order {
            log.decode_and_add(&updates[index]).unwrap();
        }
        show(&format!("merged-{i}"), &log, "😀e\u{301}");
        let loaded = ListOpLog::load_from(&save(&log)).unwrap();
        show(&format!("reloaded-{i}"), &loaded, "😀e\u{301}");
    }
}
