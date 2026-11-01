// SPDX-License-Identifier: Apache-2.0

//! One oracle and operation schedule for every candidate. No native/PuyuDB IDs
//! are conflated, and full native saves do not prove original update-byte or
//! application transaction-boundary retention.

use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
    time::Instant,
};

use sha2::{Digest, Sha256};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region};

use crate::candidates::{AutomergeText, DiamondText, Edit, LoroText, NativeText, Result, YrsText};

const CUTS: [u64; 11] = [
    0, 1000, 1001, 1002, 2000, 2001, 2002, 25_000, 50_000, 75_000, 100_000,
];

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn materialize<T: NativeText>(candidate: &T, expected: &str) -> Result<u128> {
    let started = Instant::now();
    let actual = candidate.text()?;
    let elapsed = started.elapsed().as_nanos();
    if actual != expected {
        return Err("native current text differs from oracle".into());
    }
    Ok(elapsed)
}

pub(crate) fn run(allocations: bool) -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(2..=3).contains(&args.len())
        || (allocations && args[1] != "w01")
        || (args.len() == 3 && !matches!(args[1].as_str(), "w02" | "seed"))
    {
        return Err("usage: candidate_workloads ENGINE w01|w02|seed [NEW_ARTIFACT_PATH]; candidate_allocations ENGINE w01".into());
    }
    let artifact = args.get(2).map(Path::new);
    let output = io::stdout();
    let mut output = output.lock();
    match args[0].as_str() {
        "automerge" => execute::<AutomergeText>(&args[1], allocations, artifact, &mut output),
        "yrs" => execute::<YrsText>(&args[1], allocations, artifact, &mut output),
        "loro" => execute::<LoroText>(&args[1], allocations, artifact, &mut output),
        "diamond-types" => execute::<DiamondText>(&args[1], allocations, artifact, &mut output),
        _ => Err("unknown candidate".into()),
    }
}

fn execute<T: NativeText>(
    workload: &str,
    allocations: bool,
    artifact: Option<&Path>,
    output: &mut impl Write,
) -> Result<()> {
    match workload {
        "w01" => typical::<T>(allocations, output),
        "w02" => long_history::<T>(1024 * 1024, 100_000, artifact, output),
        "seed" => seed_probe::<T>(artifact, output),
        _ => Err("expected w01, w02 or seed".into()),
    }
}

fn typical<T: NativeText>(allocations: bool, output: &mut impl Write) -> Result<()> {
    write!(
        output,
        "engine,baseline_bytes,sample,coordinate_ns,native_ns,call_ns,materialize_ns"
    )?;
    if allocations {
        write!(
            output,
            ",allocations,deallocations,reallocations,bytes_allocated,bytes_deallocated,bytes_reallocated"
        )?;
    }
    writeln!(output)?;
    for bytes in [10 * 1024, 100 * 1024] {
        let mut oracle = "a".repeat(bytes);
        let mut candidate = T::seed(&oracle)?;
        materialize(&candidate, &oracle)?;
        for index in 0..80 {
            let start = index * 97 % 1024;
            let started = Instant::now();
            let edit = Edit::at(&oracle, start..start + 1, "x")?;
            let coordinate_ns = started.elapsed().as_nanos();
            // Only the separate allocator binary installs this allocator.
            let region = allocations.then(|| Region::new(&INSTRUMENTED_SYSTEM));
            let started = Instant::now();
            let native_ns = candidate.splice(&edit)?.as_nanos();
            let call_ns = started.elapsed().as_nanos();
            let stats = region.map(|r| r.change());
            oracle.replace_range(start..start + 1, "x");
            let materialize_ns = materialize(&candidate, &oracle)?;
            if index >= 16 {
                write!(
                    output,
                    "{},{bytes},{},{coordinate_ns},{native_ns},{call_ns},{materialize_ns}",
                    T::NAME,
                    index - 16
                )?;
                if let Some(stats) = stats {
                    write!(
                        output,
                        ",{},{},{},{},{},{}",
                        stats.allocations,
                        stats.deallocations,
                        stats.reallocations,
                        stats.bytes_allocated,
                        stats.bytes_deallocated,
                        stats.bytes_reallocated
                    )?;
                }
                writeln!(output)?;
            }
        }
        if candidate.text()? != oracle {
            return Err("W01 native text differs from oracle".into());
        }
    }
    Ok(())
}

fn long_history<T: NativeText>(
    baseline_bytes: usize,
    edits: u64,
    artifact: Option<&Path>,
    output: &mut impl Write,
) -> Result<()> {
    let mut oracle = "a".repeat(baseline_bytes);
    let started = Instant::now();
    let mut candidate = T::seed(&oracle)?;
    let seed_ns = started.elapsed().as_nanos();
    let seed_materialize_ns = materialize(&candidate, &oracle)?;
    let mut cuts = vec![(0, candidate.cut(), oracle.clone())];
    let paste = "p".repeat(64 * 1024);
    let mut restore = String::new();
    let mut native_ns = 0;
    let mut call_ns = 0;
    let mut coordinate_ns = 0;
    let started = Instant::now();
    for index in 0..edits {
        if index % 10_000 == 2000 {
            restore = oracle[..64 * 1024].to_owned();
        }
        let (range, inserted) = match index % 10_000 {
            1000 => (0..0, paste.as_str()),
            1001 | 2000 => (0..64 * 1024, ""),
            2001 => (0..0, restore.as_str()),
            _ => {
                let position = usize::try_from(index % 1024)?;
                (
                    position..position + 1,
                    if index % 2 == 0 { "x" } else { "y" },
                )
            }
        };
        let translation_started = Instant::now();
        let edit = Edit::at(&oracle, range.clone(), inserted)?;
        coordinate_ns += translation_started.elapsed().as_nanos();
        let call_started = Instant::now();
        native_ns += candidate.splice(&edit)?.as_nanos();
        call_ns += call_started.elapsed().as_nanos();
        oracle.replace_range(range, inserted);
        let completed = index + 1;
        if CUTS.contains(&completed) || completed == edits {
            if candidate.text()? != oracle {
                return Err("W02 captured native text differs from oracle".into());
            }
            cuts.push((completed, candidate.cut(), oracle.clone()));
        }
    }
    let history_ns = started.elapsed().as_nanos();
    if candidate.text()? != oracle {
        return Err("W02 final text differs from oracle".into());
    }
    writeln!(output, "metric,value")?;
    writeln!(output, "engine,{}", T::NAME)?;
    writeln!(output, "baseline_bytes,{baseline_bytes}")?;
    writeln!(output, "logical_edit_batches,{edits}")?;
    writeln!(output, "seed_and_writer_setup_ns,{seed_ns}")?;
    writeln!(output, "seed_materialize_ns,{seed_materialize_ns}")?;
    writeln!(output, "history_build_ns,{history_ns}")?;
    writeln!(output, "native_edit_ns_sum,{native_ns}")?;
    writeln!(output, "native_call_ns_sum,{call_ns}")?;
    writeln!(output, "coordinate_ns_sum,{coordinate_ns}")?;
    let started = Instant::now();
    let bytes = candidate.save()?;
    writeln!(output, "native_save_ns,{}", started.elapsed().as_nanos())?;
    writeln!(output, "native_artifact_bytes,{}", bytes.len())?;
    writeln!(output, "native_artifact_sha256,{}", digest(&bytes))?;
    if let Some(path) = artifact {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .write_all(&bytes)?;
    }
    let started = Instant::now();
    let loaded = T::load(&bytes)?;
    writeln!(output, "native_load_ns,{}", started.elapsed().as_nanos())?;
    writeln!(
        output,
        "latest_text_materialization_ns,{}",
        materialize(&loaded, &oracle)?
    )?;
    drop(loaded);
    for (index, (completed, cut, expected)) in cuts.iter().enumerate() {
        // Fresh latest-state load before every historical query; no accumulated
        // historical-view cache is carried from another cut.
        let started = Instant::now();
        let mut loaded = T::load(&bytes)?;
        writeln!(
            output,
            "cut_{index}_reload_ns,{}",
            started.elapsed().as_nanos()
        )?;
        writeln!(
            output,
            "cut_{index}_latest_text_ns,{}",
            materialize(&loaded, &oracle)?
        )?;
        let started = Instant::now();
        let actual = loaded.text_at(cut)?;
        let elapsed = started.elapsed().as_nanos();
        if &actual != expected {
            return Err(format!("W02 saved cut {completed} differs from oracle").into());
        }
        writeln!(output, "cut_{index}_after_batches,{completed}")?;
        writeln!(output, "cut_{index}_read_ns,{elapsed}")?;
        writeln!(
            output,
            "cut_{index}_text_sha256,{}",
            digest(actual.as_bytes())
        )?;
    }
    writeln!(output, "verified_cuts,{}", cuts.len())?;
    writeln!(output, "final_text_sha256,{}", digest(oracle.as_bytes()))?;
    Ok(())
}

fn seed_probe<T: NativeText>(artifact: Option<&Path>, output: &mut impl Write) -> Result<()> {
    writeln!(
        output,
        "engine,case,artifact_bytes,artifact_sha256,text_sha256"
    )?;
    for (index, baseline) in ["", "A😀e\u{301}\r\n\0", "abcdef"].into_iter().enumerate() {
        let mut candidate = T::seed(baseline)?;
        let bytes = candidate.save()?;
        if T::load(&bytes)?.text()? != baseline {
            return Err("seed did not roundtrip".into());
        }
        if let Some(prefix) = artifact {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(prefix.with_extension(format!("{index}.native")))?
                .write_all(&bytes)?;
        }
        writeln!(
            output,
            "{},{index},{},{},{}",
            T::NAME,
            bytes.len(),
            digest(&bytes),
            digest(baseline.as_bytes())
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deleted_cuts<T: NativeText>() {
        let mut output = Vec::new();
        long_history::<T>(128 * 1024, 2002, None, &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("logical_edit_batches,2002\n"));
        assert!(output.contains("verified_cuts,7\n"));
        assert!(output.contains("cut_2_after_batches,1001\n"));
        assert!(output.contains("cut_5_after_batches,2001\n"));
    }

    #[test]
    #[ignore = "retained-history cost fixture; execute explicitly in release mode"]
    fn all_candidates_reconstruct_paste_delete_and_restore_intervals() {
        deleted_cuts::<AutomergeText>();
        deleted_cuts::<YrsText>();
        deleted_cuts::<LoroText>();
        deleted_cuts::<DiamondText>();
    }
}
