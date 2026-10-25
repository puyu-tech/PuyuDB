// SPDX-License-Identifier: Apache-2.0

//! Separate allocator-instrumented execution of the same candidate W01 workload.

#[path = "support/candidates.rs"]
mod candidates;
#[path = "support/workloads.rs"]
mod workloads;

#[global_allocator]
static GLOBAL: &stats_alloc::StatsAlloc<std::alloc::System> = &stats_alloc::INSTRUMENTED_SYSTEM;

fn main() -> candidates::Result<()> {
    workloads::run(true)
}
