// SPDX-License-Identifier: Apache-2.0

//! Puyu CLI entry point. Database operations are not implemented.

use clap::Command;

fn main() {
    Command::new("puyu")
        .bin_name("puyu")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Puyu command-line tools")
        .after_help("Foundation only. No database operations are implemented yet.")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("db")
                .about("PuyuDB database and synchronization engine")
                .arg_required_else_help(true)
                .after_help(
                    "Foundation only. No database operations are implemented yet.\n\
                     PuyuDB is not ready for production use.",
                ),
        )
        .get_matches();
}
