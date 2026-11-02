# I05 implementation plan — native and file projections

**Status: draft, kept outside git on purpose (working document).** When I05
starts, a copy belongs in `puyuDB/docs/plans/I05-native-text-projections.md`.

**Branch:** fresh `i05-native-text-projections` off `main` (the remote branch
of that name still points at the stale `2aec9d6`; recreate it from `main`).

**Normative sources:** `PUYUDB-BLUEPRINT.md` §18.3 row I05 ("Text integration
and metadata MV registers, liveness/epochs/opaque values/path collisions";
deps I02+I03; placement core/crdt; acceptance T01–T03 plus slow reference
parity; size XL), §5.2–§5.8, §6.3, §22.1–22.2; `puyuDB/docs/spec/canonical-profile-v1.md`
§6.3, §7, §9, §11, §12.5, §13; the I04 plan's out-of-scope note (projections and
native scope checks were deferred here); current
`crates/puyudb-core/src/publication.rs` (admission, artifacts, DAG) and
`profile.rs` (codec: `ArtifactManifest`, `NativeArtifact`, `EpochDescriptor`,
metadata operation tags).

**What I04 already gives I05:** signed-batch admission with stream/DAG/lamport
checks, content-addressed artifacts with hydration verification,
`content_availability`, `requests` idempotency, owner lock, fault-VFS tests.
`publish` currently refuses batches with native artifacts (`NativeUnchecked`) —
I05 lifts that only behind scope checks.

## Scope, in order

| # | Bullet | Blueprint / spec | Files |
|---|---|---|---|
| P1 | Metadata artifact decode + admission: validate operation tags 0–9 (§7.1), caps (`MAX_METADATA_OPERATIONS`/`MAX_CONTEXT_ASSERTIONS`), `relation_tag` opaque (width only), path rules (§11) on path ops | §7, §11 | `profile.rs`, `publication.rs`, fixtures |
| P2 | `field_writes` storage + MV-register evaluation: write identity `(batch_id, op_index)`, causal supersession within the batch's closure, maximal alternatives in stable bytewise order, smallest-identity display representative | §5.6, §6.3 | `publication.rs` (new module `projection.rs`?), tests |
| P3 | File projections: `files_current`, `paths_current`, liveness (`alive`/`deleted`, no implicit resurrection), path collision sets (never auto-resolved), file/directory prefix conflicts, epoch selection per file | §5.6, §6.3 | projection module, tests |
| P4 | Conflict derivation + resolution op (tag 8): stable `ConflictId`s (§9), cross-field delete/edit conflict rule, explicit resolution supersession | §5.6, §9 | projection module, `profile.rs` |
| P5 | Semantic manifest (§7.2): canonical `SemanticManifestV1` output at a cut, sorted/dedup rules, hash stability vs arrival order — the T02/T13 comparison unit | §7.2 | projection module, golden fixtures |
| P6 | Epoch lifecycle: epoch descriptor records (`text`/`opaque`/`symlink`), create (tag 0), epoch select/reset (tag 3, old epochs retained and queryable), opaque value assignment (tag 4), mode (tag 7) | §5.4, §8.2, §7.1 | `profile.rs`, projection, tests |
| P7 | Native text admission: `native_changes` table (epoch/native-hash → artifact, epoch-owned seed or unique producer), DT actor/seed derivations already in codec (§12.2–12.4), scope checks per §12.5 (actor name, root text object, op kinds, actor-sequence continuity, native parents within the batch parent cut → else `invalid_native_scope`), then lift `NativeUnchecked` | §6.3, §12.5, §6.3 native artifact | `publication.rs`, `puyudb-crdt` binding, tests |
| P8 | Native text reads: current text view per (file, epoch) from seed + changes; historical read at a version; UTF-8 byte-offset public positions (§5.5) | §5.5, §5.8 | core/crdt projection, tests |
| P9 | T01–T03 evidence: convergence/determinism fixtures, metadata register cases (incl. collision/conflict negatives), slow reference parity vs AM conformance path; semantic-manifest equality across two independent replicas of one cut | §17 | tests, `docs/validation/` |

## Explicitly out of scope (later issues)

Git import/baselines (I07), versions/frontiers/checkpoints (I08), restore (I09),
CLI (I10), watcher (I11), branches (I12), checkouts (I13), anchors/provenance
indexes (I16 — note: the accepted deleted-gap anchor limitation stays),
`relation_tag` semantics registry (owned by I05/I07 jointly — I05 validates
width only), quotas (I15), replication apply (I21).

## Definition of done

- P1–P8 landed with green gates: `cargo fmt --check`, clippy `-D warnings`,
  workspace tests in all three feature configs, cargo-deny, python unittest.
- T01–T03: convergence and register fixtures pass; two independent replicas of
  one closed cut produce identical `SemanticManifestId`; native scope
  negatives rejected with `invalid_native_scope`; slow reference parity vs the
  AM conformance path on the shared fixtures.
- Accepted I02 limitations unchanged: no canonical native encoding, deleted-gap
  anchors, no fabricated non-interleaving guarantee.
- `docs/IMPLEMENTATION_STATUS.md` records I05 progress; plan copied into
  `docs/plans/` at branch start.

## Commit policy

Checkpoint commits on the branch, English messages, trailers exactly:

```
Co-Authored-By: Mistral Vibe <vibe@mistral.ai>
Signed-off-by: p1xelll <81173399+p1xelll@users.noreply.github.com>
```

(no `Generated by` line). Push only to `i05-native-text-projections`; no PR/merge
without maintainer approval.

## Open questions / risks

- I05 is XL: split into reviewable slices (suggest P1–P5 first — metadata +
  projections without native; then P6–P8 native).
- `projection.rs` vs tables inside `publication.rs`: decide at P2, keep the
  smallest coherent seam.
- Register evaluation is O(candidate history) initially (§5.8); index later,
  do not over-build now.
- Native hydration stays verified per artifact; `Applied` vs `Hydrated` split
  from I04 (content_availability) must survive into native reads.
