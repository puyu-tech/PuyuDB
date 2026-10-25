# I02 common candidate comparison, revision 2

**Status: bounded comparative baseline complete; no engine selected and I02 open.**
All four candidates completed the common trusted native workload on Debian WSL2
and macOS ARM64. This supplies comparable observations, not equal PuyuDB-fit
validation or a universal performance ranking. No I03 format decision is made.

## Scope and provenance

Issue I02; native text/history and resource evidence corresponding to T01/T02/T09.
The maintainer explicitly requested comparison before accepting the proposed
Automerge direction. [Provisional WSL2 evidence is approved](../adr/01-wsl2-validation-scope.md);
native Linux support and production resource guarantees are not.

| Candidate | Tested native profile |
| --- | --- |
| Automerge 0.12.0 | Explicit scalar indexing; fixed epoch seed; distinct fixture writer; zero edit timestamps; no edit messages; compressed native full save |
| Yrs 0.28.0 | Explicit UTF-16 indexing, `skip_gc=true`; seed client 1 imported into writer client 2; full update V1; historical `Snapshot` including delete set |
| Loro 1.17.1 | Seed/writer peers 1/2; timestamps and change merging disabled; no edit messages; full non-shallow native snapshot |
| diamond-types | Git `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`, unpublished manifest 2.0.0; defaults/LZ4 disabled; full oplog; owned remote cuts; one retained live branch |

These are **native cost profiles**, not authenticated PuyuDB bindings. Candidate
identities, native transaction models and encoders differ. In particular, a DT
delete+insert pair is a logical workload batch, not an invented native atomic
transaction. Native operation/change counts are not equated across candidates.
No native full snapshot is labeled a PuyuDB authoritative batch ledger.

Both recorded runs used Rust/Cargo 1.99.1, LLVM 23.1.1, the default Cargo release
profile, no `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS`, and the same input inventory:

- Source fingerprint: `74ed9166b8698401474f4f4773d851f4cfc2f61a138a7746b70bc4073bdf6f4a`.
- Input archive SHA-256: `9549bac3d4a2307a1ff6fe013365380a8dd6c3c8b944ef4ab99ade4790b9d5da`.
- Cargo.lock SHA-256: `a31fc17f20dda26b52772c23d3500842fdc372e8c691102987baf73db56851b4`.
- WSL2 run: `wsl2-20261025T212021310067Z`.
- macOS run: `macos-native-20261025T212021010421Z`.

WSL2 is Debian 12, x86-64, kernel `6.18.54.1-microsoft-standard-WSL2`, ext4,
12 guest logical CPUs on the previously documented Ryzen 5 3600 host.
macOS is 27.0.1/Darwin 27.0.0 on the previously documented M1 Max host.
Background load, host/guest scheduling and allocator behavior are uncontrolled.
These are not the original controlled native-Linux reference class.

Each run completed **40 steps**: eight setup/check/build/release-fixture steps,
then 32 standalone candidate processes. All exit codes were zero, all output
contracts validated, and no candidate timed out. Reports say
`comparison_recorded` and `reference_gate: not_evaluated`, not engine accepted.
Each workspace run passed 51 integration tests; both default-ignored release
fixtures were executed explicitly and passed. Python: 21 tests passed on
macOS/WSL2. Additional native Windows checks passed 51 integration tests and
20 Python tests, with the Unix-only process-group test skipped. **The common
Windows performance workload was not executed.**

Revision-1 exploratory runs are excluded from these tables. Revision 2 resolves
review findings concerning deferred materialization, Unicode starting offsets,
candidate-failure continuation, process-group cleanup and CSV headers. Later
report/status/CSV-checkout-policy edits do not change the measured Rust or
collector source. Artifact CSVs are checked out with LF even on Windows.

## Raw data and reproducibility

- [WSL2 raw candidate CSVs and sanitized manifest](candidates/wsl2-v2/README.md).
- [macOS raw candidate CSVs and sanitized manifest](candidates/macos-v2/README.md).
- [Per-process W01 summary](candidates/w01-summary.csv).
- [W02 summary](candidates/w02-summary.csv).
- [Lossless native seed artifacts and signatures](candidates/seeds.csv).

Each profile directory contains 32 byte-identical stdout CSV copies. Its
`manifest.json` records source/lock/compiler identities, input-file hashes,
step results/resources, seed-repeatability outcomes, and per-CSV lengths/hashes.
Commands with local absolute paths and raw stderr were not copied wholesale.
Complete raw results and W02 native artifacts remain locally:

```text
target/i02-comparison-remote/wsl2-20261025T212021310067Z/
target/i02-baselines/macos-native-20261025T212021010421Z/
```

Cloning the repository preserves the CSVs/manifests/seed bytes, not these ignored
full result directories or large native history artifacts. No private project
contents, credentials, full environment dump or architecture input is in the
generated evidence. Collection and evidence export performed no commit, merge,
push, publication or deployment. A later local checkpoint does not accept the
engine or change the recorded measurement inputs.

Reproduce using the [platform guide](I02-platform-runs.md):

```sh
python3 scripts/i02_baseline.py collect --profile wsl2 --comparison
```

Use `--profile macos-native` locally, or the native Windows profile where available.
Provisioning is locked; subsequent Cargo commands are offline. Repeated process
order rotates. A failed candidate case is recorded without skipping the others;
setup/source failures still stop. Case timeouts are 300 seconds.

## Measurement boundaries

W01 uses the identical repeated-ASCII 10/100 KiB inputs and sequential localized
replacements, 16 warmups and 64 measured samples per size, in **three fresh
processes per candidate**. W01 reads the complete UTF-8 text after each edit;
W02 does not impose a full-text render after every edit.

- `coordinate_ns`: byte-boundary validation and scalar/UTF-16 coordinate conversion
  against the independent oracle. This is **not** a production historical-view
  lookup, authorization, inspection or admission adapter.
- `native_ns`: native mutation and required transaction completion, when present.
- `call_ns`: enclosing benchmark-adapter call. It includes API lookup/preparation
  outside the narrower native interval.
- `materialize_ns`: full current UTF-8 text construction, separately timed.
- The displayed **call + text** observation is calculated for each sample as
  `call_ns + materialize_ns`, then summarized. It excludes coordinates, oracle
  checking and unrelated loop work. It is not durable or full PuyuDB API latency.

DT can defer rope work to later edits/reads. No artificial per-edit DT flush is
imposed in W02. Seed text is read before editing; native load and UTF-8 output
are separate stages. Historical queries start from a fresh full load and
separately measured latest-text read, so prior-cut view caches are not reused.
Historical reads include candidate-native reconstruction/checkout and UTF-8 output;
Yrs needs snapshot export plus a new document/import, whereas Loro uses checkout.
These method differences are part of the cost, not hidden as identical operations.

Allocations use a **separate executable** installing `stats_alloc`; the region
covers the adapter's native call, not coordinates or later text materialization.
Its timings are not mixed with the uninstrumented samples. A zero count can be
a real no-allocation region, not an unavailable memory metric. In particular,
DT may allocate while flushing/materializing outside this region; zero native-call
allocations do not establish allocation-free edit-to-text completion.

W02 performs the same 1 MiB/100,000 logical-batch trace, including 64 KiB
paste/delete/restore. Its 11 cuts are after
0/1000/1001/1002/2000/2001/2002/25000/50000/75000/100000 batches: ten earlier
states and the final state, including the short deleted-content intervals.
Expected strings come from an independent `String` oracle; the collector checks
their hashes against a separate Python byte-array trace. Every candidate matched
all cuts and the final digest.
The quarter-history cuts share the same text digest because the trace repeats;
they are distinct causal cuts, not four independent different text contents.

## Descriptive results

Nearest-rank p95 selects rank 61 of 64 within each process. The following primary
WSL2 table gives the **median of the three process p95s**, plus their observed
range. Samples are not pooled across processes, sizes, engines or platforms.
These are small sequential samples, not confidence intervals, independent-run
tail guarantees, or a stable regression baseline.

### W01, 100 KiB, nanoseconds

| Candidate | Native API p95, median of 3 | Call + text p95, median of 3 | Call + text p95 range across processes |
| --- | ---: | ---: | ---: |
| Automerge | 101,059 | 2,975,338 | 2,955,340–3,017,038 |
| Yrs | 3,105 | 6,332 | 6,322–8,596 |
| Loro | 3,557 | 12,544 | 12,153–17,954 |
| diamond-types | 561 | 7,504 | 7,424–8,636 |

These numbers distinguish tiny edit calls from complete-text output. They do not
measure incremental patches or editor UI latency, and do not imply a universal
engine ordering. The 10 KiB and macOS results remain in the summary and raw CSVs.

### W02, one WSL2 process per candidate

All times are nanoseconds; byte columns are not heap estimates.

| Candidate | Seed/writer setup | History construction | Native artifact bytes | Native load | Latest UTF-8 text | Whole-process max RSS bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Automerge | 1,230,979,333 | 13,954,208,103 | 5,615 | 1,856,456,688 | 26,958,157 | 395,202,560 |
| Yrs | 3,629,543 | 53,161,361,672 | 3,561,915 | 58,839,017 | 541,263 | 1,147,621,376 |
| Loro | 2,846,207 | 398,742,318 | 87,214 | 4,737,105 | 3,195,450 | 192,315,392 |
| diamond-types | 1,967,031 | 107,179,655 | 2,659,695 | 26,405,514 | 193,352 | 64,704,512 |

History time includes common oracle work, coordinate conversion, cut capture and
cut checks, not only native mutation. Seed rendering is separate in raw/summary
data. Maximum saved-cut read times were 232,522,826 / 1,340,241,807 /
997,123,620 / 10,143,617 ns respectively; these are maxima over heterogeneous
cuts, not a sampled p95 distribution.

RSS is the whole native workload process, including the original writer, native
artifact, retained expected texts, fresh loads and native historical temporaries.
It excludes Cargo/the collector, and is not per-document, idle-space or pure heap
memory. GNU time KiB values were converted to bytes; macOS uses Darwin byte units.
No common Windows peak comparison is inferred from earlier sampled lower bounds.

Artifact sizes use different native encoders/compression and repetitive input.
They **exclude a PuyuDB ledger and external original per-update retention**.
The common Automerge profile omits per-edit binding messages and changes fixture
writer identity; its 5,615-byte save must not be compared with the older
Automerge-specific 291,896-byte save as an optimization or storage regression.
No change in that earlier profile or evidence was made.

### Native seed repeatability

All four profiles produced byte-identical native full-history seed artifacts for
three baselines in three fresh processes on each of two platforms. The bytes also
matched between those two platforms. [The retained hexadecimal artifacts](candidates/seeds.csv)
are lossless observations of those native exports.

This is stronger than the earlier same-process challenger probe, but **not**
a frozen epoch seed specification. It does not by itself prove native actor
ownership, candidate-specific dependency admission, future-version compatibility,
or original per-change byte retention.

## PuyuDB-fit matrix: separate from native costs

“Not tested” is not “failed.” An observed failure applies to its named profile.

| Gate | Automerge | Yrs | Loro | diamond-types Git |
| --- | --- | --- | --- | --- |
| Common observed-delete/duplicate/reload trace | Observed pass | Observed pass, GC off | Observed pass, full history | Observed pass, remote cuts |
| New shared Unicode edits and delete-only cuts after reload | Observed pass | Observed pass, UTF-16 | Observed pass | Observed pass |
| Common W01/W02, 11 cuts including deleted intervals | Observed pass | Observed pass | Observed pass | Observed pass |
| Native full seed byte repetition, fresh processes | Observed pass | Observed pass | Observed pass | Observed pass |
| Epoch-seed binding feasibility and original-change golden corpus | AM-specific experimental evidence | Not tested | Not tested | Not tested |
| Larger mixed-observation convergence corpus | Existing AM-specific evidence | Not tested equivalently | Not tested equivalently | Not tested equivalently |
| Unicode/historical anchors | Broad AM-specific evidence | UTF-16 fixture passes; byte profile fails | Existing fixture passes | Equivalent gate not tested |
| Original per-change bytes retained through save/load | Existing small AM corpus passes | Not tested | Not tested | Not tested |
| Operation-scope/metadata/dependency inspection negatives | Experimental AM corpus | Not tested | Source-supported APIs; validator not tested | Source-supported APIs; validator not tested |
| Outer immutable batches, authenticated writers, durable admission | Not delivered | Not delivered | Not delivered | Not delivered |

A native full save proving old text is not proof of original update bytes or
application transaction boundaries. Yrs aggregates updates; Loro's change merging
is disabled here; DT run-length-encodes operations and has no equivalent outer
batch ledger. Candidate-native client/peer widths and provenance/binding costs
need explicit designs, not forced Automerge identity types.

## Recompute from retained data

Python standard library only; the command verifies raw-copy hashes and prints
the three independent 100 KiB process p95s plus the median used above:

```sh
python3 - <<'PY'
import csv, hashlib, json, statistics
from pathlib import Path

root = Path("docs/validation/candidates")
for folder in ("wsl2-v2", "macos-v2"):
    directory = root / folder
    manifest = json.loads((directory / "manifest.json").read_text())
    assert len(manifest["files"]) == 32
    for name, expected in manifest["files"].items():
        data = (directory / name).read_bytes()
        assert len(data) == expected["bytes"]
        assert hashlib.sha256(data).hexdigest() == expected["sha256"]
    for engine in ("automerge", "yrs", "loro", "diamond-types"):
        native, ready = [], []
        for trial in range(3):
            path = directory / f"candidate-{engine}-w01-{trial}.csv"
            with path.open() as stream:
                rows = [r for r in csv.DictReader(stream)
                        if r["baseline_bytes"] == "102400"]
            assert len(rows) == 64
            native.append(sorted(int(r["native_ns"]) for r in rows)[60])
            ready.append(sorted(int(r["call_ns"]) + int(r["materialize_ns"])
                                for r in rows)[60])
        print(folder, engine, native, ready,
              statistics.median(native), statistics.median(ready),
              min(ready), max(ready))
PY
```

This recomputes descriptive results, not a new workload execution. The fixed
run directories, sizes, sample counts and percentile definition are intentional
parameters of this experiment; no inferential statistical claim follows.

## Interpretation and next bounded step

The additional candidates are not rejected. Loro and DT show favorable native
cost observations in these workloads; Yrs's retained long-history trace is
substantially costlier than its tiny W01 edits. That is not a whole-product verdict.
Automerge remains the deepest tested fidelity/reference path, not an automatically
selected winner. No upstream algorithm is independently proved by these tests.

Before accepting an engine, prioritize a bounded **Loro PuyuDB-fit experiment**:
operation-scope inspection with hidden-operation negatives, identity/epoch binding
feasibility, and exact original-update/transaction-boundary retention/cost.
Keep Automerge as the reference candidate; DT's unpublished Git/notice/maintenance
risks remain, and Yrs's GC/anchor/history-profile observations remain explicit.
Any engine substitution needs an evidence-backed ADR, not a benchmark-only switch.

I02 stays open for that fit/decision work. The proposed Automerge counter
clarification is conditional on selecting that binding; it must not be imposed
on other candidates as a fake common rule. I03 has not started.
