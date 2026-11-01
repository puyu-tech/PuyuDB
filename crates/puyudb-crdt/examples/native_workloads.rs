// SPDX-License-Identifier: Apache-2.0

//! Native-only W01/W02 measurements; no SQLite, durability, or database benchmark.

use automerge::{Automerge, ReadDoc, transaction::Transactable};
use puyudb_crdt::experiment::{EditContext, seed};
use sha2::{Digest, Sha256};
use std::{error::Error, time::Instant};

fn context(sequence: u64) -> EditContext {
    EditContext {
        public_key: [1; 32],
        branch: [1; 16],
        writer_epoch: [1; 16],
        content_epoch: [7; 32],
        outer_sequence: sequence,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let workload = std::env::args()
        .nth(1)
        .ok_or("usage: native_workloads w01|w02")?;
    match workload.as_str() {
        "w01" => typical_edits()?,
        "w02" => long_history()?,
        _ => return Err("expected w01 or w02".into()),
    }
    Ok(())
}

fn typical_edits() -> Result<(), Box<dyn Error>> {
    println!("workload,baseline_bytes,sample,native_commit_ns");
    for bytes in [10 * 1024, 100 * 1024] {
        let baseline = "a".repeat(bytes);
        let mut seeded = seed(&[7; 32], &baseline)?;
        seeded.document.set_actor(context(1).actor());
        // One native transaction models one 50-ms batch; idle time is not slept.
        for index in 0..80 {
            let options = context(index + 1).options();
            let started = Instant::now();
            let mut tx = seeded.document.transaction();
            tx.splice_text(&seeded.text, (index as usize * 97) % 1024, 1, "x")?;
            tx.commit_with(options);
            let elapsed = started.elapsed().as_nanos();
            // First 16 transactions warm the view; publish all 64 measured samples.
            if index >= 16 {
                println!("W01,{bytes},{},{elapsed}", index - 16);
            }
        }
        assert_eq!(seeded.document.length(&seeded.text), bytes);
    }
    Ok(())
}

fn long_history() -> Result<(), Box<dyn Error>> {
    let baseline = "a".repeat(1024 * 1024);
    let started = Instant::now();
    let mut seeded = seed(&[7; 32], &baseline)?;
    println!("metric,value");
    println!("baseline_bytes,{}", baseline.len());
    println!("seed_ns,{}", started.elapsed().as_nanos());
    seeded.document.set_actor(context(1).actor());
    let mut cuts = vec![(seeded.document.get_heads(), baseline.clone())];
    let paste = "p".repeat(64 * 1024);
    let mut restore = String::new();
    let started = Instant::now();
    for index in 0..100_000u64 {
        let options = context(index + 1).options();
        if index % 10_000 == 2000 {
            restore = seeded.document.text(&seeded.text)?[..64 * 1024].to_owned();
        }
        let mut tx = seeded.document.transaction();
        match index % 10_000 {
            1000 => tx.splice_text(&seeded.text, 0, 0, &paste)?,
            1001 => tx.splice_text(&seeded.text, 0, 64 * 1024, "")?,
            2000 => tx.splice_text(&seeded.text, 0, 64 * 1024, "")?,
            2001 => tx.splice_text(&seeded.text, 0, 0, &restore)?,
            _ => tx.splice_text(
                &seeded.text,
                index as usize % 1024,
                1,
                if index % 2 == 0 { "x" } else { "y" },
            )?,
        }
        tx.commit_with(options);
        if (index + 1) % 25_000 == 0 {
            cuts.push((
                seeded.document.get_heads(),
                seeded.document.text(&seeded.text)?,
            ));
        }
    }
    println!("native_edit_transactions,100000");
    println!("history_build_ns,{}", started.elapsed().as_nanos());
    println!(
        "retained_native_changes,{}",
        seeded.document.get_changes(&[]).len()
    );
    println!(
        "raw_native_snapshot_bytes,{}",
        seeded.document.save_nocompress().len()
    );
    let saved = seeded.document.save();
    println!("compressed_native_snapshot_bytes,{}", saved.len());
    let started = Instant::now();
    let loaded = Automerge::load(&saved)?;
    println!("cold_native_decode_ns,{}", started.elapsed().as_nanos());
    for (index, (heads, expected)) in cuts.iter().enumerate() {
        let started = Instant::now();
        let actual = loaded.text_at(&seeded.text, heads)?;
        let elapsed = started.elapsed().as_nanos();
        assert_eq!(&actual, expected, "historical cut {index}");
        println!("historical_cut_{index}_read_ns,{elapsed}");
    }
    assert_eq!(loaded.text(&seeded.text)?.len(), baseline.len());
    let digest = Sha256::digest(loaded.text(&seeded.text)?.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    println!("final_text_sha256,{hex}");
    Ok(())
}
