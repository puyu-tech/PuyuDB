// SPDX-License-Identifier: Apache-2.0

//! Fixed trusted-fixture original-byte bookkeeping, not a PuyuDB storage format,
//! admission/allocator benchmark or durability test. No native compression.

#[path = "../tests/support/diamond_fit.rs"]
mod fit;

use diamond_types::{HasLength, list::encoding::EncodeOptions};
use sha2::{Digest, Sha256};
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--seeds"] {
        println!("baseline_hex,epoch_hex,expected_seed_agent,bytes,sha256,artifact_hex");
        for baseline in ["", "A😀e\u{301}\r\n日本", "baseline"] {
            let base = fit::FixtureBase::seed([7; 32], baseline)?;
            let raw = base
                .document()?
                .encode(&EncodeOptions::full().compress_content(false));
            println!(
                "{},{},{},{},{},{}",
                fit::hex(baseline.as_bytes()),
                fit::hex(&[7; 32]),
                fit::seed_name(&[7; 32]),
                raw.len(),
                fit::hex(&Sha256::digest(&raw)),
                fit::hex(&raw),
            );
        }
        return Ok(());
    }
    if args.len() > 1 {
        return Err("usage: diamond_fit_artifacts [NEW_OUTPUT_DIRECTORY | --seeds]".into());
    }
    let output = args.first().map(Path::new);
    if let Some(directory) = output {
        fs::create_dir(directory)?;
    }
    let epoch = [7; 32];
    let writer = fit::FixtureWriter {
        key: [1; 32],
        branch: [1; 16],
        writer_epoch: [1; 16],
    };
    let name = fit::writer_name(&epoch, &writer);
    let baseline = "a".repeat(10 * 1024);
    let base = fit::FixtureBase::seed(epoch, &baseline)?;
    let seed_log = base.document()?;
    let seed = seed_log.encode(&EncodeOptions::full().compress_content(false));
    let mut cuts = vec![fit::frontier(&seed_log)];
    let first = fit::author(
        &base,
        &epoch,
        &writer,
        1,
        &[fit::ByteEdit {
            range: 0..1,
            inserted: "x",
        }],
    )?;
    // Match the Loro byte probe: inspect only the first positive update.
    // Remaining rows measure native export/hash/retention, NOT admission cost.
    let mut source = fit::inspect(&base, &epoch, &writer, 1, &first)?;
    cuts.push(fit::frontier(&source));
    let mut view = source.checkout_tip();
    let agent = source.get_or_create_agent_id(&name);
    let mut oracle = baseline;
    oracle.replace_range(0..1, "x");
    let mut originals = vec![first];
    for sequence in 2..=1000 {
        let from = source.local_frontier();
        let position = (sequence - 1) as usize * 97 % 1024;
        let inserted = if sequence % 2 == 1 { "x" } else { "y" };
        view.delete(&mut source, agent, position..position + 1);
        view.insert(&mut source, agent, position, inserted);
        oracle.replace_range(position..position + 1, inserted);
        let context = fit::message(&epoch, &writer, sequence);
        originals.push(fit::CapturedUpdate::new(
            source.encode_from(
                &EncodeOptions::patch()
                    .compress_content(false)
                    .user_data(context.as_bytes()),
                from.as_ref(),
            ),
        ));
        cuts.push(fit::frontier(&source));
    }
    if view.content() != oracle {
        return Err("artifact fixture differs from oracle".into());
    }
    let snapshot = source.encode(&EncodeOptions::full().compress_content(false));
    let mut loaded = diamond_types::list::ListOpLog::load_from(&snapshot)?;
    let before_ops = loaded.len();
    let mut digest_index = Sha256::new();
    let mut replay_noops = 0;
    let mut bounded_equal = 0;
    let mut index = if let Some(directory) = output {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("updates.csv"))?;
        writeln!(
            file,
            "ordinal,native_agent,native_start,native_end,bytes,sha256,artifact_hex"
        )?;
        Some(file)
    } else {
        None
    };
    for (i, original) in originals.iter().enumerate() {
        if <[u8; 32]>::from(Sha256::digest(&original.bytes)) != original.sha256 {
            return Err("retained original digest changed".into());
        }
        digest_index.update(original.sha256);
        loaded.decode_and_add(&original.bytes)?;
        if loaded.len() != before_ops {
            return Err("known original replay introduced native history".into());
        }
        replay_noops += 1;
        // DT has no bounded upper-tip export API: explicitly reconstruct the
        // trusted prefix and restore separately known original outer context.
        let prefix = fit::FixtureBase::capture(epoch, &loaded, cuts[i + 1].clone())?.document()?;
        let from = prefix
            .cg
            .agent_assignment
            .try_remote_to_local_frontier(cuts[i].iter())
            .map_err(|_| "bounded fixture frontier unavailable")?;
        let context = fit::message(&epoch, &writer, (i + 1) as u64);
        let reexport = prefix.encode_from(
            &EncodeOptions::patch()
                .compress_content(false)
                .user_data(context.as_bytes()),
            from.as_ref(),
        );
        if reexport == original.bytes {
            bounded_equal += 1;
        }
        if let Some(file) = &mut index {
            writeln!(
                file,
                "{},{},{},{},{},{},{}",
                i + 1,
                name,
                i * 2,
                i * 2 + 2,
                original.bytes.len(),
                fit::hex(&original.sha256),
                fit::hex(&original.bytes)
            )?;
        }
    }
    if loaded.checkout_tip().content() != oracle {
        return Err("known replay changed native text".into());
    }
    let writer_atoms: usize = loaded
        .iter_remote_mappings()
        .filter(|s| s.0 == name)
        .map(|s| s.1.len())
        .sum();
    if writer_atoms != 2000 {
        return Err("unexpected writer atom count".into());
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
    println!("profile,diamond-fit-i02-retained-updates-v1");
    println!("baseline_bytes,10240");
    println!("logical_batches,1000");
    println!("seed_snapshot_bytes,{}", seed.len());
    println!(
        "retained_original_update_bytes,{}",
        originals.iter().map(|a| a.bytes.len()).sum::<usize>()
    );
    println!(
        "smallest_original_update_bytes,{}",
        originals.iter().map(|a| a.bytes.len()).min().unwrap()
    );
    println!(
        "largest_original_update_bytes,{}",
        originals.iter().map(|a| a.bytes.len()).max().unwrap()
    );
    println!("digest_value_bytes,{}", originals.len() * 32);
    println!("full_native_snapshot_bytes,{}", snapshot.len());
    println!("source_native_operation_atoms,{}", source.len());
    println!("reloaded_native_operation_atoms,{}", loaded.len());
    println!("native_run_entries,{}", loaded.iter_full().count());
    println!("original_replays_noop,{replay_noops}");
    println!("bounded_prefix_reexports_after_context_restore_byte_equal,{bounded_equal}");
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
