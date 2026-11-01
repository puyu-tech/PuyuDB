// SPDX-License-Identifier: Apache-2.0

//! Allocator-instrumented W01 companion; not a durable transaction benchmark.
//! Keep separate from native_workloads: instrumentation changes timing overhead.

use automerge::{ReadDoc, transaction::Transactable};
use puyudb_crdt::experiment::{EditContext, seed};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::{alloc::System, error::Error, time::Instant};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

fn main() -> Result<(), Box<dyn Error>> {
    println!(
        "baseline_bytes,sample,native_ns,allocations,deallocations,reallocations,bytes_allocated,bytes_deallocated,bytes_reallocated"
    );
    for bytes in [10 * 1024, 100 * 1024] {
        let mut seeded = seed(&[7; 32], &"a".repeat(bytes))?;
        let mut context = EditContext {
            public_key: [1; 32],
            branch: [1; 16],
            writer_epoch: [1; 16],
            content_epoch: [7; 32],
            outer_sequence: 1,
        };
        seeded.document.set_actor(context.actor());
        for index in 0..80 {
            context.outer_sequence = index + 1;
            let options = context.options();
            let region = Region::new(GLOBAL);
            let started = Instant::now();
            let mut tx = seeded.document.transaction();
            tx.splice_text(&seeded.text, (index as usize * 97) % 1024, 1, "x")?;
            tx.commit_with(options);
            let elapsed = started.elapsed().as_nanos();
            let stats = region.change();
            if index >= 16 {
                println!(
                    "{bytes},{},{elapsed},{},{},{},{},{},{}",
                    index - 16,
                    stats.allocations,
                    stats.deallocations,
                    stats.reallocations,
                    stats.bytes_allocated,
                    stats.bytes_deallocated,
                    stats.bytes_reallocated
                );
            }
        }
        assert_eq!(seeded.document.length(&seeded.text), bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_counter_observes_a_known_heap_allocation() {
        let region = Region::new(GLOBAL);
        let allocated = std::hint::black_box(vec![0u8; 4096]);
        let stats = region.change();
        assert!(stats.allocations >= 1);
        assert!(stats.bytes_allocated >= allocated.len());
    }
}
