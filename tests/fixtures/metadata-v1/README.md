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

Provenance:

- Every body is hand-encoded from the frozen layouts by
  `scripts/i05_semantic_manifest.py`; no CBOR library and no Rust code is
  involved, so agreement with the codec is evidence about the specification.
- Inputs are deterministic (`sha256` of fixed labels), so regeneration is
  idempotent and the committed files must match the generator exactly
  (asserted by `test_i05_semantic_manifest.py`).
- The validator re-implements the canonical CBOR subset (§1.1) with strict
  minimal-encoding and full-exhaustion checks, and rejects non-canonical
  forms rather than normalizing them.

Deliberately not covered here, because it needs a history database rather
than one record: register evaluation, causal supersession and conflict
derivation itself — `crates/puyudb-core/src/projection.rs` owns those and its
tests must agree with these golden bytes.
