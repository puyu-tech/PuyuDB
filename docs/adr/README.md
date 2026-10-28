# Architecture decision index

The identifiers below reserve the proposed architecture's decisions. They do
not freeze formats or assert that their validation has passed. I01 implements the
four-package foundation of ADR-10 and the original-code licensing policy of
ADR-12 only. ADR-01's native-semantic direction is accepted (2026-10-26: DT
selected, I02 closed); ADR-01 also has an accepted, I02-only measurement-platform
amendment and the 2026-10-27 engine-bound profile amendment for I03 T1. I03
closed 2026-10-28 on that profile and the pinned fork; see the
[closure record](../I03-closure.md). The remaining directions are proposed, not
implemented contracts.

| ID | Proposed direction | Validation / implementation gate |
| --- | --- | --- |
| ADR-01 | Reuse native text convergence within PuyuDB-owned history | [DT direction accepted 2026-10-26](01-native-text-evidence.md); [engine-bound profile frozen 2026-10-27](01-native-text-evidence.md#amendment-1-2026-10-27-engine-bound-canonical-profile-for-diamond-types); fork pin and [I03 closed 2026-10-28](01-native-text-evidence.md); no native encoding canonicalized |
| ADR-02 | Patched SQLite, WAL/FULL, one owning writer | I04 publication and failure tests; no binding selected in I01 |
| ADR-03 | Immutable logical history separate from rebuildable caches | I03/I04/I08 closed-history fixtures |
| ADR-04 | System Git mutations; tested read-only library plumbing | I07/I14 real-Git fixtures, explicit effects |
| ADR-05 | Embedding plus optional single-owner daemon | I06/I17 lifecycle and platform tests |
| ADR-06 | Exact DAG replication with strict canonical CBOR profiles | I20–I22 independent codecs and simulator |
| ADR-07 | Shared local/remote application HTTP contracts | I17/I26 API parity and authentication tests |
| ADR-08 | Single-node server and per-space SQLite | I23–I25 recovery and resource isolation |
| ADR-09 | Authentication/authorization outside replicated user state | I24 negative authorization and revocation tests |
| ADR-10 | Four initial packages; extraction only at demonstrated seams | Foundation present in I01; no future empty crates or backend framework |
| ADR-11 | Rust embedding/client before optional language-native ABIs | I26–I29 independent integration examples |
| ADR-12 | Apache-2.0 original code/docs; DCO and provenance | Policy present in I01; distribution/legal reviews still required |
| ADR-13 | Flat full paths, stable files, explicit metadata conflicts | I05 reference-projector and collision tests |
| ADR-14 | Preserve logical history; no destructive distributed GC initially | I18/I19 lossless maintenance; any erasure requires I37 approval |

I02 must record stop/go evidence. A failed architectural gate requires an explicit
amendment, consequences, and approval before changing fundamental semantics.
Accepted decisions must later be superseded explicitly rather than silently
rewritten. Native, history, storage, transport, and package versions are separate
compatibility contracts.
