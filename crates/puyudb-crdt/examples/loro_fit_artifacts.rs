// SPDX-License-Identifier: Apache-2.0

//! Original-update byte bookkeeping for a fixed trusted Loro fixture.
//! Not a PuyuDB storage format, allocator/admission benchmark or durability test.

#[path = "../tests/support/loro_fit.rs"]
mod fit;

use loro::{ExportMode, IdSpan};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() > 1 {
        return Err("usage: loro_fit_artifacts [NEW_OUTPUT_DIRECTORY]".into());
    }
    let output = args.first().map(Path::new);
    if let Some(directory) = output {
        fs::create_dir(directory)?;
    }
    let baseline = "a".repeat(10 * 1024);
    let base = fit::FixtureBase::seed([7; 32], &baseline)?;
    let binding = fit::EpochPeers::new([7; 32]).bind(
        fit::FixtureWriter {
            public_key: [1; 32],
            branch: [1; 16],
            writer_epoch: [1; 16],
        },
        2,
    )?;
    let seed = base.document()?.export(ExportMode::Snapshot)?;
    let first = fit::author(
        &base,
        &binding,
        1,
        &[fit::ByteEdit {
            range: 0..1,
            inserted: "x",
        }],
    )?;
    // The small positive path is inspected once. Subsequent rows measure
    // export/hash/retention only, NOT end-to-end admission or inspection cost.
    let source = fit::inspect(&base, &binding, 1, &first)?;
    let text = source.get_text("text");
    let mut oracle = baseline;
    oracle.replace_range(0..1, "x");
    let mut originals = vec![first];
    let mut spans = vec![(0, 2)];
    for sequence in 2..=1000 {
        let from = source.oplog_vv();
        let start = from.get(&2).copied().unwrap_or(0);
        let position = (sequence - 1) as usize * 97 % 1024;
        let inserted = if sequence % 2 == 1 { "x" } else { "y" };
        text.delete(position, 1)?;
        text.insert(position, inserted)?;
        source.commit_with(
            loro::CommitOptions::new()
                .timestamp(0)
                .commit_msg(&fit::message(&binding, sequence)),
        );
        oracle.replace_range(position..position + 1, inserted);
        let end = source.oplog_vv().get(&2).copied().unwrap_or(0);
        spans.push((start, end));
        originals.push(fit::CapturedUpdate::new(
            source.export(ExportMode::updates(&from))?,
        ));
    }
    if text.to_string() != oracle {
        return Err("artifact fixture text differs from oracle".into());
    }
    let snapshot = source.export(ExportMode::Snapshot)?;
    let loaded = fit::document(9)?;
    if loaded.import(&snapshot)?.pending.is_some() || loaded.is_shallow() {
        return Err("expected complete non-shallow artifact fixture".into());
    }
    let version = loaded.oplog_vv();
    let mut byte_equal = 0;
    let mut replay_noops = 0;
    let mut digest_index = Sha256::new();
    let mut index = if let Some(directory) = output {
        let mut index = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("updates.csv"))?;
        writeln!(
            index,
            "ordinal,native_peer,native_start,native_end,bytes,sha256,artifact_hex"
        )?;
        Some(index)
    } else {
        None
    };
    for (index_number, (original, &(start, end))) in originals.iter().zip(&spans).enumerate() {
        if <[u8; 32]>::from(Sha256::digest(&original.bytes)) != original.sha256 {
            return Err("retained original artifact checksum changed".into());
        }
        digest_index.update(original.sha256);
        let result = loaded.import(&original.bytes)?;
        if result.pending.is_some() || result.success.iter().next().is_some() {
            return Err("original replay unexpectedly introduced native history".into());
        }
        replay_noops += 1;
        let reexported = loaded.export(ExportMode::updates_in_range(vec![IdSpan::new(
            2, start, end,
        )]))?;
        if reexported == original.bytes {
            byte_equal += 1;
        }
        if let Some(file) = &mut index {
            writeln!(
                file,
                "{},2,{start},{end},{},{},{}",
                index_number + 1,
                original.bytes.len(),
                fit::hex(&original.sha256),
                fit::hex(&original.bytes)
            )?;
        }
    }
    if loaded.oplog_vv() != version || loaded.get_text("text").to_string() != oracle {
        return Err("known original replay changed the fixture".into());
    }
    if let Some(directory) = output {
        for (name, bytes) in [("seed.native", &seed), ("snapshot.native", &snapshot)] {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join(name))?
                .write_all(bytes)?;
        }
    }
    println!("metric,value");
    println!("profile,loro-fit-i02-retained-updates-v1");
    println!("baseline_bytes,10240");
    println!("logical_batches,1000");
    println!("seed_snapshot_bytes,{}", seed.len());
    println!(
        "retained_original_update_bytes,{}",
        originals.iter().map(|r| r.bytes.len()).sum::<usize>()
    );
    println!(
        "smallest_original_update_bytes,{}",
        originals.iter().map(|r| r.bytes.len()).min().unwrap()
    );
    println!(
        "largest_original_update_bytes,{}",
        originals.iter().map(|r| r.bytes.len()).max().unwrap()
    );
    println!("digest_value_bytes,{}", originals.len() * 32);
    println!("full_native_snapshot_bytes,{}", snapshot.len());
    println!("source_native_changes,{}", source.len_changes());
    println!("reloaded_native_changes,{}", loaded.len_changes());
    println!("original_replays_noop,{replay_noops}");
    println!("same_range_reexports_byte_equal,{byte_equal}");
    println!(
        "ordered_original_digest_index_sha256,{}",
        fit::hex(&digest_index.finalize())
    );
    println!(
        "final_text_sha256,{}",
        fit::hex(&Sha256::digest(oracle.as_bytes()))
    );
    Ok(())
}
