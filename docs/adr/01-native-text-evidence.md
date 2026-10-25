# ADR-01 evidence addendum: native text/history feasibility

**Status: Proposed — local validation evidence, not format acceptance.**

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
is now available. The maintainer requested comparison before selection; the
Automerge direction below remains proposed, not a decision that rejects alternatives.

## Findings and proposed direction

Retain **Automerge 0.12.0 as the proposed text candidate**. Local evidence supports
deterministic epoch-owned seeds, original-byte retention, historical cuts,
explicit scalar indexing, and text-only operation inspection. The ordinary writer
and seed namespaces remain separate. No new CRDT, eager seeding fallback, or
native/PuyuDB identity conflation is justified by current findings.

Alternatives remain meaningful but are not drop-in replacements:

- **Yrs 0.28.0:** tested history needs GC disabled. The byte-offset anchor fixture
  fails; an explicit UTF-16 profile with checked conversion passes separately.
- **Loro 1.17.1:** tested full-history snapshots and historical-fork anchors work.
  Peer-ID width, change coalescing and historical anchor fallback need a distinct
  binding design before adopting it.
- **diamond-types pinned upstream:** the safe no-LZ4 Git revision passes the common
  trace and retained remote-frontier checks. It is unpublished, has a legacy list
  codec, prerelease/transitive maintenance risk and a distribution-notice gap.

No cross-engine speed ranking or universal non-interleaving guarantee follows.
The backward-insertion diagnostic confirms a native interleaving limitation; this
does not change convergence semantics or license a hand-written merge override.

## Required clarification before accepting a native profile

“Native counter continuity” must not mean gapless operation counters per actor.
The pinned engine allocates a change's operation counter after the **exact base
cut's maximum counter**, so observing another actor permits a jump. Native change
sequences remain gapless per actor; outer batch sequence and IDs are independent.
The existing executable jump test checks this interpretation.

This is a proposed clarification, not a silent format change. I03 must disposition
the metadata syntax, supported counter widths, exact seed/profile versions and
decoder/resource limits explicitly.
Those exact profile details are I03 deliverables, not a requirement to finish
I03 before it can start. The meaning of the native counter rule, however, is an
I02 feasibility decision because it changes which histories are accepted.

## Stop/go and remaining cost decision

**GO:** continue the proposed native approach using retained original bytes.
**NO-GO:** public native-format freeze, production-safe admission, or starting I03.

Local correctness fixtures and native-only cost baselines have passed. The
experimental inspector still clones/forks/scans historical state and is not an
optimized or resource-hardened publication interface. Compressed native input is
refused before upstream inflation; raw byte limits and tested negatives do not
prove bounded work for every malicious column encoding.

[Native Windows, Debian WSL2 and refreshed macOS resource observations](../validation/I02-platform-results.md)
are now available from identical input inventories. Windows memory is only a
sampled lower bound. WSL2 remains a VM observation, not independent reference
Linux; the accepted amendment permits its provisional use for I02 only.
No remote infrastructure was provisioned. W01's native timings do not include
`edit_at`/`inspect`; W02's process maximum includes original/reloaded documents
and temporaries. Do not claim production latency/memory budgets from these numbers.

The [cost assessment](../validation/I02-cost-assessment.md) records the numerical
evaluation and engineering GO recommendation with its exclusions. No observed
native W01/W02 result requires an engine replacement; first-edit and retained
history costs remain substantial. This is not owner acceptance of costs or a
resource upper bound.

## Candidate direction remains proposed

The common baseline records favorable observations for additional candidates,
but PuyuDB-fit validation is not equally complete. Prioritize the proposed
bounded Loro-fit experiment before engine selection; keep AM as the reference.
This document's AM-specific direction is retained for acceptance or supersession,
not silently frozen from native timing results:

1. Continue with pinned Automerge 0.12.0, epoch-owned deterministic lazy seeds,
   retained original native bytes/history and distinct native/outer identities.
2. Accept the counter clarification above: gapless native change sequences per
   actor, but operation-counter allocation follows the exact base cut's maximum,
   not the actor's last operation or the replica's unrelated current history.
3. Accept the native-feasibility cost assessment under the approved WSL2 scope,
   with no durable-latency, hardened-admission or production memory guarantee.

Acceptance would close I02's direction/feasibility decision, not freeze a
persistent or wire format. I03 then owns canonical records, tags, exact profile
syntax/versioning, supported widths/limits and signed compatibility vectors.
The current `i02` fixture metadata and experimental limits are not thereby made
normative. No I03 implementation or format freeze is authorized by this proposal
while it remains unaccepted. If another engine is proposed, its native identity
and counter binding needs its own evidence-backed amendment; the AM counter
rule must not be treated as a universal candidate requirement.
