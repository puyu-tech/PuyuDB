// SPDX-License-Identifier: Apache-2.0

//! Common native-only candidate workloads; not a database or admission benchmark.

#[path = "support/candidates.rs"]
mod candidates;
#[path = "support/workloads.rs"]
mod workloads;

fn main() -> candidates::Result<()> {
    workloads::run(false)
}
