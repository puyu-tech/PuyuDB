# Metadata artifact v1 fixtures (I05 P5)

Generated, committed, and compared byte-for-byte with their generator:

```sh
python3 scripts/i05_semantic_manifest.py --write   # regenerate
python3 scripts/i05_semantic_manifest.py           # verify, nonzero on mismatch
python3 -m unittest discover -s scripts -p 'test_i0*.py'
```

Source of truth: [`docs/spec/canonical-profile-v1.md`](../../../docs/spec/canonical-profile-v1.md)
§1.1, §7.1, §7.2, §8.1–8.2, §9, §11; plan: `docs/plans/I05-native-text-projections.md` P5.
Nothing here is a release format: the Rust codec in `crates/puyudb-core/src/profile.rs`
and `projection.rs` must reproduce these bytes.

| File | Contents |
| --- | --- |
| `scenarios.json` | 5 metadata-artifact scenarios (create+rename, concurrent path alternatives, delete+concurrent edit, mode conflict, resolution) — structured operations plus the expected canonical hex of each operation, the artifact, epoch descriptors, conflict records, the manifest, and every `EpochId` / `ConflictId` / `SemanticManifestId` |
| `negative.json` | 10 negative cases: unsorted participants / epochs / entries / paths (rejected and hash-diverging), wrong and cross domain separators (hash-diverging), and unassigned registry tags |

## Two kinds of goldens

**Record-encoding goldens** — all the hex and digests above, plus the 10
negative cases. These are pure §1.1/§7.1/§7.2/§8/§9/§11 encodings: every body is
hand-encoded from the frozen layouts by `scripts/i05_semantic_manifest.py`
with no CBOR library and no Rust code, the validator re-implements the
canonical CBOR subset (§1.1) with strict minimal-encoding and full-exhaustion
checks and rejects non-canonical forms rather than normalizing them, and the
inputs are deterministic (`sha256` of fixed labels), so regeneration is
idempotent and the committed files must match the generator exactly (asserted
by `test_i05_semantic_manifest.py`). Agreement with the Rust codec is
therefore evidence about the specification, not about a shared dependency.

**Derivation-semantics goldens** — the semantic facts pinned inside the
scenarios, which mirror the derivation rules implemented in
`crates/puyudb-core/src/projection.rs`:

- a tag-8 resolution with tag 0 (RetainDelete) on the kind-1 `delete_edit`
  conflict carries the fresh same-batch tag-2 `Deleted` liveness assignment —
  the "New `Deleted` liveness assignment" of spec §9, enforced in Rust by
  `has_live_op` in `validate_resolutions` — and the validator rejects any
  scenario whose resolution lacks its same-batch effect or whose tag does not
  match the named conflict's kind (§22.2 `kind_matches`: tags {0,1} only on
  kind-1 `delete_edit`, tags {2,3} only on kind-2 `epoch_edit`, tag 4 only on
  kind-0 register conflicts);
- after that resolution the recomputed cut carries no `ConflictId`: the
  manifest entry's `conflicts` list is empty and matches the scenario's
  derived-conflicts list, as `tag8_resolution_publishes_fresh_assignment_and_supersedes_alternatives`
  (`crates/puyudb-core/src/publication.rs`) proves for the Rust path;
- a kind-1 `delete_edit` conflict's `epochs` hold the edited epoch only, not
  every maximal epoch selection of the file (`conflicts_impl`).

The full Rust cross-check of these derivation semantics — re-deriving every
scenario cut from a history database in `crates/puyudb-core/src/projection.rs`
and comparing the derived conflicts and `SemanticManifestId` against these
golden bytes, i.e. the T01–T03 parity evidence — is a **P9 deliverable**
(`docs/plans/I05-native-text-projections.md`, P9), not part of this corpus:
it needs a history database rather than a single record.
