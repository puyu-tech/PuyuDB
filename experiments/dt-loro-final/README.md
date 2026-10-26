# Final bounded DT–Loro experiment

This experiment closes the remaining **selection evidence** gaps, not production
qualification or I03. It compares the pinned Loro 1.17.1 fixture binding with DT
`89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922` plus the existing two native-interface
patches. The main workspace, dependency lock and toolchain are not changed.

## Protocol fixed before measurement

The tested interfaces are the author/inspect/publish/history/anchor flow and the
experimental input-inspection flow approved in the thread. No second merger,
replacement parser, new native format, production storage or authentication
implementation is introduced.

### Correctness

Run both bindings through deterministic mixed-observation histories, partition
and rejoin schedules, duplicate and missing-predecessor delivery, intermediate
reloads, saved cuts and anchors. Validate each new original against its declared
author base before merging it into a receiver. A receiver's newer tip is not a
substitute for that base. Check an independent scalar/string oracle where
concurrent ordering is unambiguous; do not demand that different native CRDT
tie-breaks or cursor policies produce identical results.

Keep the earlier strict-inspection and deleted-anchor regression tests. Run the
new Rust controls in debug and release. Attempt the relevant complete upstream DT
package tests separately; report pre-existing failures and missing prerequisites
instead of removing tests or calling an incomplete suite complete.

### Performance

The fixed measurement matrix has seven cells per engine:

| Case | Baseline | Trusted, native-generated prefill batches |
| --- | --- | --- |
| `ascii10k` | 10 KiB ASCII | 0, 1,000, 10,000 |
| `unicode100k` | Approximately 100 KiB Unicode | 0, 1,000, 10,000 |
| `churn1m` | Approximately 1 MiB, paste/delete churn | 1,000 |

Each cell has **64 measured sequential updates**, rotating four fixture writers.
Every measured update is fully authored, inspected and published; native-only
prefill is explicitly not counted as inspected admission. Three fresh processes
per engine/cell produce 42 measured processes and 2,688 operation rows per
platform if every primary cell completes. Alternate paired engine order; never run measured workloads concurrently.
There is no post-hoc sample exclusion, extra warmup removal or pooled percentile
across processes. Use nearest-rank p50/p95 within each process, then retain all
three process values and their descriptive median.

Time required capture/clone/fork/reconstruction work, full inspection, publication,
anchor creation/resolution, historical read, save/load and original-byte
hash/retention, including required temporary-state destruction. Retain component
times and the enclosing total. Independent oracle edit computation, report hashing
and output I/O are outside; correctness comparisons and reload verification inside
the exercised flow remain timed. A load observation therefore includes its
text/anchor/history verification and teardown, not only parser time.
Record whole-process lifetime maximum RSS where the OS supplies it; Windows live
peak samples are only lower bounds and must not be presented as equivalent.

Retained original update bytes, seed, digests and native snapshot are separate
components, not total database storage. No SQLite/WAL, signatures, indexes or
durability cost is inferred. Fixed process watchdog: 180 seconds per measurement.
A timeout or failed cell is retained as such, never silently dropped.

#### Correctness-pilot amendment before formal measurements

The implementation pilot exposed a **fixture-profile limitation**, not a Loro
convergence failure: the original `churn1m` combined 4,096-scalar replacement can
export multiple native changes, while the existing complete Loro inspector
requires exactly one native change. Large paste-only/delete-only edits passed.
Do not weaken that inspector or silently replace the original workload.

Keep all seven primary cells, including any original `churn1m` failure. Add
exactly one explicitly supplemental cell, `churn1m_bounded` at prefill 1,000:
the same 1 MiB baseline, 64-update schedule and large paste/delete cycles, but
combined replacement spans are 512 scalars. Run all three fresh-process trials
for **both** engines. This adds six processes, for 48 attempted per platform.
Report primary failures and supplemental results separately; never present the
supplement as a successful completion of the original matrix. No further
adaptive workload tuning is part of this experiment.

### Bounded malformed-input screening

Use a fixed deterministic corpus of at most 600 cases per engine/layer, with
valid controls, truncations, checksummed mutations, trailing input, extreme
length/varint probes and small seeded random inputs. Recompute the outer digest
so the binding is actually reached. Reuse the comparison's DT binding checks,
not an easier second inspector. Test ordinary native decode separately and label
it separately from the restricted binding dialect.

Each trial runs in a fresh child with a three-second wall watchdog and retains
its candidate bytes before decoding. Record returned errors, accepted variants,
panics, process crashes, timeouts, caller mutation and must-reject violations
separately. A changed byte is not automatically invalid. Apply child-only OS
memory/CPU limits where available; disclose their platform limitations.
This is deterministic mutation screening, **not coverage-guided cargo-fuzz, a
security proof or a guarantee against all resource exhaustion**.

### Decision rule

Correctness or validation failures cannot be traded for faster timings. Prefer
DT only if the complete tested binding remains attractive on relevant workloads
and its small patch/cursor-policy ownership is acceptable. Report mixed outcomes
instead of manufacturing one aggregate score. Loro's offline peer-allocation
policy and both candidates' outer publication/empty-seed obligations remain
explicit. No acceptance latency SLO has been supplied; descriptive measurements
cannot establish one.

Unneeded for this bounded selection experiment: a full database implementation,
every upstream feature, Wasm/32-bit qualification or an automatic engine switch.
Native Linux production support is not established by WSL2 measurements.

## Reproduce

Use the repository's provisioned Rust 1.99.1 toolchain/dependency cache and Python
3.9 or later. No dependency installation into the main graph is performed.

```sh
python3 -m unittest discover -s experiments/dt-loro-final -p 'test_*.py'
python3 experiments/dt-loro-final/run.py prepare
```

`prepare` prints a fresh `target/dt-loro-final/<timestamp>` directory. Substitute
that directory for `RUN` below:

```sh
python3 experiments/dt-loro-final/run.py check --run RUN
python3 experiments/dt-loro-final/run.py screen --run RUN
python3 experiments/dt-loro-final/run.py measure --run RUN
python3 experiments/dt-loro-final/run.py summarize --run RUN
```

Use `py -3.14` on the authorized native Windows machine. Finish other builds/tests
before `measure`; do not run Windows and WSL measurements simultaneously on the
same host. `all` creates a fresh run and executes the four phases in that order.
Phases refuse to overwrite retained commands or accept changed source inputs.

The runner reads the exact pinned Git object, safely extracts disposable native
source under ignored `target/`, and applies the existing patches there. It does
not modify Cargo source caches or permanently vendor an upstream tree. Generated
dependency resolution is seeded from the main lock; new registry versions or
unexpected path packages are rejected. Both native DT features and the main
dependency/toolchain inputs are checked explicitly.
The existing seven anchor and sixteen strict-input regression tests are copied
unchanged into the generated harness and executed with the integrated controls
in both debug and release.
The screen binary reuses the comparison module. Its duplicate embedded
`compare::tests::` executions are filtered out; those same tests run normally in
the standalone comparison target in each profile. No unique regression is skipped.

Raw stdout/stderr, generated manifests/locks, command outcomes, environment
observations, input/output hashes and per-process summaries are retained. The
summary verifies every sample's workload identity and compares text hashes across
engines and process trials before computing descriptive statistics.
