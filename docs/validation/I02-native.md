# I02 native text/history validation

**Status: native-feasibility evidence and common four-candidate costs implemented;
I02 remains open for candidate fit/selection and native-semantic stop/go.
Provisional WSL2 evidence is approved for the I02 platform criterion.**
**GO:** continue native validation. **NO-GO:** freeze a native/PuyuDB format or
start I03 until the remaining gates below receive explicit disposition.
No engine, storage, replication, or production-safe decoder is delivered here.

## Scope and versions

I01 was closed after GitHub's reporting API returned `enabled: true`. The
maintainer then explicitly requested latest stable releases and inclusion of Loro.
The official Rust stable manifest reported **1.99.1**, dated 2026-10-15, so both
the pinned compiler and workspace minimum moved to that release. Older compiler
support is not a goal. Direct candidate pins:

| Candidate | Version / license | Actual evidence |
| --- | --- | --- |
| Automerge | 0.12.0 / MIT | Seed byte/hash fixtures across independent processes, text-only operation inspection, convergence/history/Unicode tests, W01/W02 |
| Yrs | 0.28.0 / MIT | Shared offline trace, retained historical snapshots, explicit GC negative, Unicode anchor profile comparison |
| Loro | 1.17.1 / MIT | Shared offline trace, full-history snapshot/checkout, Unicode anchors and historical forks |
| diamond-types published | 1.0.0 / ISC | Broken no-LZ4 build and vulnerable default dependency recorded; not executed |
| diamond-types upstream | Git `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`, manifest 2.0.0 / ISC | Safe no-LZ4 build, shared offline/duplicate trace, full reload and portable historical cuts pass |

The first four rows describe latest **published stable direct releases**; the
additional DT experiment is an explicitly authorized, full-revision-pinned
upstream exception, not a published stable 2.0 release. None implies that every
transitive package has the same version or is independently maintained.
See the [dependency ledger](../DEPENDENCIES.md) for exact exceptions/risks.
The later [common comparison](I02-candidate-comparison.md) adds executed W01/W02
and cross-process native full-seed artifacts for all four candidates; it does
not erase the differences in validation depth recorded below.

All test implementations and inputs are newly written. Three source-informed
adversarial cases now have [explicit provenance](I02-sources.md); no upstream
implementation, test source, paper presentation, or corpus was copied.

## Reproduce

From the repository root with the pinned toolchain and provisioned dependencies:

```sh
cargo test -p puyudb-crdt --frozen
cargo test -p puyudb-crdt --test comparison --frozen
cargo test --release -p puyudb-crdt --test mixed_histories thousands_of_seeded_partition_duplicate_restart_schedules --frozen -- --ignored
cargo test -p puyudb-crdt --example native_allocations --frozen
cargo build --release -p puyudb-crdt --example native_workloads --frozen
target/release/examples/native_workloads w01
target/release/examples/native_workloads w02
cargo run --release -p puyudb-crdt --example native_allocations --frozen
```

For W02 process RSS/time on macOS, use the executable directly:

```sh
/usr/bin/time -l target/release/examples/native_workloads w02
```

Windows uses `target\release\examples\native_workloads.exe`.
Workloads operate in memory and print measurements; they do not create a database.
Cross-process Automerge fixtures run automatically in the seed integration tests.
The long native delivery test is intentionally ignored by the default test run;
it was also executed explicitly in release mode. Scheduled CI runs that same
command, but hosted CI has not run for these uncommitted changes.

## Seed and operation-scope evidence

The Automerge experiment fixes scalar indexing, one root text field, an epoch-owned
domain-separated seed actor, zero timestamps, fixed metadata, and operation order.
Empty and Unicode raw-change bytes/hashes are retained as fixtures. Three fresh
processes per fixture match them exactly; native save/load retains original bytes.
Other fixtures cover CRLF, NUL, combining sequences and distinct content epochs.

Writer actors bind synthetic key/branch/writer-epoch/content-epoch inputs. Those
inputs are **not authenticated**. The experiment forks the exact native intent cut
and sets the exact actor; it does not use `transaction_at`'s possible actor
substitution. Byte positions are checked against the base text before converting
to native scalar positions. Split UTF-8 scalars and UTF-16 surrogates are rejected.

The operation inspector checks native actor, metadata, dependency closure,
sequence, counter range, object identity, insertion/deletion forms and references.
Negative fixtures cover root/foreign objects, maps, counters, marks, non-text
values, wrong references, metadata changes, missing dependencies and bad checksums.
Unknown compressed chunks are refused before upstream decompression.
Follow-up negatives include a valid-checksum unknown column lost by expanded
decoding, genuine and malformed native author footers, zero/over-width/crossing
operation counters, an already deleted target, and dependencies known elsewhere
but absent from the declared cut. Size tests assert the specific early rejection
reason rather than accepting a later parse/operation-count failure. Multiple
staged primitive edits also validate as one native change.

Two empirical traps are preserved as explicit tests:

- Successful `Change::from_bytes` can accept a corrupted native checksum. Exact
  uncompressed decode/re-encode byte equality detects that fixture.
- `apply_changes` can return success while a change waits for dependencies.
  The inspector requires the exact closed native cut, not merely parse/apply
  success. This is not a PuyuDB Applied or durable receipt.

The inspector trusts an already validated base and is **not a hardened admission
decoder**. Exact encoder roundtripping is a deliberately narrow experiment, not a
proof of general Automerge canonical encoding or bounded adversarial allocation.

## Convergence and history evidence

- Three concurrent first edits: all six delivery orders, duplicates and reloads.
- Observed delete versus unseen insertion: both replicas retain `aXdef`.
- Stale intent-base byte ranges: newer branch state does not reinterpret them.
- All eight frontiers of a three-change history survive compressed/uncompressed
  native full saves and retain original native changes; incremental export is
  also replayed into an independently seeded document.
- 64 deterministic generated six-change/three-actor histories, all 90 topological
  schedules each: **5,760 schedules**. Seed: `0x50555955`.
- Both cursor affinities at every indexed baseline position, explicit start/end
  anchors, new-target availability and current/historical resolution are compared
  at all eight small frontiers after compressed/uncompressed reload. Unknown old
  targets are rejected, while known targets must resolve against the exact cut.
- Four-actor, 48-change histories include isolated writing, two partitions and
  asymmetric observation after reconnect. A short proptest gate runs normally.
  An explicit optimized gate passes **2,048 sampled schedules** across two fixed
  histories (1,024 distinct delivery orders each), with dropped first sends,
  duplicate delivery, dependency buffering, visible-state reload and replay of
  volatile pending input. Seeds: `0x505559554d495831` and
  `0x505559554d495832`; pinned rand 0.10.3.
- Original upstream-informed regressions and a paper-informed backward insertion
  diagnostic pass, with source/license pins in [the provenance record](I02-sources.md).

This verifies adapter/native consistency, not an independent proof of Automerge's
algorithm. Larger-history historical checks sample seven author-time cuts per
schedule; they do not enumerate every possible larger DAG frontier. This native
fixture receiver is not the production protocol, durable storage or I22 simulator.

## Fair challenger trace and findings

Each executed engine starts from `abcdef`. Independent offline writers perform
insert `X` at baseline position 2 and delete baseline range 1..3. After exchange
and duplicate delivery, both yield `aXdef`; native reload still exposes `abcdef`,
`abXcdef`, and `adef`. ASCII intentionally avoids differing native position units.
Each engine uses its own native identity/version types; none is relabeled as a
PuyuDB batch or version.

Yrs requires `skip_gc=true` to retain the tested deleted-history snapshots. The
default GC profile fails that reconstruction, captured by a negative test.
Its byte-offset sticky-anchor profile also fails the `A😀bc` fixture: a cursor
at byte 5 resolves to 0 after prefix insertion, instead of expected byte 6.
That is recorded as a **failed candidate profile**, not silently normalized.
An explicit UTF-16 profile with checked UTF-8 conversion passes insertion,
deletion, native anchor serialization, reload and old-view checks. This observation
does not establish an upstream bug classification or a universal Yrs limitation.

Loro uses independent `fork_at` documents, not shared `clone` handles. Change
merging and timestamps are disabled for the trace. Full snapshots, not shallow
or state-only exports, preserve the tested history. Unicode anchors pass current
and historical-fork checks. Historical forks restrict both native state and oplog;
checkout alone is not treated as proof for every deleted-anchor fallback. Its
64-bit peer IDs still need a collision/ownership design before replacing the
proposed longer actor namespace.

The earlier Yrs/Loro update probe repeated in three documents **within one
process**. The later common comparison additionally repeats native full-seed
artifacts across three fresh processes on WSL2 and macOS, with lossless retained
bytes. That native-export observation is not the PuyuDB epoch-seed binding or
original-change golden corpus used for the AM-specific experiment.

The diamond-types release does not compile with default features disabled:
`write_compressed_chunk` still references optional `lz4_flex`. Enabling defaults
requires 0.9.x, while RUSTSEC-2026-0041 fixes start at 0.11.6/0.12.1. No unsafe
comparison executable or vulnerability suppression was introduced.
The follow-up uses official upstream revision
`89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922` with defaults disabled; its genuinely
feature-gated no-LZ4 build passes. The same observed-delete, duplicate delivery,
full reload and historical-cut trace now passes using native **remote** frontiers
resolved fallibly after reload, never oplog-local indexes transferred across
replicas. Manifest version 2.0.0 is unpublished; distribution/notice and transitive
maintenance risks remain recorded, not silently waived.

## Native-only W01/W02 baseline

Original raw artifacts: [W01 samples](w01.csv), [W02 measurements](w02.csv).
These are retained first-slice measurements, not fresh current-lockfile runs.

Environment: Apple M1 Max, 10 physical cores, 32 GiB RAM, macOS 27.0.1
(26A434), ARM64, Rust 1.99.1, default Cargo release profile, Automerge 0.12.0.
One local run, not a controlled reference-Linux measurement. Filesystem/mount,
SSD model, encryption and power mode were not successfully established, so no
storage-latency conclusion follows.

The original measured lockfile SHA-256 was
`992da5ec1a0b0b3b9bfa1a3445c26d51cdfb1cd5cf4a33274630eab84c00a395`.
Inputs are highly compressible repeated ASCII, not representative repositories.

W01: one native transaction per nominal 50-ms batch; no artificial sleep,
16 warmups and 64 measured samples at each size. Nearest-rank percentiles:

| Baseline | p50 | p95 | p99 |
| --- | --- | --- | --- |
| 10 KiB | 31,250 ns | 48,541 ns | 59,041 ns |
| 100 KiB | 27,625 ns | 38,000 ns | 49,792 ns |

These are native transaction/commit wall times, excluding `edit_at`/`inspect`,
context construction, storage, durable IO and allocator instrumentation. There
is one run and only 64 samples per size; no confidence interval or stable
cross-engine speed ranking is claimed.

W02 uses a 1 MiB baseline and **100,000** committed native transactions,
including 64 KiB paste/delete/reinsert cycles. It retained 100,001 changes;
all five captured cuts were checked after native load: 0/25,000/50,000/75,000/
100,000 transactions, covering four earlier versions and the final state.

| Metric | Observed |
| --- | --- |
| Seed construction | 840,856,458 ns |
| History construction | 9,835,800,792 ns |
| Uncompressed native snapshot | 13,653,102 bytes |
| Compressed native snapshot | 291,896 bytes |
| Cold native decode from already resident bytes | 951,937,250 ns |
| Whole-process maximum resident size | 922,845,184 bytes |
| Whole-process elapsed / user / system time | 13.15 / 12.80 / 0.25 seconds |

Peak memory includes the original and reloaded native documents, retained cuts,
temporary snapshots and measurement work. It is not per-document or idle engine
RSS. Compressed size is not a universal history-storage ratio. This did **not**
measure filesystem cold loading, allocated-block growth, allocation counts, or
durable commit latency; no SQL/fsync implementation exists.

### Follow-up W01 allocation instrumentation

[Raw allocation samples](w01-allocations.csv) use a separate release example,
`stats_alloc 0.1.10` over the system allocator, with a calibration test that
observes a known 4 KiB heap allocation. No project-owned unsafe code is introduced.
The current measured lockfile SHA-256 is
`a31fc17f20dda26b52772c23d3500842fdc372e8c691102987baf73db56851b4`;
raw allocation CSV SHA-256:
`eae5cc9eeab5cd2d5867ed7422047e5de2851eab84f2485fedd20818d8031df6`.

Same 10/100 KiB repeated-ASCII inputs, 16 warmups and 64 samples per size:

| Baseline | Median allocations | p95 allocations | Median reallocations | Median allocated bytes |
| --- | --- | --- | --- | --- |
| 10 KiB | 88 | 100 | 11 | 9,768 |
| 100 KiB | 88 | 100 | 11 | 9,764 |

Measurement boundaries enclose the native transaction/commit, not context
construction, output or `edit_at`/`inspect`. Deallocation counts can include
objects allocated before the region, so these columns are **not** a net live
heap/RSS estimate. Instrumentation adds atomic-counter overhead; do not compare
its timings directly with the original uninstrumented run or infer a regression.
This is one local run without a controlled-machine confidence interval.

## Required decisions and remaining I02 gates

Local follow-up closes the named safe-comparator, larger-history, source-informed
adversarial, all-small-frontier-anchor, negative-fixture and allocation-count gaps
at the explicitly experimental native seam. It does not close every production
test family or prove bounds over all malicious encodings.

1. The maintainer [approved provisional WSL2 substitution for I02](../adr/01-wsl2-validation-scope.md).
   An unavailable Linux PC no longer blocks this milestone's platform evidence.
   [The cost assessment](I02-cost-assessment.md) records the engineering GO
   recommendation, retained raw samples and limitations; owner cost acceptance
   remains part of the stop/go. Independent reference Linux is deferred, not
   declared passed, and must be revisited before Linux production claims/I36.
   Windows memory remains a sampled lower bound. Durable IO and production
   decoder/resource safety are not claimed from native-only measurements.
2. Accept or supersede the [proposed ADR-01 evidence addendum](../adr/01-native-text-evidence.md)
   before I02 closes or I03 starts, after candidate-fit evaluation. The common
   cost baseline does not select an engine or prove original per-update
   retention, transaction boundaries or hostile operation-scope validation.
   The existing AM-specific proposed
   clarification: **native change sequences are gapless per actor; native operation
   counters start after the exact base's maximum counter**, so observing another
   actor permits counter jumps. Outer batch sequence/identity stays separate.
   Metadata syntax, counter widths, native/seed versions and decoder/resource
   limits are exact I03 profile deliverables, not a demand to finish I03 first;
   the chosen candidate's semantic direction must be accepted first. Do not
   impose AM native counter rules on other engines. The current AM inspector is
   not a production-safe input boundary.

The maintainer has native Windows x86-64 and Debian WSL2 available.
The [platform collection guide](I02-platform-runs.md) transfers the current
uncommitted source without a push and collects separate, fingerprinted reports.
Its complete workflow and 13 contract tests passed on all three platforms.
The first Windows failure exposed CLI executable-name drift; explicit `puyu`
naming fixed the existing snapshots. WSL2 remains Linux-VM evidence; its
provisional I02 use is now explicitly approved rather than silently equated
with independent reference hardware.

No eager-seed fallback or replacement engine was selected. Current evidence does
not justify an architectural substitution or a public native-format freeze.

## Primary sources

- [Rust stable distribution manifest](https://static.rust-lang.org/dist/channel-rust-stable.toml).
- [Automerge 0.12.0 metadata](https://crates.io/api/v1/crates/automerge/0.12.0),
  [native change decoding](https://docs.rs/automerge/0.12.0/src/automerge/change.rs.html),
  [native transactions/history](https://docs.rs/automerge/0.12.0/src/automerge/automerge.rs.html),
  [expanded operations](https://docs.rs/automerge/0.12.0/src/automerge/legacy/mod.rs.html),
  [chunk parsing/checksum](https://docs.rs/automerge/0.12.0/src/automerge/storage/chunk.rs.html).
- [Yrs 0.28.0 API/source](https://docs.rs/yrs/0.28.0/yrs/),
  [native sticky indices](https://docs.rs/yrs/0.28.0/src/yrs/sticky_index.rs.html).
- [Loro 1.17.1 API](https://docs.rs/loro/1.17.1/loro/),
  [pinned public source](https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro/src/lib.rs),
  [cursor fallback](https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/loro.rs#L3249-L3370).
- [diamond-types 1.0.0 release source](https://docs.rs/crate/diamond-types/1.0.0/source/),
  [RUSTSEC-2026-0041](https://rustsec.org/advisories/RUSTSEC-2026-0041.html).
- [Pinned diamond-types upstream](https://github.com/josephg/diamond-types/tree/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922),
  [fixture/source provenance](I02-sources.md),
  [stats_alloc 0.1.10](https://docs.rs/stats_alloc/0.1.10/stats_alloc/).
