# Implementation status

**Completed issue IDs: I01. Current issue: I02, common four-candidate baseline
implemented; engine fit/selection and native-semantic stop-go remain open.**
Work is maintained on `i01-repository-foundations`. Local checkpoint commits
record reviewed progress; they do not close I02, accept a native format, or
constitute a release. No Git merge, push, publication, deployment, or GitHub
issue creation was performed. IDs below are implementation-plan IDs.

## I01 closure

GitHub's private-vulnerability-reporting API returned **`enabled: true`** after
the maintainer enabled it. The implementation agent only verified that setting.
The security policy now names the private channel without promising staff or SLA.

I01's four-package workspace, Apache-2.0/shared metadata, locked offline build,
help/version-only CLI, contributor policies, dependency tooling, and CI foundation
are complete. The original build/tests passed on Rust 1.85.0 and 1.99.0 on macOS
ARM64. By explicit subsequent direction, the current compiler **and minimum are
now 1.99.1**, the verified latest stable release; older compiler support is dropped.

## What actually exists

| Package | Current behavior |
| --- | --- |
| `puyudb-core` | Documented domain/publication boundary only; no database |
| `puyudb-crdt` | Experimental AM inspection seam, shared candidate workload adapters and research tests; no accepted format or stable SDK |
| `puyudb-local` | Documented local boundary only; no Git/filesystem operations |
| `puyu-cli` | `puyu` help/version and `puyu db` help only |

No storage, authoritative history publication, replication, server, or SDK exists.
The CLI does not link the research engines. Original code/docs remain Apache-2.0.

## I02 evidence and stop/go

See [the cited validation report](validation/I02-native.md) for exact scope,
raw measurements, independent-review findings and remaining acceptance gates.

- Pinned Automerge 0.12.0; deterministic retained seed byte/hash fixtures match
  across independent processes.
- Native convergence/history/Unicode/operation-scope tests, including every
  delivery order of small fixtures and 5,760 generated small-history schedules.
  The larger four-writer/48-change corpus also passed 2,048 explicitly executed
  optimized delivery schedules with partitions, duplicate/lost first sends,
  dependency buffering and native snapshot reload.
- Executed shared offline/duplicate/reload/history traces on Automerge 0.12.0,
  Yrs 0.28.0 and Loro 1.17.1. Yrs's byte-anchor profile fails the astral fixture;
  an explicit UTF-16 profile with checked UTF-8 conversion passes separately.
- Native-only W01 measurements and complete 1 MiB/100,000-transaction W02 run,
  including four earlier-version checks plus the final cut and raw CSV artifacts.
- A reviewed full-revision DT upstream pin (`89ae3a0...`, manifest 2.0.0,
  unpublished) now builds without LZ4 and passes ASCII/Unicode text, duplicate,
  reload and portable historical-frontier traces. The broken published 1.0.0
  remains excluded; no known vulnerability is suppressed.
- Every-small-frontier cursor/affinity checks, valid-checksum unknown-column and
  author/counter/resource negatives, source-informed adversarial cases, and
  calibrated W01 allocation samples are added. The backward insertion diagnostic
  converges with interleaving; no stronger ordering guarantee is fabricated.
- Optional source-transfer and baseline collectors are prepared for the
  maintainer's native Windows x86-64 and Debian WSL2 environments. They transfer
  current uncommitted inputs, retain raw outputs and source fingerprints, and
  never approve the gate. See [the platform run guide](validation/I02-platform-runs.md).
- Actual native Windows, Debian WSL2 and fresh macOS collections all passed
  against one source fingerprint. The first Windows run exposed executable-name
  drift in help output; setting the canonical CLI binary name fixed the four
  failing snapshots without changing them. See [the results](validation/I02-platform-results.md).
- The maintainer has [approved WSL2 instead of an unavailable Linux PC for I02](adr/01-wsl2-validation-scope.md).
  The [cost assessment](validation/I02-cost-assessment.md) recommends continued
  native-candidate work with explicit first-edit/history/resource limitations.
  WSL2 W01/allocation raw samples are retained alongside the W02 CSV.
- The [revision-2 common comparison](validation/I02-candidate-comparison.md)
  executed W01/W02 for all four candidates on WSL2 and macOS, with 11 cuts
  bracketing deleted intervals, separate native/text/coordinate stages and
  three W01 processes per engine. All cases passed. Native full seed artifacts
  repeat across three fresh processes on both platforms; raw CSVs, sanitized
  manifests, summaries and lossless seed bytes are retained in the repository.

**GO:** continue I02 candidate-fit validation. **NO-GO:** engine selection,
I03/native-format freeze. Common cost observations do not supply equivalent
operation-scope, ownership, original per-update byte or transaction-boundary
validation. Prioritize a bounded Loro-fit experiment while retaining AM as the
deepest fidelity/reference path; no alternative is declared rejected.
The AM counter clarification is conditional on selecting its binding, not
a rule imposed on other engines. WSL2 is approved; independent native Linux
is deferred, not a current hardware-procurement blocker.
See [the proposed addendum](adr/01-native-text-evidence.md) and
[fixture provenance](validation/I02-sources.md). Production admission hardening,
storage, public anchor statuses and protocol simulation are not implemented here.

## Current verification

Local environment: macOS ARM64, Rust/Cargo 1.99.1. The exact required commands pass:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

**51 default integration tests pass:** 45 native research tests and 6 CLI process
tests. Two long fixtures are intentionally ignored by the default runner and
have passed explicitly in release mode: the mixed-history delivery gate and
the all-candidate paste/delete/restore workload fixture. Allocator calibration
also passes as a separate example test.
Some passing tests deliberately preserve a failed candidate profile; they do not
mean that profile satisfies the product contract.

Cargo-deny 0.20.2 passes with the explicitly documented dev-only unmaintained and
license exceptions in [the ledger](DEPENDENCIES.md); no vulnerability is ignored.
Additional passing checks:

```sh
cargo fetch --locked
cargo build --workspace --all-targets --frozen
cargo test --workspace --no-default-features --frozen
cargo test --workspace --all-features --frozen
cargo test --release -p puyudb-crdt --test mixed_histories thousands_of_seeded_partition_duplicate_restart_schedules --frozen -- --ignored
cargo test --release -p puyudb-crdt --example candidate_workloads --frozen -- --ignored
cargo test -p puyudb-crdt --example native_allocations --frozen
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --frozen
target/tools/bin/cargo-deny --locked check
```

Actionlint 1.7.12, YAML/front matter, local links, whitespace and frozen feature
checks pass for all three workflows/current sources. The explicit long release
gate was rerun after the distinct-order assertion was added and passes. The
two original CSVs retain their original fingerprint; the new allocator CSV
matches its recorded current-lockfile run. A compatible-update dry run reports
no lockfile changes.

Independent engineering review identified two test gaps: non-specific size-error
assertions and missing positive new-anchor checks. Both were corrected and the
updated tests pass. Spec review confirms local feasibility is not unconditional
I02 acceptance. Neither review is an independent proof of the native algorithm.

The portable collector's **13 Python contract tests pass on all three platforms**.
Complete Windows, WSL2 and macOS collections passed all 10 steps, including
the long native gate and validated W01/W02/allocation output. The common measured
source fingerprint is
`dc9cc73733fed3e5ea9a6da3b18606dc5ead01fbf4fdaf1ce3191d123787c289`.
The results document retains exact run IDs, raw hashes, W02 CSV copies and
resource-method limits. Later guide/status edits do not change measured code.
Independent review found and prompted fixes for omitted legacy Cargo configuration
and unchecked compiler overrides; regression tests cover both.
Follow-up read-only review independently verified the platform report inventories,
all 12 raw-artifact hashes, the three W02 CSV copies and the CLI red/green evidence;
no actionable finding remained. It did not rerun platform commands or independently
verify the separate installation/hardware probes.

The WSL2 cost assessment's retained raw CSVs match their original stdout bytes
and hashes. Nearest-rank percentiles were recomputed and cross-checked against
empirical-CDF selection; no samples or allocation outliers were removed.
Independent numerical review confirmed the reported values and identified one
wording correction: W02 checks four earlier cuts plus the final state, not five
old versions. Reports now state that distinction. No runtime code changed in
the platform-amendment/cost-assessment follow-up; no new benchmark run is claimed.

The later common-comparison slice adds real dev-only native workload code.
Both revision-2 reports completed all 40 steps, with source fingerprint
`74ed9166b8698401474f4f4773d851f4cfc2f61a138a7746b70bc4073bdf6f4a`.
Python contract tests: 21 pass on macOS/WSL2; native Windows passes 20 and skips
the Unix-only process-group test. Native Windows also passes all 51 integration
tests; its new common performance workload is not run.
Independent code reviews found and prompted fixes for DT materialization scope,
Unicode starting offsets, candidate-error continuation, Unix wrapper cleanup
and ambiguous CSV headers. Regressions and fresh v2 runs pass.
Independent numerical review verified all 64 retained raw CSVs, 96 W01 summary
values, 64 W02 summary values, 88 saved-cut hashes and 12 seed payloads against
72 original artifacts. No discrepancy or blocking unsupported claim remained.

## Limits and next action

CI is configured for Linux x86-64/ARM64, macOS x86-64/ARM64 and Windows x86-64
with Rust 1.99.1. Native Windows x86-64, Debian WSL2 and macOS ARM64 checks have
run locally/remotely; GitHub Actions, independent native Linux and macOS Intel
checks have **not run**. No supported release, staffed security process,
distribution/legal audit, or production durability/recovery/performance
guarantee exists.

Continue **I02**, not I03: evaluate the strongest challenger's PuyuDB-fit gates,
then present an evidence-backed engine/binding and cost decision. The previous
AM-only cost proposal is not selection against the other libraries.
Exact canonical syntax, widths, profile versions and fixed limits remain I03
deliverables after that gate. Independent native Linux must be revisited when
available and before Linux production support/performance claims/I36.
Remote tests used the maintainer's authorized existing Windows/WSL machine;
no remote infrastructure was provisioned. Scheduled CI is configuration, not
executed platform evidence.
