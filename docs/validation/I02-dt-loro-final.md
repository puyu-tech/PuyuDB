# I02 final bounded DT–Loro comparison

**Collection status: complete on macOS ARM64, native Windows x86-64 and
Debian WSL2.** All joint checks, measurements and malformed-input screening
passed on every platform, and the retained evidence passed an independent
hash/consistency audit. **Decision (2026-10-26): the maintainer selected DT
with the small maintained patch set. I02 is closed. I03 has not started and no
native/PayuDB format is frozen.**

## Question and tested scope

Does DT remain the preferable long-term candidate when its two small native
extensions and complete experimental binding are exercised, rather than comparing
bare engine timings with an unmeasured admission implementation?

Executed on 2026-10-26 with Rust 1.99.1:

- DT manifest 2.0.0, unpublished Git
  `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`, native defaults off, plus the
  [existing two patches](../../experiments/diamond-types-interfaces/README.md).
- Loro 1.17.1, defaults off, using the
  [complete existing fixture inspector](../../crates/puyudb-crdt/tests/support/loro_fit.rs#L235).
- A [separate generated harness](../../experiments/dt-loro-final/README.md),
  not a main-workspace dependency change or a production binding.

The native patch footprint remains **150 added / 18 removed lines in five files**.
No patch, merger, decoder, native encoding, dependency pin or main lock was changed
to obtain these results. Original main-workspace characterization tests still
exercise the unmodified DT dependency.

## Method fixed before formal measurement

The [protocol](../../experiments/dt-loro-final/README.md#protocol-fixed-before-measurement)
and [runner](../../experiments/dt-loro-final/run.py#L1) retain raw outcomes, not a
weighted engine score:

- Seven primary cells: 10 KiB ASCII and 100 KiB Unicode, each with 0 / 1,000 /
  10,000 trusted-generated prefill batches; plus 1 MiB Unicode paste/delete churn
  with 1,000 prefill batches.
- Exactly 64 measured updates per completed process, rotating four writers.
  Every measured update is fully authored, inspected and published.
- Prefill is native-generated trusted history, **not** 10,000 inspected admissions.
  Its seed/original updates/digests are retained and counted separately.
- Three fresh processes per engine/cell; alternating paired engine order.
  No simultaneous measured workloads on the same host, sample removal, discarded
  warmup, cross-process pooled percentile or timing-driven workload retuning.
- Nearest-rank p50/p95 within each 64-sample process: ranks 32 and 61. Tables use
  the descriptive median of the three process percentiles; all three remain in
  retained data. This is not an inferential confidence interval or a supplied SLO.
- Failed cells remain failed. Partial output is retained but is not a completed
  process distribution and is not assigned a comparable p95.

Current source constructs baselines of **10,240 / 102,400 / 1,048,576 UTF-8
bytes**. A pilot summary quoted slightly smaller Unicode lengths; those were
stale and are not the final-source budgets. Prefill/edits subsequently change
rendered byte lengths.

### What the clocks include

The [timed flow](../../experiments/dt-loro-final/compare.rs#L819-L921) includes:

1. Two anchor creations.
2. Exact-base capture and required clone/fork/import, authoring, byte-to-scalar
   edit conversion, encoding and the fixture digest.
3. Complete binding inspection, private staging and captured-base destruction.
4. Publishing the inspected stage, including destruction of the previous state.
5. Four anchor resolutions, historical checkout/fork and text materialization.
6. A full native save and load on **every update**. Load also verifies reloaded
   text, anchors and an older cut, then destroys its temporary document/snapshot.
7. Additional outer SHA-256, original-byte copying/indexing and temporary disposal.
8. Remaining native materialization/comparisons and per-sample scratch disposal.

Independent oracle edit computation, selection of the fixture's scalar anchor
gap, reporting hashes, retained oracle-text copies and output I/O are outside.
The total is therefore the **complete scripted experimental flow**, not a bare
keystroke, parser-only load, durable transaction or finished PuyuDB API latency.
The save/load and verification work must not be silently removed when interpreting
these totals.

DT uses exact-tip native clones and strict original-field inspection. Loro uses
the existing complete snapshot/fork/prefix-JSON/replay/re-encoding fixture.
These are materially different implementations of the tested binding obligations.
The measurements do not prove the same speed ratio is inherent to either CRDT,
or that an optimized Loro inspector would retain this cost.

## Preserved original churn failure and one supplemental cell

The correctness pilot found an existing Loro **one-native-change fixture**
constraint. The original `churn1m` combined replacement deletes 4,096 scalars and
inserts 4,096 scalars in one authored batch. Native export can split this into
two changes; the complete inspector correctly refuses its current one-change
profile. Large 16,385-scalar paste-only/delete-only operations passed.

The original seven-cell matrix was **not** replaced or weakened. Before formal
measurements, exactly one supplemental cell was recorded:
`churn1m_bounded`, the same baseline/prefill/schedule/large paste/delete operations,
with only the combined replacement reduced to 512 scalars, for **both** engines.
It receives the same three-process treatment.

Thus there are **48 attempted processes per platform**, not a redefined successful
42-process primary matrix. A complete observed platform has 45 successes and
three retained original Loro failures. Each original Loro failure occurs at sample
2 after two output rows and reports `original_blob_meta.change_num = 2`.
There is no comparable complete-flow p95 for that cell.

This is **not a native Loro editing/convergence failure or a universal 4,096-scalar
edit limit**. It identifies work needed to broaden this particular inspector to
validate multiple contiguous native changes within one outer batch. Such a change
would require its own correctness evidence; this experiment did not implement it.
The pinned source's relevant code is `loro-internal/src/oplog/change_store.rs`
(`MAX_BLOCK_SIZE`, `split_when_exceeds`, `split_change_then_insert`).

## Correctness and compatibility

The [integrated corpus](../../experiments/dt-loro-final/compare.rs#L1) uses
32 deterministic histories × 24 authored batches × four delivery schedules,
per engine. Each original is inspected at its **declared author base** before
native integration into a potentially newer receiver. Deferred predecessors,
duplicates, partition/rejoin delivery and intermediate reloads are exercised.

Per engine, per explicit release `correctness` execution:

| Observation | Count |
| --- | ---: |
| Authored batches | 768 |
| Complete inspections, including schedule rechecks | 3,840 |
| Receiver publications | 3,072 |
| Deferred arrivals | 1,582 |
| Exact outer duplicate deliveries | 3,072 |
| Reloads | 768 |
| Historical checks | 12,288 |
| Anchor checks | 26,112 |

Sequential/unambiguous histories use an independent scalar/string oracle.
Concurrent native tie-breaks are not forced to match between different engines.
Identity, saved-cut, historical text and anchor import/reload invariants are
checked within each engine; this is bounded execution evidence, not a proof of
either upstream algorithm.

Five integrated tests, three screening controls, the unchanged seven native-anchor
regressions and sixteen strict-input regressions run in both debug and release on
all three platforms: **31 unique tests per profile**. Only duplicate embedded comparison-test executions
inside the screening binary are filtered; the standalone comparison target runs
all five.

Fixed controls include foreign ownership, context/digest/stale-cut rejection,
caller immutability, native span/parent/counter rules, outer equivocation,
opposite-arrival-order originals and the deleted-anchor witness. The new DT
strict path is not the disproven whole-byte reconstruction guard.

### Cursor policy is a decision, not a performance bug

For `abc → aZc`, anchors at old gap 1 resolve:

| Policy | Left | Right |
| --- | ---: | ---: |
| Tested DT identity-boundary adapter | 1 | 2 |
| Tested Loro native cursor | 2 | 2 |

Each is checked against its own native policy and import/reload invariants.
The difference must be explicitly accepted in the selected PuyuDB profile.
No alternate merger or numerical repair is used to force agreement.

## Complete-flow observations

All entries below are **median of three process p95s, milliseconds**; smaller is
faster. Raw integer nanoseconds are the source of truth.

### macOS ARM64

Apple M1 Max, 32 GiB RAM, 10 logical CPUs; Darwin 27.0.0. Hardware was queried
after collection, not continuously monitored. Background load/power mode are not
controlled reference-hardware guarantees.

| Case | Prefill | DT flow | Loro flow | Loro / DT |
| --- | ---: | ---: | ---: | ---: |
| ASCII 10 KiB | 0 | 0.341 | 2.594 | 7.61× |
| ASCII 10 KiB | 1,000 | 3.510 | 21.022 | 5.99× |
| ASCII 10 KiB | 10,000 | 34.868 | 185.172 | 5.31× |
| Unicode 100 KiB | 0 | 1.251 | 7.658 | 6.12× |
| Unicode 100 KiB | 1,000 | 4.634 | 47.666 | 10.29× |
| Unicode 100 KiB | 10,000 | 37.070 | 387.224 | 10.45× |
| Original 1 MiB churn | 1,000 | 16.195 | Profile rejection | — |
| Supplemental bounded churn | 1,000 | 15.607 | 446.546 | 28.61× |

DT's complete tested flow is faster in every paired completed macOS cell.

### Native Windows x86-64

Windows 11 Professional (build 10.0.26200), AMD64, AMD Ryzen 5 3600 6-Core
Processor, 12 logical CPUs, host RAM 17,101,266,944 bytes (about 15.9 GiB);
Python 3.14.7, rustc host `x86_64-pc-windows-msvc`. Hardware was queried after
collection, not continuously monitored.

| Case | Prefill | DT flow | Loro flow | Loro / DT |
| --- | ---: | ---: | ---: | ---: |
| ASCII 10 KiB | 0 | 0.694 | 5.564 | 8.02× |
| ASCII 10 KiB | 1,000 | 7.567 | 43.095 | 5.70× |
| ASCII 10 KiB | 10,000 | 63.021 | 372.608 | 5.91× |
| Unicode 100 KiB | 0 | 3.093 | 12.424 | 4.02× |
| Unicode 100 KiB | 1,000 | 8.363 | 85.147 | 10.18× |
| Unicode 100 KiB | 10,000 | 61.017 | 703.047 | 11.52× |
| Original 1 MiB churn | 1,000 | 28.845 | Profile rejection | — |
| Supplemental bounded churn | 1,000 | 28.319 | 841.644 | 29.72× |

### Debian WSL2

Debian 12 (bookworm) on WSL2 kernel 6.18.54.1-microsoft-standard-WSL2, x86_64,
AMD Ryzen 5 3600 6-Core Processor, 12 logical CPUs, guest MemTotal 8,094,512 kB
(about 7.7 GiB), ext4; Python 3.11.2, rustc host `x86_64-unknown-linux-gnu`.
Hardware was queried after collection. WSL2 is the maintainer-approved I02
substitute, not independent native Linux.

| Case | Prefill | DT flow | Loro flow | Loro / DT |
| --- | ---: | ---: | ---: | ---: |
| ASCII 10 KiB | 0 | 0.463 | 4.370 | 9.44× |
| ASCII 10 KiB | 1,000 | 4.973 | 31.864 | 6.41× |
| ASCII 10 KiB | 10,000 | 48.203 | 290.823 | 6.03× |
| Unicode 100 KiB | 0 | 1.687 | 13.287 | 7.88× |
| Unicode 100 KiB | 1,000 | 6.137 | 63.288 | 10.31× |
| Unicode 100 KiB | 10,000 | 50.595 | 526.492 | 10.41× |
| Original 1 MiB churn | 1,000 | 22.751 | Profile rejection | — |
| Supplemental bounded churn | 1,000 | 22.470 | 546.505 | 24.32× |

DT's complete tested flow is faster in every paired completed cell on all three
platforms (4.02×–29.72×). The ranking is consistent across hosts, but absolute
times differ: these are three different machines, not replicated reference
hardware.

### Cross-platform stage pattern

The same stage pattern holds on all three platforms; that does **not** make every
operation faster:

| Unicode 100 KiB, 10,000 prefill | macOS DT | macOS Loro | Win DT | Win Loro | WSL2 DT | WSL2 Loro |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Complete inspection, including base teardown | 2.466 | 126.205 | 4.702 | 188.013 | 4.103 | 161.177 |
| Creation of two anchors | 5.623 | 0.007 | 9.465 | 0.010 | 8.071 | 0.009 |
| Resolution of four anchors | 11.090 | 7.529 | 18.025 | 13.615 | 14.967 | 12.196 |
| Load + verification + teardown | 14.270 | 125.451 | 22.890 | 241.673 | 19.893 | 170.288 |

Loro is markedly better at native cursor creation on every platform; DT's
root-history reconstruction is a real cost and is not a proposed per-keystroke
implementation. The earlier bare-native load comparison is a different metric
from this verified load flow.

## Memory and retained bytes

Whole-process RSS includes original artifacts, oracle/checking state, native
documents and temporaries; it is not pure native heap or idle per-document cost.
macOS/WSL kernel lifetime maxima are comparable within each host. Windows live
peak samples are **lower bounds**, not equivalent lifetime-maximum observations.

Median of the three processes per engine and cell:

| Case / prefill | macOS DT | macOS Loro | Win DT* | Win Loro* | WSL2 DT | WSL2 Loro |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| ASCII 10 KiB / 0 | 4.45 | 8.44 | 6.14 | 10.02 | 3.36 | 6.84 |
| Unicode 100 KiB / 10,000 | 32.19 | 84.28 | 23.71 | 65.13 | 21.84 | 66.47 |
| Supplemental 1 MiB / 1,000 | 71.11 | 76.25 | 31.28 | 42.97 | 32.23 | 52.32 |

\* Windows values are live peak working-set samples (lower bounds).

Retained byte components at the last sample are deterministic observations,
not timing statistics or total persistent storage. They are byte-identical
across all three platforms (independent evidence audit):

| Unicode 100 KiB / 10,000 | DT bytes | Loro bytes |
| --- | ---: | ---: |
| Original seed + prefill + measured updates | 2,152,490 | 1,822,893 |
| Digest values alone | 322,080 | 322,080 |
| Full native snapshot/cache, separate component | 168,276 | 65,162 |

Loro's native compression and more compact artifacts remain real advantages.
The repetitive input is compressible. Framing, signatures, indexes, outer
publication records, SQLite/WAL and durability are absent; do not call a sum of
these columns the database's storage requirement.

## Bounded malformed-input/resource screening

The [screening protocol](../../experiments/dt-loro-final/security-notes.md) runs
each trial in a fresh release child. It preserves original candidate bytes before
decoding and distinguishes native decoding from the restricted binding.

Per platform: **512 DT-binding + 512 DT-native + 480 Loro-binding + 480 Loro-native
= 1,984 trials**. These include every small-update truncation, valid controls,
raw/checksummed mutations, trailing/concatenated inputs, unsupported fields,
extreme lengths/varints, fixed-seed random bytes and authentic Loro expansion
controls. Outer digests are recomputed so rejection is not just a stale SHA check.
Fixture CRC-32C/XXH32 helpers are checked against native exports.

All three platform screening collections report no caught panic, process crash,
timeout, forbidden/unknown base-state observation or must-accept/must-reject
violation. Native successful imports may change state; that is expected, not a
caller-mutation failure of staged binding inspection. Changed bytes accepted as
legal variants are not automatically invalid.

The outcome counts are identical on all three platforms. The binding outcomes
are DT **6 accepted / 506 returned errors** and Loro **3 accepted / 477
returned errors**, with no binding-base mutation. Raw native decoding accepts
more variants by design (DT 19 accepted / 493 returned errors; Loro 15 / 465).
These are corpus outcomes, not acceptance-rate or relative-security scores.

Resource limits are qualified:

- Three-second wall watchdog per child.
- macOS applied CPU/core limits but refused `RLIMIT_DATA`; **no effective hard
  memory bound** was established there.
- WSL2's recorded probe observed `RLIMIT_AS` 1 GiB plus CPU/core limits applied.
- Native Windows has the wall watchdog, not a supplied hard memory bound.

A pilot debug-only Loro compressed import exceeded three seconds. The real release
control passed; the debug diagnostic is not presented as a release failure.
This deterministic mutation campaign is **not coverage-guided cargo-fuzz, a
hostile-input security audit, an allocation/recursion bound or an absence-of-UB
proof**. Production decoder hardening remains required for either engine.

## Upstream regression controls

Fresh original/patched native trees were tested on macOS, with separate target
directories and byte-identical locks. Main-lock-seeded controls preserve every
main DT registry selection and add only real native-default/dev-fixture
requirements, recorded separately from the benchmark's unchanged registry graph.

| Native package suite | Original pin | Both patches |
| --- | --- | --- |
| Defaults, debug | 141 unit + 5 doctests pass; 21 ignored | Same |
| Defaults, release | 138 pass; 3 fail; 21 ignored | Same three failures |
| Patched explicit release doctests | — | 5 pass |
| No-default-features library tests | Compile failure | Same compile failure |

The three release failures are
`ost::content_tree::test::{content_tree_fuzz_once,replace_item,simple_inserts}`:
existing `dbg_check` deliberately panics without debug assertions. The
no-default-feature test target unconditionally imports optional `lz4_flex`.
Neither is fixed, hidden as passing, or attributed to these patches.
The selected default-off **project harness** passes its debug/release controls.

The full published Loro cache lacks 32 literal fixture includes and a path-only
upstream development module. No repository clone, substitute fixture or purported
complete Loro-upstream test pass is supplied. This asymmetry is explicit; DT's
local patches carry a rebase obligation that the published Loro integration does
not.

## Evidence, review and decision

Raw runs:

- macOS: `target/dt-loro-final/20261026T174156000949Z`.
- Windows: `target/dt-loro-final-remote/20261026T174233274505Z/windows-evidence`
  (fetched from the measurement host; transfer archive SHA-256
  `948628bd51bda1dbab0dc71a7255224e089b1d1e2ea09b6ee85b07e8b658e37c`).
- WSL2: `target/dt-loro-final-remote/20261026T174233274505Z/wsl-evidence`
  (same; transfer archive SHA-256
  `b6e8fcc156ce2cde39021b7b3d088a91ac25ff47daa47e0a828f3fc0b3d88bed`).
- Remote source/control directory:
  `target/dt-loro-final-remote/20261026T174233274505Z`.
- Upstream raw controls:
  `target/dt-loro-upstream-evidence/20261026T170851213418Z` and
  `20261026T171748056592Z-main-seeded`.

Frozen compiled-input fingerprint:
`635e469d530f331e7fba951108fa0b0e292a0cb664ce642ee2a01b28eb3689f3`,
identical in all three platform manifests. The source-transfer fingerprint
covers a larger inventory and is intentionally different. Original generated
files and checked executable bytes are sealed; summarization rejects
changed/deleted raw files rather than re-sealing them.

### Independent evidence audit

After collection, all three evidence trees were audited locally: every sealed
file matched its manifest SHA-256 and byte size (4,105 macOS / 4,103 Windows /
4,153 WSL2 files, with zero unrecorded extras outside the excluded upstream
trees), all 246 generated inputs matched, and all 11 check steps (patch
application/checks, dependency resolve, fmt, Clippy, debug/release tests,
release build and correctness) exited zero. Each `summary.json` was re-derived
independently from the raw benchmark rows with an exact match (45 completed
processes and 15 median rows per platform). Screening steps were recounted
(512/512/480/480 per platform) with no panics, violations, timeouts or
binding-base mutations. The frozen source fingerprint, upstream revision and
archive, registry graph and native feature set are identical across platforms;
per-platform executable hashes differ as expected for different targets; text
signatures and retained byte components are identical across platforms.

Independent implementation/spec reviews found and closed real defects before
formal measurement: stale-target execution, missing helper provenance, silent
evidence re-sealing, CSV ordering, Unicode byte budgets, hidden teardown,
uninspected sibling originals and ambiguous unknown-state reporting. Review
closure is not substituted for actual joint test/measurement execution.

**Recommendation:** select DT for this project's binding direction with the
small maintained patch set, subject to explicit maintainer acceptance. The
complete three-platform measurements and the audited evidence chain support
that choice; they do not justify a general claim that Loro is a worse CRDT.

**Decision (2026-10-26):** the maintainer selected DT with the small
maintained patch set. I02 is closed. Accepted with the selection: the native
identity-boundary cursor policy, the epoch-owned external empty-seed contract,
the original native-span/outer-publication association, and
patch/dependency/notice ownership. The main workspace, dependency lock and DT
pin remain unchanged; promoting the reviewed patch set into a production
binding is I03 work. Loro's full offline writer-to-peer allocation/collision
policy remains unimplemented; Loro is retained as a documented alternative,
not a rejected engine. Both candidates still need production authentication,
atomic publication, durability and decoder/resource hardening; these are not
claimed delivered by choosing an engine.

No main dependency promotion, permanent vendored engine, new persistent format,
upstream submission, push, publication or release occurred. The I02 closure is
recorded as a local checkpoint commit on the working branch only. Native Linux
production/reference-hardware support, macOS Intel, 32-bit/Wasm, native MSRV 1.96
and the complete upstream feature/platform matrix are not established here.
