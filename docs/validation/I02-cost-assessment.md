# I02 native-cost assessment

**Status: engineering assessment complete; native-profile acceptance pending.**
This is a descriptive assessment of the existing 2026-10-25 runs, not a new
benchmark execution or an approved native format.

This is the **earlier AM-specific profile assessment**, not a four-engine
selection. The subsequent [common candidate comparison](I02-candidate-comparison.md)
uses different declared cost profiles and must be considered before selection.
Do not mix its numbers with this older run as a regression or optimization.

## Question and scope decision

Does the measured native-only implementation justify continuing with the proposed
Automerge candidate, or does current evidence require rejecting it before profile
design? The maintainer has [accepted provisional WSL2 evidence for I02](../adr/01-wsl2-validation-scope.md);
independent native Linux costs are deferred, not represented as measured.

**Engineering recommendation: GO for continued candidate/profile work, subject to
explicit native-semantic acceptance; NO-GO for production latency/memory claims.**
Current measurements do not justify replacing the text engine or pretending
that historical views are free. This assessment does not authorize starting I03.

## Inputs and data quality

Primary run: `wsl2-20261025T190808885690Z`, Debian 12, x86-64, ext4 on WSL2,
Ryzen 5 3600 host, 12 reported guest logical CPUs and 8,094,516 KiB guest
`MemTotal`. The host has 6 physical cores and about 16 GiB RAM, not the original
reference class. Exact compiler/native versions, input fingerprint, environment
limits and raw-report hashes are in [the platform report](I02-platform-results.md).
The native workload source has not changed since those runs.

Retained raw CSVs for the primary cost decision:

| Artifact | SHA-256 |
| --- | --- |
| [W01 native samples](platform-wsl2-w01.csv) | `cbdae5efde96199be1a004bd2073787542745b642972b9d09ce7561cda9ec491` |
| [W01 allocator samples](platform-wsl2-allocations.csv) | `b9d0543111d396d14a7e52c73abb53906bfe178e76a154211afa100debb69839` |
| [W02 measurements](platform-wsl2-w02.csv) | `6bfa36498b7e9119766f7b476d45422c901777694e83bcc6109512fe36f839f1` |

The two sample files each have 128 rows: 64 measured transactions for each of
10,240 and 102,400 baseline bytes. Their natural key is `(baseline_bytes, sample)`;
all keys are unique, indices are exactly 0–63 per group, cells are present,
and timings are positive. No measured rows or slow samples were dropped.
Sixteen earlier warmup transactions per size are intentionally outside the CSV.
W02 has one observation per metric, not 100,000 independent timing samples.

The workload is highly repetitive ASCII. Edits are localized within the first
1 KiB of each warm view; this is not a representative repository or a test of
arbitrary edit locations. No cross-platform or cross-engine speed ranking is made.

## Descriptive results

Percentiles below use the **nearest-rank** rule: sort `n` values and select
1-based rank `ceil(p*n/100)`. For `n=64`, p50/p95/p99 use ranks 32/61/64;
p99 is therefore the observed maximum, not a reliable production-tail estimate.
Sequential samples share one process/document and are not independent runs.
No confidence interval, hypothesis test or regression claim is inferred.

### W01: uninstrumented native transaction/commit wall time

All values are nanoseconds:

| Baseline | n | Minimum | p50 | p95 | p99 / maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| 10 KiB | 64 | 23,304 | 34,584 | 56,696 | 143,829 |
| 100 KiB | 64 | 23,534 | 33,322 | 53,530 | 106,399 |

The 100 KiB fixture's observed p95 is 0.053530 ms, below the proposed 5-ms
native-compute target in this run. That does not establish an API or controlled
reference-machine p95: timing excludes `edit_at`/`inspect`, context construction,
seeding, allocation instrumentation, storage and durable IO. No 50-ms sleep
is inserted; each native transaction models one nominal batch.

### W01: separately instrumented allocator region

Counts and bytes use the same nearest-rank definition:

| Baseline | Allocations p50 / p95 | Reallocations p50 / p95 | Allocated bytes p50 | Allocated bytes maximum |
| --- | ---: | ---: | ---: | ---: |
| 10 KiB | 88 / 100 | 11 / 16 | 9,768 | 19,250 |
| 100 KiB | 88 / 100 | 11 / 16 | 9,764 | 111,411 |

The 100 KiB sample at index 5 allocates 111,411 bytes. It remains in the data:
small p50/p95 counts do not imply constant-cost or allocation-free updates.
Allocated bytes are region totals, not peak live heap or RSS. Deallocation
counts can refer to allocations before the region. Instrumented timings are
not mixed with the uninstrumented W01 timings.

### W02: retained 1 MiB history

| Metric | WSL2 observation |
| --- | ---: |
| Seed construction | 1,331,148,829 ns |
| 100,000-transaction history construction | 14,412,642,701 ns |
| Retained native changes including seed | 100,001 |
| Uncompressed / compressed native snapshot | 13,653,102 / 291,896 bytes |
| Native decode from already resident bytes | 1,920,602,031 ns |
| Saved-cut reads, cuts 0–4 | 211,112,893 / 221,312,152 / 227,793,622 / 237,014,902 / 27,634,643 ns |
| Whole-process maximum RSS | 438,824,960 bytes |

The saved cuts are after 0, 25,000, 50,000, 75,000 and 100,000 transactions:
four earlier versions and the final state, not five old versions. All matched,
as did the final text digest. History-construction timing includes loop
bookkeeping and capturing cut texts, not just native commits. Snapshot sizes
are encoded buffer sizes, not filesystem disk growth. The process maximum
includes original and reloaded documents, retained cuts and temporary buffers;
it is not an idle or per-document working set. Compression benefits from
repetitive input.

The native Windows/macOS runs provide additional portability observations, not
pooled samples or comparable heap measurements. In particular, Windows's
476,053,504-byte sampled peak working set remains a **lower bound**. It cannot
establish a Windows memory ceiling.

## Consequences for implementation

1. Preserve lazy per-epoch seed initialization and original native history bytes.
   A 1 MiB seed already takes about 1.33 seconds in this fixture: warm-edit
   timings must not be advertised as first-edit latency.
2. Keep large history/old-view work separately budgeted. This long-history job
   does not satisfy or refute an idle 10k-file-space budget; that is a different
   workload. Do not eagerly load one live native document per repository file
   or infer constant memory from a compressed snapshot.
3. The proposed candidate has no observed native-compute feasibility failure
   in W01/W02, but the current fork/clone/scan inspector has not been benchmarked
   as a production admission path. No resource-safety proof or API latency
   target is accepted.
4. No general 8 MiB document-cost claim follows from the 1 MiB W02 input.
   Experimental wire/operation/insertion guards are not frozen production
   file limits or validated upper memory budgets.
5. Durable latency, disk-cold loading and controlled native-Linux budgets remain
   later measurements. The platform scope exception changes none of their semantics.

## Reproduce the summaries

From the repository root, with Python's standard library only:

```sh
python3 - <<'PY'
import csv
from pathlib import Path

def percentile(values, p):
    return sorted(values)[(p * len(values) + 99) // 100 - 1]

for name, metric in [
    ("platform-wsl2-w01.csv", "native_commit_ns"),
    ("platform-wsl2-allocations.csv", "bytes_allocated"),
]:
    rows = list(csv.DictReader((Path("docs/validation") / name).open()))
    assert len(rows) == 128
    for size in (10240, 102400):
        group = [r for r in rows if int(r["baseline_bytes"]) == size]
        assert len(group) == 64
        assert {int(r["sample"]) for r in group} == set(range(64))
        values = [int(r[metric]) for r in group]
        print(name, size, min(values),
              *(percentile(values, p) for p in (50, 95, 99)), max(values))
PY
```

These describe the retained run; they do not rerun a benchmark or validate
a different compiler/lockfile. The source and archive fingerprints remain those
recorded in the platform report, not newly edited documentation.

## Remaining owner decision

The WSL2 platform substitution is approved; the numerical assessment above
is an engineering recommendation. The [native evidence ADR](../adr/01-native-text-evidence.md)
still needs explicit acceptance or supersession before I02 closes and I03 starts,
especially native change sequences versus base-wide Lamport operation counters.
Neither raw `commands_passed` nor the instruction to use WSL2 accepts that
separate semantic decision.
