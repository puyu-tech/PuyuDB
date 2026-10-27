# ADR-01 proposal: long-term native text binding

**Status: Accepted 2026-10-26 — DT selected with the small maintained patch
set; I02 closed. Amended 2026-10-27 by
[amendment 1](#amendment-1-2026-10-27-engine-bound-canonical-profile-for-diamond-types)
for I03 T1: the engine-bound profile is frozen as a specification in
[docs/spec/canonical-profile-v1.md](../spec/canonical-profile-v1.md). I03 is open
and only its T1 slice is authorized; nothing is implemented, no binding is
promoted, and no native/PuyuDB format is implemented or released.**
The maintainer prioritizes one long-term engine choice before format freeze,
not the easiest initial implementation followed by a planned engine migration.
The earlier initial-AM recommendation remains on hold. A separate reviewed
native-interface spike addresses the two DT adapter gaps without a replacement
merger or decoder; the main workspace still uses unmodified DT.

## Context

PuyuDB needs reusable text convergence and retained native history inside its own
batch, identity, metadata and durability contracts. I02 compares native candidates
before I03 freezes public identities or a native profile.

The [I02 report](../validation/I02-native.md) and
[source/provenance record](../validation/I02-sources.md) are the evidence ledger.
No production engine is selected solely because a benchmark or smoke trace passes.
The maintainer has separately [accepted provisional WSL2 evidence for I02](01-wsl2-validation-scope.md).
That platform-only amendment does not accept this native-semantic proposal.
The subsequent [common four-candidate comparison](../validation/I02-candidate-comparison.md)
and [Loro-fit report](../validation/I02-loro-fit.md) are now available.
The [diamond-types assessment](../validation/I02-diamond-types-assessment.md)
records the executed fit outcomes and unresolved native seams. More existing tests for one candidate
are evidence, not a reason to select it before investigating a credible rival.
The [final bounded DT–Loro comparison](../validation/I02-dt-loro-final.md) then
measured the complete validation-path flow for both engines on macOS, native
Windows and WSL2 under one frozen source fingerprint; those results are the
deciding measurements for this proposal.

## Recommendation and alternatives

**DT is the selected engine (2026-10-26) with a small maintained native patch
set.** The [bounded spike](../validation/I02-dt-interface-spike.md) is not a
permanent fork; its reviewed patches are the reference for I03 promotion.
Keep AM as the conformance reference, not the assumed production winner.

The earlier AM recommendation favored the smaller demonstrated binding design.
AM still has the deepest executed seed, original-byte, operation-scope,
historical-cut and anchor evidence. That is not proof it is the best long-term
choice: diamond-types has favorable native edit/history/historical-read/memory
observations. Its follow-up now demonstrates useful identity/history projection
but also concrete stable-anchor and representation-independent inspection gaps.

Correctness, retained history, offline identity and anchors are hard requirements.
Among candidates that satisfy them, evaluate complete native/binding workloads,
format/decoder compatibility and an explicit maintenance obligation. Neither the
fastest microbenchmark nor the smallest amount of already-written adapter code
alone decides. No universal “best forever” guarantee follows; the goal is to
avoid a known-compromise choice justified by an assumed later migration.

| Candidate | Evidence and limitation | Proposed disposition |
| --- | --- | --- |
| Automerge 0.12.0 | Broadest binding-specific corpus; deterministic empty/Unicode seeds, original changes and exact-cut negatives; expensive seed/load/history work; inspector not hardened | Conformance reference; prior initial-binding recommendation remains on hold |
| Loro 1.17.1 | Favorable native seed/history/load costs and markedly faster native cursor creation; complete tested binding flow slower in every paired cell on all three measured platforms; narrow inspection and new same-cut/cursor controls pass; full-writer-to-peer allocation and empty-seed semantics still need a feasible amended contract | Documented alternative; not selected 2026-10-26; not declared rejected |
| Yrs 0.28.0 | Common trace passes with retained history; default-GC and byte-anchor profiles fail named fixtures; explicit UTF-16 profile passes; editor interoperability is not a current deciding requirement | Retain evidence; do not adopt for the initial binding |
| diamond-types Git `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922` | Favorable native costs; original adapters have two counterexamples, now addressed by a separate small native-interface prototype; faster complete tested binding flow in every paired cell on all three measured platforms; unpublished/format/maintenance obligations remain | **Selected 2026-10-26** with explicit patch/policy ownership; production binding promotion is I03 work |

These dispositions are not universal algorithm rankings. Missing equivalent
tests are not observed failures. No candidate supplies PuyuDB authentication,
immutable outer batches or durable publication by itself. A new specialized CRDT,
whole-repository Automerge model or generic multi-engine production framework
is not justified by these findings. Native convergence is reused only for text;
core continues to own domain/history rules.

## Compared costs and the proposed cost acceptance

Decision inputs are the retained 2026-10-25 common native runs and 2026-10-26
fit evidence, not production telemetry or a new measurement. The complete
four-engine/macOS data remain in the [common report](../validation/I02-candidate-comparison.md).
No samples are removed or pooled here. The earlier
[AM-specific cost assessment](../validation/I02-cost-assessment.md) uses a
different metadata/profile setup and is not substituted into this comparison.
The [final bounded comparison](../validation/I02-dt-loro-final.md) adds the
complete validation-path flow (inspection, publication, anchors, historical
read, save/load) for DT and Loro on three platforms; its totals include
verification work and are not bare native timings.

Selected WSL2 observations, **nanoseconds unless the row says bytes**:

| Metric | Automerge | Loro | diamond-types |
| --- | ---: | ---: | ---: |
| W01, 100 KiB native API p95, median of three process p95s | 101,059 | 3,557 | 561 |
| W01, same aggregation, call + full UTF-8 text p95 | 2,975,338 | 12,544 | 7,504 |
| W02, 1 MiB seed/writer setup | 1,230,979,333 | 2,846,207 | 1,967,031 |
| W02, 100,000-batch history construction | 13,954,208,103 | 398,742,318 | 107,179,655 |
| W02, native save | 7,063,721 | 50,090,762 | 17,063,630 |
| W02, native load from resident bytes | 1,856,456,688 | 4,737,105 | 26,405,514 |
| W02, maximum saved-cut read in that run | 232,522,826 | 997,123,620 | 10,143,617 |
| W02, whole-process maximum RSS, bytes | 395,202,560 | 192,315,392 | 64,704,512 |

Sources: [W01 per-process summaries](../validation/candidates/w01-summary.csv),
[W02 summaries](../validation/candidates/w02-summary.csv) and the common report's
saved-cut observations. W01 uses nearest-rank p95 within each 64-sample process;
the table reports their median, not a pooled p95. W02 has one process per engine,
not a latency distribution. Its history construction includes oracle/coordinate
work and cut checks. Saved-cut maxima cover heterogeneous cuts, not a p95.

DT has the lowest observations in several displayed paths, but does not dominate:
Loro loads faster in this run, and AM's native save is faster. DT can defer rope
work to later reads; the complete-text W01 row matters, not just its tiny native
call. Whole-process RSS includes writer/reloaded documents, expected text and
temporaries; it is not per-document heap or an idle-space budget. macOS results
remain separate. The baseline is repetitive ASCII with localized edits, not a
representative repository or a full multi-writer PuyuDB cost model.
There is no equivalent common Windows performance run, and its earlier sampled
working set is only a lower bound. WSL2's approved I02 substitution remains a VM
observation, not independent native Linux or production support.

Any future AM acceptance must explicitly address O(n) initialization and
potentially seconds-scale seed/load work in these retained fixtures, not promise
that a later engine switch or cache will remove it. Initialize edited epochs
lazily, not one live CRDT per repository
file, and keep historical reads separately budgeted. This is not acceptance of
an end-to-end latency SLO, an 8 MiB-file upper resource bound, storage growth,
idle memory, durable receipts or disk-cold performance. The fork/scan inspectors
are not included in common native W01/W02 timings.

The common profiles omit per-edit binding messages and external original-update
retention; they cannot establish total cost of either production binding.
Loro's separate 10 KiB/1,000-update fit probe retains 375,708 payload bytes plus
32,000 digest-value bytes despite a 7,382-byte native snapshot. These are byte
components, not total storage, and must not be extrapolated to the different
1 MiB/100,000-batch common fixture.

## AM-specific semantics if chosen

1. **Epoch-owned seed, not first-arrival ownership.** Retain the deterministic
   native seed bytes and identity once per content epoch, including empty text.
   Seed and writer namespaces remain separate. Never regenerate an existing
   lineage with a newer library or add a hidden eager-seeding fallback.
2. **Full writer context, distinct identities.** Derive the ordinary native actor
   from the full writer key, branch, writer epoch and content epoch. A native
   actor/hash is neither a PuyuDB batch/artifact ID nor authentication.
   Authentication and admission policy remain separate core obligations.
3. **Exact intent cut.** Interpret UTF-8 byte ranges at scalar boundaries in the
   captured base, using an explicit UTF-16 conversion only when requested.
   Multiple edits to an epoch use its staged view in order. A historical fork
   cannot reuse an actor's newer counter state or acquire unrelated current heads.
   If the previous outer-stream batch is absent from the cut, use a fresh writer
   epoch instead of silently widening that cut.
4. **Native counter clarification.** Native change sequences are gapless per
   actor. Operation counters are allocated after the exact base cut's maximum,
   so observing another actor can cause a jump; they are not gapless per actor.
   Outer batch sequence and the outer DAG Lamport clock remain separate from
   both native counters.
   The existing executable counter-jump fixture supports this interpretation.
5. **One native change per edited epoch per outer batch.** Inspect the expected
   root text object, insert/delete forms, targets, dependencies, actor and metadata
   against that cut. Bind writer context, outer sequence, epoch and ordinal;
   native parse/apply success or visible text equality alone is insufficient.
6. **Retained original artifacts and outer publication.** Preserve original
   native changes and seed bytes, dependency/producer associations and the outer
   immutable batch. A native snapshot is a rebuild accelerator, not permission
   to drop original authoritative artifacts. A change shared as a dependency is
   not newly authored again; seed availability does not make an editing batch
   its producer. Core, not a native commit, will enforce cross-file atomicity.

These are semantic constraints for the next design stage, not public types or
an implementation of storage/authentication. Exact actor/metadata encodings,
tags, widths, profile versions, limits, canonical records and signed compatibility
vectors remain I03 deliverables. Current experimental `i02` strings and numeric
limits are not frozen by acceptance of this direction.

## What choosing Loro instead would require

Loro remains a credible alternative, not a failed algorithm. The fit experiment
does not yet establish the following replacement semantics:

- **Offline native-peer allocation and collisions.** Define a full
  writer/branch/writer-epoch/content-epoch association to native `u64` peers.
  The caller-supplied fixture registry is not that allocator. Hash truncation,
  first-arrival ownership or a central allocator cannot be introduced silently:
  preserve full authentication context and explicitly address independent offline
  histories that assign the same peer to different writers. Never rewrite
  retained native IDs to repair a collision or claim it cannot happen.
- **Empty-seed amendment.** The tested empty root needs no native change and has
  identical native snapshots across epochs. Adopt an explicit epoch-owned outer
  descriptor/seed-binding contract if that is intended, rather than pretending
  it satisfies AM's native seed-change rule or adding a hidden dummy operation.
- **Native spans versus immutable publications.** Retain exact original updates
  and outer producer/batch associations independently. Export/import can merge
  native changes despite local no-merge configuration. Fully known conflicting
  native IDs can be import no-ops; that cannot establish byte equality or authorize
  a new producer. Unique messages in a fixture are not a durable batch ledger.
- **Binding validation and native bounds.** Specify ownership, exact-cut span
  and operation rules, native counter exhaustion and anchor behavior, with
  corresponding negatives/compatibility vectors. Existing narrow reconstruction
  tests are not equivalent to AM's broader corpus or a proof over every encoding.

The identity/collision and seed-contract choices are I02 feasibility decisions,
not merely field names to leave for I03. Accepting a different engine requires
an explicit amendment that resolves them; AM's counter rule must not be imposed
on Loro. Authentication, original-artifact retention and hardened decoding remain
obligations for either choice, not requirements invented only for the challenger.

## Remaining risks and stop/go

**DECIDED (2026-10-26):** DT selected with the small maintained patch set;
I02 closed.
**NO-GO now:** start I03 without an explicit maintainer go-ahead, or freeze a
native/PayuDB format. Promoting the reviewed patch set into the main dependency
graph is I03 work, not part of this decision.

The AM inspector rejects compressed input before upstream inflation; the Loro
fixture's metadata decoder already allocates/decodes before reporting counts.
DT's strict fixture guard catches ignored input but is not representation-independent:
valid same-cut updates can fail it after opposite import orders. Its numeric
cursor projection likewise fails import-order stability after deletion.
No experimental inspector establishes bounded hostile-input work or general
canonical native encoding. Decoder hardening/fuzzing and full-system performance
still need their implementing gates; their absence is not concealed by green
native tests.

The backward-insertion diagnostic confirms a native interleaving limitation:
convergence is not semantic code correctness or universal non-interleaving.
No hand-written merger or actor sort is proposed to override native integration.
No release, production support, storage/replication implementation, legal clearance
or independent proof of an upstream algorithm follows from I02 acceptance.

## Maintainer decision (2026-10-26)

The previous request to accept initial AM binding/counters/costs remains **on hold**.
The clarified goal was one evidence-backed long-term choice before I03, without
planning an engine migration as the answer to a known performance compromise.

**Decision: DT is selected** with the small maintained native patch set, on the
completed [DT-fit experiments](../validation/I02-diamond-types-assessment.md#decisive-fit-gates)
and [final bounded DT–Loro comparison](../validation/I02-dt-loro-final.md)
(macOS, native Windows and WSL2, audited evidence chain). I02 is closed.

Accepted with the selection, per the [small native-interface prototype](../validation/I02-dt-interface-spike.md):
the native identity-boundary cursor policy including post-deletion replacement
behavior, the epoch-owned external empty-seed descriptor, the original
operation-span/outer-publication association, and patch/dependency/notice
ownership. The old numeric cursor and byte-reconstruction adapters are not the
permanent implementation; the spike patches are the reviewed reference for I03
promotion. Loro remains a documented alternative; its full offline
writer-to-peer allocation/collision policy and empty-seed amendment remain
unimplemented and were not required for this decision. AM remains the
conformance reference.

Neither path has production authentication, outer publication,
resource-hardened decoding or a full-system cost assessment; choosing DT does
not claim them. The current experimental bytes remain unfrozen; no transparent
future engine migration is promised.

The [native-interface spike](../validation/I02-dt-interface-spike.md)
supplies a concrete small implementation of both DT seams: native insertion/
tombstone ordering and strict all-new-operation decoding with original metadata.
It does not change merge rules or the native format, and it fixes the tested
counterexamples in a separate harness. The old unmodified-pin characterization
results remain valid.

**I03 has not started.** The maintainer explicitly deferred it; starting I03
requires an explicit go-ahead. Exact syntax/widths/versions and promotion of
the patch set into the production binding remain I03 work. No end-to-end
performance, hostile-input security, upstream acceptance or distribution
guarantee follows from the prototype or this decision.

## Amendment 1 (2026-10-27): engine-bound canonical profile for diamond-types

**Scope: I03 T1 only.** The maintainer authorized starting I03 with a restricted
scope — this amendment plus the frozen specification — and confirmed the
promotion route for later T3: a pinned fork revision with recorded provenance
(the plan's default), not a pending upstream PR. T2–T6 and the audit phase remain
unauthorized. This amendment changes no Rust source, dependency, lockfile or
workflow.

The blueprint still names Automerge in the profile vocabulary (`am-text-v1`,
`puyudb/am-seed-actor/v1`, `puyudb/am-actor/v1`, §5.1/§5.4/§22.1/§22.2). DT is
selected, so four engine-bound choices are decided here and frozen in the
[canonical profile specification](../spec/canonical-profile-v1.md):

1. **Frozen kind and profile tag.** The v1 native text kind/profile is
   `dt-text-v1`, bound to DT revision
   `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922` (dialect `DMNDTYPS` protocol
   version 0, no LZ4) plus the reviewed patch set as the production binding.
   Any other revision, dialect or feature set is a different profile string and
   needs its own fixtures.
2. **Engine-bound seed and writer namespaces.** `puyudb/dt-seed-actor/v1` and
   `puyudb/dt-actor/v1` replace the `am-*` drafts, and the §22.2 ownership rules
   are unchanged: a seed is an immutable epoch-owned baseline dependency, never
   owned by the first editing batch, and non-seed native changes must use the
   enclosing writer's derived actor. The mechanism is engine-bound because DT
   interns actors as **names**, not byte actors: DT agent names are strings under
   50 UTF-8 bytes, its numeric `AgentId` is replica-local interning, and a full
   256-bit binding therefore has to fit a textual form. The blueprint's leading
   `0x00`/`0x01` namespace tag is realized as the leading `s:`/`w:` name tag over
   the same domain-separated digests, so seed and writer namespaces still cannot
   overlap.
3. **Blueprint references superseded for v1.** Blueprint `am-*` names describe
   the conformance reference only. The specification is normative for v1 and says
   so; a blueprint rev 0.3 note recording this is applied by the maintainer,
   because the blueprint lives outside this repository.
4. **Re-frozen lazy-seed profile.** The §5.4 lazy-seed stop/go gate was written
   for AM. For DT it is re-frozen on the accepted empty-seed contract: the seed
   is an epoch-owned native artifact whose payload binds `epoch32`; an empty
   baseline yields a **zero-length** native payload with no sentinel insert and
   no hidden eager seeding, and the artifact stays epoch-specific because its
   payload binds the epoch rather than only the bytes. Golden cross-process seed
   fixtures and an independent validator are owed by T2, not asserted here.
   I02's cross-process DT seed repetition is the starting evidence, not a frozen
   profile.

Also decided with this amendment, without changing an accepted direction:

- Native artifact identity is over the exact bytes a producer exported. DT's
  native encoder leaks local import order into agent-name serialization, so a
  replica must never re-encode a history and claim the producer's `ArtifactId`,
  and admission inspects logical content against the declared cut instead of
  reconstructing bytes across replicas.
- The accepted identity-bound cursor policy stands, including its tested
  post-deletion gap limitation. No replacement merger, actor-order override or
  tombstone workaround is authorized.

Still not claimed: a canonical native encoding, a hostile-input-hardened decoder,
upstream acceptance of the patch set, production authentication, storage,
replication, or any end-to-end performance guarantee. AM remains the conformance
reference; Loro remains a documented alternative with unimplemented offline
peer-allocation and empty-seed amendments. Promoting the patch set into the
production dependency graph is T3 work and is not part of this amendment.
