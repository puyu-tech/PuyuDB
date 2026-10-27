# Implementation status

**Completed issue IDs: I01, I02. Current issue: I03, T1/T2/T4 complete and T3
half done (crypto dependency only; the DT production pin is open). I02 closed on
2026-10-26: the maintainer definitively selected diamond-types (DT) with the
small reviewed native patch set, on the completed common baseline, Loro/DT-fit
experiments, native-interface spike and three-platform final comparison. On
2026-10-27 the maintainer authorized I03 from T1 onward through the codec and
confirmed the later promotion route as a pinned fork revision with recorded
provenance. The profile is frozen as a specification plus fixtures, and
`puyudb-core` now implements the codec that reproduces them byte-for-byte.
There is still no storage, durable publication, protocol, daemon or CLI surface,
no native encoding is canonicalized, and the DT patch set is still dev-only
because a pinned fork needs a fork revision to pin.**
Work is maintained on `i03-identity-batch-profile`. Local checkpoint commits
record reviewed progress; they do not accept a native format or constitute a
release. No Git merge, push, publication, deployment, or GitHub
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
| `puyudb-core` | Frozen identity/profile codec: typed IDs, canonical CBOR subset, records and digests, strict validation, path rules, DT actor/seed derivations, Ed25519 preimages, writer-stream checks; no database, storage or publication |
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
- The [bounded Loro-fit experiment](validation/I02-loro-fit.md) adds 18 passing
  tests for exact-cut plain-text inspection, hidden operations, full fixture
  identity/epoch checks, replay and original-byte/boundary limitations. All pass
  on macOS, native Windows and WSL2. A separate 1,000-update byte probe retains
  375,708 original payload bytes plus 32,000 digest-value bytes; its 7,382-byte
  native snapshot is not total storage or an original-artifact archive.
  Empty seeds and peer width need an explicit binding amendment. No production
  registry, authenticated admission, bounded decoder or durable batch is delivered.
- The [executed DT-fit follow-up](validation/I02-diamond-types-assessment.md)
  adds 41 default tests, including three new Loro controls. Full-digest identity,
  exact-cut authoring, original-target projection and external byte retention
  have bounded positive evidence. Two tested adapter assumptions fail: deleted
  anchors can resolve as 3 versus 1 by import order, and equivalent native
  updates can differ in agent-name serialization order. Green characterization
  tests preserve these failures; DT native text still converges.
- The [final bounded DT–Loro comparison](validation/I02-dt-loro-final.md)
  executed the complete validation-path flow (author, strict inspection,
  publish, anchors, historical read, save/load) for both engines across the
  seven primary cells plus one supplemental cell, three processes each, on
  macOS, native Windows and WSL2 under one frozen source fingerprint. DT was
  faster in every paired completed cell on all three platforms (4.02×–29.72×);
  Loro remains markedly faster at native cursor creation and retains the
  smaller native snapshot. The original 1 MiB combined-replacement cell is
  kept as a Loro fixture-profile failure. 1,984 malformed-input screening
  trials per platform found no panic, crash, timeout or control violation,
  and an independent audit verified all sealed evidence hashes, re-derived
  summaries and cross-platform consistency.

**DECIDED (2026-10-26):** the maintainer selected DT with the small maintained
patch set; I02 is closed on the completed [DT/Loro fit evidence](validation/I02-diamond-types-assessment.md#decisive-fit-gates)
and [final bounded comparison](validation/I02-dt-loro-final.md). The accepted
[engine decision](adr/01-native-text-evidence.md) records the cursor policy,
empty-seed contract, span/publication association and patch ownership.
**NO-GO:** starting I03 without an explicit maintainer go-ahead, or freezing a
native format. The 2026-10-27 restricted go-ahead ([I03 T1–T4](#i03-t1t4-frozen-identity-and-batch-profile-2026-10-27))
supersedes this for T1/T2 only. The patch set stays experimental until I03 T3
promotes it. Common cost observations do not supply equivalent
operation-scope, ownership, original per-update byte or transaction-boundary
validation. The Loro-fit follow-up narrows some gaps but does not eliminate them.
Retain AM as the deepest fidelity/reference path; no alternative is declared rejected.
The AM counter clarification is conditional on selecting its binding, not
a rule imposed on other engines. WSL2 is approved; independent native Linux
is deferred, not a current hardware-procurement blocker.
See [the proposed addendum](adr/01-native-text-evidence.md) and
[fixture provenance](validation/I02-sources.md). Production admission hardening,
storage, public anchor statuses and protocol simulation are not implemented here.

## I03 T1–T4: frozen identity and batch profile (2026-10-27)

Authorized scope was T1 through T4 of
[the I03 plan](plans/I03-identity-and-batch-profile.md); T3 is only half done and
T5/T6 are open. T1/T2 changed documentation and Python fixtures only.

- [ADR-01 amendment 1](adr/01-native-text-evidence.md#amendment-1-2026-10-27-engine-bound-canonical-profile-for-diamond-types)
  re-specifies the four engine-bound semantics: `dt-text-v1` bound to DT
  `89ae3a0…` with the reviewed patch set, `puyudb/dt-seed-actor/v1` and
  `puyudb/dt-actor/v1` replacing the `am-*` drafts with the `s:`/`w:` textual
  namespace tags (DT interns actor **names** under 50 bytes, not byte actors),
  the blueprint `am-*` references superseded for v1, and the DT lazy-seed profile
  re-frozen on the accepted zero-length empty-seed contract.
- [Canonical profile v1](spec/canonical-profile-v1.md) freezes record layouts,
  domain separators, typed-ID widths and presentation, tag registries, paths,
  writer-stream/lamport/parent rules, frontier normalization, `causal_fork`,
  chunk/artifact framing, declared limits, and the DT actor/seed derivations.
  It also records what a replica may not do: no re-encoding claimed as a
  producer's artifact, no last-arrival equivocation winner, no persisted
  numeric DT `AgentId`.
- T2 adds `scripts/i03_canonical_vectors.py` (hand-encoded layouts, strict
  record validator, RFC 8032 reference signer) and
  `scripts/test_i03_canonical_vectors.py`, plus committed fixtures in
  [tests/fixtures/canonical-v1](../tests/fixtures/canonical-v1/README.md):
  **20 positive vectors** covering every frozen layout with expected digests and
  two deterministic signing vectors, **50 negative records** carrying their
  required stable codes, and **17 path cases**. The Python suite (23 tests) also
  self-checks the Ed25519 implementation against RFC 8032 §7.1 TEST 1/2, so the
  signature vectors are anchored to something external.

- T3a wires only the dependencies the codec needs: `ed25519-dalek 3.0.0` (exact
  pin, default features off, `verify_strict`), `sha2`/`thiserror` as direct core
  dependencies, and `serde_json` as a dev-only fixture reader that was already in
  the graph. Its BSD-3-Clause license is an explicit global `deny.toml` allow with
  the notice-reproduction obligation recorded in `THIRD_PARTY_NOTICES.md`; the
  ledger now lists eight affected rows (227 registry versions, no duplicates).
  **The reviewed DT patch set is still not promoted**: the confirmed pinned-fork
  route needs a fork revision to pin, and no dependency moved out of dev-only.
- T4 implements the codec: `canon` (strict CBOR subset with minimal-length and
  no-map/no-tag/no-float rules), `identity` (typed IDs and strict presentation
  parsing), and `profile` (records, domain-separated digests, strict validation
  with the frozen stable codes, path rules, `dt-text-v1` actor and seed
  derivations, Ed25519 preimages with strict verification, writer-stream and
  equivocation checks). The integration test consumes the committed corpus:
  every vector must decode, validate and re-encode to identical bytes; every
  digest is re-derived; every negative case must fail with its declared code; the
  two signature vectors must equal the Python-produced signatures byte-for-byte;
  and the 45-byte actor names must match literals baked in both languages.
- T4 immediately found a real defect in the T2 validator: its base64url helper was
  shifted by one group, so every derived actor name was wrong while all 23 Python
  tests stayed green (it only ever compared its own names). Fixed in `43119c8`;
  no fixture body or digest changed.

Nothing is measured: this is a codec and its conformance corpus, not durability,
storage, replication or performance evidence. The DT re-encoding and deleted-gap
cursor limitations remain accepted, unfixed findings.

Local verification of this slice on macOS ARM64 with Rust 1.99.1: **132 workspace
tests pass** (104 native research, 9 canonical-profile integration, 13 core unit,
6 CLI), 44 Python tests pass, and `cargo fmt --check`, clippy `-D warnings`, the
frozen all-target build, the no-default-features and all-features test runs,
warning-denied rustdoc, `cargo fetch --locked` and `cargo-deny --locked check`
are green. The two long ignored release fixtures were not rerun because no
`puyudb-crdt` source changed; hosted CI was not run.

## Current verification

Local environment: macOS ARM64, Rust/Cargo 1.99.1. The exact required commands pass:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

**132 workspace tests pass:** 104 native research tests, 9 canonical-profile
integration tests, 13 core unit tests and 6 CLI process tests. Two long fixtures are intentionally ignored by the default runner and
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

The subsequent Loro-fit slice passes all 69 integration tests, formatting,
Clippy, frozen feature checks and the byte probe on macOS, native Windows and
WSL2 with source fingerprint
`b73e3d32a40c480e42fc2dfc2ea882a874b1f6eab713e317da8bd68ca7c1d973`.
Python again passes 21 tests on macOS/WSL2 and 20 plus one skip on Windows.
Both long release fixtures, frozen all-target build/docs, allocator calibration,
cargo-deny and actionlint were rerun successfully on macOS only.
Independent static follow-up review confirms the backward-delete, capture
side-effect and epoch-test findings addressed; it did not rerun the commands.
The retained 1,000-update corpus, seed and snapshot match byte-for-byte across
all three platforms. No new common performance measurement is claimed.
Independent evidence review verified the corpus, hashes, source inventories,
saved platform logs and test counts without a discrepancy; its exact verification
limits are recorded in the [fit report](validation/I02-loro-fit.md).

The earlier initial-AM engine/binding proposal changed documentation only. Independent
read-only review found no actionable issue in its rationale or acceptance scope
and verified all 16 cost-table cells against retained evidence, including original
resource logs. Local formatting, Clippy and all 69 default integration tests
were rerun successfully; at that point Rust/Python sources, Cargo configuration
and lockfile still matched the verified fit inventory. That slice claimed no new benchmark or remote run,
and review approval is not maintainer acceptance of the proposal.

The maintainer's subsequent long-term-choice clarification puts that recommendation
on hold. Two read-only DT source investigations cover binding APIs and format/
maintenance obligations. That initial assessment separated source directions from
then-unexecuted fit cases. The proposal also displayed DT's retained costs;
all 24 numeric cells were checked against existing evidence. That source-assessment
slice changed docs only, not Rust/build inputs, and claimed no new experiments.
Independent read-only review found no actionable issue in the revised scope or
source claims; all 24 table cells, three copied DT figures and 54 local links
were verified. It did not run the proposed DT experiments or audit all upstream
decoder safety.

The subsequent executed DT-fit slice passes all 110 default integration tests,
formatting, Clippy, frozen feature/all-target checks, warning-denied rustdoc and
both byte probes on macOS, native Windows and WSL2 with source fingerprint
`bf435fa8b97f1f3a5fd4364593adbe7d954ab4ba6b70f6fb279958f012dd5447`.
All 16 remote/21 macOS recorded steps exit zero; macOS extras include both long
release fixtures, allocator calibration, cargo-deny and actionlint.
Python passes 21 tests on macOS/WSL2; Windows passes 20 with one Unix-only skip.
Three fresh DT seed processes per platform repeat all three cases. All 1,000 DT
original payloads, seed and snapshot match across platforms; fresh Loro control
artifacts match the earlier retained corpus. DT retained payload/digest components
are 370,770 / 32,000 bytes, separate from its 14,389-byte native snapshot.
Independent code review executed and confirmed the anchor and same-cut inspection
counterexamples, including equal native text/IDs or logical history. This is
completed feasibility research, not passing every fit gate or selecting an engine.
Independent evidence review verified both original-byte corpora, source/archive
inventories, raw hashes, seed repetition, recorded commands and platform counts
without a discrepancy. Its confidence applies to retained evidence, not general
decoder safety or production performance.

The [native-interface spike](validation/I02-dt-interface-spike.md) adds 150 native
lines/removes 18 across five files, reusing the existing merger and decoder.
Seven anchor and 16 strict-input tests pass in debug/release on macOS, native
Windows and WSL2, with 15 unchanged native byte observations and four Python
extraction contracts. Actual LZ4 compatibility passes on Unix defaults and on
Windows's LZ4-only text profile; unchanged upstream Unix-only storage fails
native Windows defaults and is not ported. Independent review found and confirmed
the fix for a borrowed-metadata lifetime regression; no remaining blocking native
seam finding was reported. Main Cargo inputs and the original 110-test workspace
remain unchanged; the new tests use a separate generated harness, not a production
fork or a fifth permanent workspace package.

The [final bounded comparison](validation/I02-dt-loro-final.md) ran 48 attempted
measurement processes per platform (45 complete; three retained original Loro
`churn1m` profile rejections exiting 101), plus correctness, debug/release tests,
formatting, Clippy and 1,984 screening trials per platform, all green on macOS
ARM64, native Windows x86-64 and Debian WSL2 with frozen source fingerprint
`635e469d530f331e7fba951108fa0b0e292a0cb664ce642ee2a01b28eb3689f3`. An independent
audit verified every sealed evidence file hash, re-derived each platform summary
from the raw benchmark rows with an exact match, and confirmed identical text
signatures, retained byte components and source fingerprints across platforms.
The main workspace, dependency lock and DT pin remain unchanged; the experiment
used a separate generated harness. The 2026-10-26 I02 closure (DT selected)
changed documentation only; no Rust/build inputs changed after the audited runs.

## Limits and next action

CI is configured for Linux x86-64/ARM64, macOS x86-64/ARM64 and Windows x86-64
with Rust 1.99.1. Native Windows x86-64, Debian WSL2 and macOS ARM64 checks have
run locally/remotely; GitHub Actions, independent native Linux and macOS Intel
checks have **not run**. No supported release, staffed security process,
distribution/legal audit, or production durability/recovery/performance
guarantee exists.

**I02 is closed (2026-10-26):** the maintainer definitively selected DT with the
small maintained patch set. The earlier initial-AM recommendation remains on
hold, not accepted. The [completed maintainability spike](validation/I02-dt-interface-spike.md)
supports DT with a small native patch set rather than a separately developed
CRDT, and the [completed final comparison](validation/I02-dt-loro-final.md) shows DT's
complete tested binding flow faster on all three measured platforms while Loro
keeps markedly faster native cursor creation and a smaller native snapshot.
Accepted with the selection: native identity-boundary cursor semantics,
external epoch-owned empty seeds, original operation-span/outer-batch binding,
and patch/dependency/notice ownership. Loro remains a documented alternative
with unresolved offline peer allocation/collisions; AM remains the conformance
reference, not an automatic fallback choice. An unpublished upstream pin is a
risk to resolve (patch rebase/notice obligations), not a rejected choice. No
transparent engine migration is promised.
**I03 is open with T1, T2 and T4 done and T3 half done.** What remains: promote
the reviewed DT patch set into a pinned production binding with lock/deny/notice
updates (T3), which needs the fork revision the maintainer's route implies; run
the full CI matrix (T5) — the Rust↔golden-vector cross-tests already run inside
the workspace suite, but hosted CI has not executed this slice; closure docs
(T6); and the audit phase. Independent native Linux must be
revisited when available and before Linux production support/performance
claims/I36. Remote tests used the maintainer's authorized existing
Windows/WSL machine; no remote infrastructure was provisioned. Scheduled CI is
configuration, not executed platform evidence.
