# Audit prompt B — security (I03)

Paste this whole file as the task in a **fresh session with no prior context**,
in a **separate session from audit A** (sharing context would inherit A's
conclusions). Repository root:

`/Users/pixel/Downloads/puyu/puyuDB` (branch `i03-identity-batch-profile`).

Nothing here is a finding. It is an attack plan, not a bug list.

---

## Role

You are an adversarial security reviewer. The code under review is a canonical
binary format layer: a strict CBOR decoder, record validators, hash derivations,
an Ed25519 signature layer, path rules and writer-stream checks. It is not yet
wired to storage, network or a database, which means **your findings are cheap to
fix now and expensive to fix later** — treat that as a reason to look harder, not
softer.

Assume the author is wrong. The corpus and tests are the author's product: a
previous round shipped a derived-name bug that all 23 of its own tests blessed.
Derive expected results from `docs/spec/canonical-profile-v1.md` yourself.

## Read fully before attacking

- `docs/spec/canonical-profile-v1.md` — §1.1 (CBOR subset), §4 (batch/signature),
  §6 (artifacts/chunking), §7 (metadata), §8 (epoch/descriptor), §9 (conflicts),
  §10 (limits), §11 (paths), §12 (native profile, actor names), §13 (streams),
  §14 (receipts/codes).
- `crates/puyudb-core/src/canon.rs`, `identity.rs`, `profile.rs`.
- `crates/puyudb-core/tests/canonical_vectors.rs`,
  `tests/fixtures/canonical-v1/{vectors.json,negative.json,README.md}`.
- `docs/validation/I02-diamond-types-assessment.md` — the "other executed
  native-wire findings" section and the cursor counterexample. Those are *settled
  I02 limits*; do not re-open them, but do check the profile does not claim more
  than they support.
- `docs/DEPENDENCIES.md`, `deny.toml`, `THIRD_PARTY_NOTICES.md` for the crypto
  dependency posture.

## Attacks you must actually attempt

Do not just read the code. Construct inputs and run them (scratch files in
`/tmp`, never inside the repo).

1. **Allocation before bounds.** Trace every path from `canon::decode` into
   `profile` validation and find a case where a length is trusted before a cap
   check: array lengths, text/byte lengths, chunk counts, frontier heads, parent
   counts, metadata operations, context events. Try `u64::MAX`, boundary values
   (exactly at and one past every cap in §10), and deep nesting. Report measured
   behaviour, not theory.
2. **Canonicality bypass.** Hunt for an input that the decoder accepts but the
   canonical re-encoding does not reproduce, or a *non-canonical* input that
   passes validation: non-minimal integers at every position, indefinite lengths,
   trailing bytes inside a nested element, invalid UTF-8, duplicate or unsorted
   lists, empty required fields, `null` where a value is required and vice versa.
3. **Signature layer.** Can two different messages produce one preimage (length
   ambiguity in `space || batch` / descriptor body)? Is any domain separator
   reused across meanings? Does `verify_strict` really reject small-order `A`/`R`
   and `S >= L` — prove it with a constructed input. The batch signature preimage
   binds only space and body digest; the body binds `public_key`. Is the
   "signature must match the body's key" claim in §4.2 actually enforced by any
   code, or only stated? If only stated, is that a hole or a documented caller
   obligation?
4. **Writer stream.** Bypass `validate_stream_step`/`detect_equivocation`: same
   key but different branch, previous pointing into another stream, `sequence = 0`,
   `lamport` overflow, a `previous` that is not a parent, two bodies that differ
   only in an artifact list. Say precisely what a caller must check that this code
   does not.
5. **Paths.** Attack §11: NFC/NFD pairs, case variants of `.git`/`.puyu`, `..`
   hidden by composition, backslashes, trailing dot/space, Windows device names
   (`CON`, `NUL`), exactly-255-byte components, exactly-4096-byte paths, 4-byte
   UTF-8, lone surrogates via malformed UTF-8. Then state which *materialization*
   conflicts the spec deliberately leaves to a later profile — and whether the
   spec says that honestly.
6. **Artifact framing.** Trust relationships between `raw_length`, chunk count
   and chunk order: exactly `CHUNK_RAW_BYTES`, exactly `MAX_ARTIFACT_RAW_BYTES`,
   one past each, zero-length artifact, 4 096 vs 4 097 chunks, kind-string
   tricks (case, trailing NUL, 64 vs 65 bytes, a reserved kind, a kind that
   prefixes an assigned one). Does manifest validation ever imply the payload was
   verified when it was not?
7. **Conflicts and semantic manifest.** Can a `ConflictId` or
   `SemanticManifestId` be made arrival-order dependent? Can participants be
   forged (they carry only batch id + index — what proves they exist)?
8. **Trust-boundary inventory.** `validate_record` is not admission. Enumerate
   every assumption a caller must add before a batch can be considered
   admissible: DAG ancestry, minimal-frontier normalization, native dependency
   scope against the cut, hydration state, quota. This list is the deliverable
   that stops I04 from mistaking "validated" for "safe to apply".
9. **Fuzz-lite.** Write a throwaway mutator over the committed vector bytes
   (single-byte flips, truncations, splices) and assert the decoder/validators
   never panic and never accept a mutated body as valid. Report trial counts and
   any panic with the exact bytes.

## Output format

1. **Findings table**: `ID | severity | location | attack | impact | evidence
   (exact bytes/command) | fix class`. Severity: `blocker` (memory safety,
   panics, unbounded work, signature/identity forgery, canonicality bypass),
   `major` (a rule that exists on paper but is unenforced, or a false doc claim),
   `minor`, `nit`.
2. **Documentation gaps**: not exploitable, but a later implementer would guess.
3. **Residual risk**: what this audit did **not** cover — the DT native decoder,
   storage/durability, network, timing/side channels, fuzzing at scale, the
   Ed25519 library internals, license/notice compliance.
4. **Cheapest next action** per finding.

## Constraints

- Read-only for the repository: change no file, add no dependency, create no
  fixture inside it. Scratch scripts of your own are fine outside.
- Every finding needs a reproduction or an exact code location plus the input
  bytes. No speculative severity.
- Do not re-open settled I02 limits (deleted-gap cursors, non-canonical native
  re-encoding, decoder hardening of diamond-types itself) unless the profile
  contradicts the evidence.
- Do not weaken validation to make a test pass, and do not propose a new
  dependency unless a missing capability is a blocker; cost it either way.
