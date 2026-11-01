// SPDX-License-Identifier: Apache-2.0

//! Independent-process seed probe, not a Puyu CLI command or supported interface.

use puyudb_crdt::experiment::seed;
use std::{error::Error, fmt::Write};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let epoch_byte: u8 = args
        .next()
        .ok_or("usage: native_seed_probe EPOCH_BYTE TEXT")?
        .parse()?;
    let baseline = args.next().ok_or("missing baseline text")?;
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let seed = seed(&[epoch_byte; 32], &baseline)?;
    let mut hex = String::with_capacity(seed.change.raw_bytes().len() * 2);
    for byte in seed.change.raw_bytes() {
        write!(hex, "{byte:02x}")?;
    }
    println!("{hex}\n{}", seed.change.hash());
    Ok(())
}
