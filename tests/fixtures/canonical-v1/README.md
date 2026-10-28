# Canonical profile v1 fixtures

Generated, committed, and compared byte-for-byte with their generator:

```sh
python3 scripts/i03_canonical_vectors.py
python3 -m unittest discover -s scripts -p 'test_i0*.py'
```

Source of truth: [`docs/spec/canonical-profile-v1.md`](../../../docs/spec/canonical-profile-v1.md).
Nothing here is a release format: the Rust codec lands in I03 T4 and must
reproduce these bytes (T5).

| File | Contents |
| --- | --- |
| `vectors.json` | 20 positive records — every frozen layout — with expected digests and, where a signature is involved, the exact preimage/signature/public key |
| `negative.json` | 50 rejected records with the stable error code each must produce, plus 17 path cases |

Provenance:

- Every body is hand-encoded from the frozen layouts by
  `scripts/i03_canonical_vectors.py`; no CBOR library and no Rust code is
  involved, so agreement with the codec is evidence about the specification.
- Inputs are deterministic (`sha256` of fixed labels), so regeneration is
  idempotent and the committed files must match exactly.
- Ed25519 signing uses the RFC 8032 reference procedure with a fixed seed. The
  module self-checks against RFC 8032 §7.1 TEST 1/2, and `test_i03_*.py`
  re-asserts those known answers; without them no signature vector would mean
  anything. `batch/signature` and `branch/envelope` therefore freeze one
  deterministic signature per preimage that `ed25519-dalek` must reproduce.
- `native/empty-seed` freezes the accepted empty-baseline case: a zero-length
  native payload whose artifact identity still differs per epoch.

Deliberately not covered here, because it needs history rather than one record:

- Writer-stream equivocation, ancestor dominance in frontier normalization, and
  native dependency scope against the declared cut (T4/T5).
- Payload hydration checks: `artifact_id` binds kind, length, raw digest and
  chunk list at manifest level; verifying that the resident chunks actually hash
  to those IDs happens during hydration.
- Ed25519 `verify_strict` hardening (small-order `A`/`R` rejection). The Python
  check is the fixture-level equation `R + H(R,A,m)A == S B`; strict rejection is
  the Rust binding's obligation.
