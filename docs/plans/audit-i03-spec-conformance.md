# Audit prompt A — spec conformance (I03)

Paste this whole file as the task in a **fresh session with no prior context**.
Point the agent at the repository root:

`/Users/pixel/Downloads/puyu/puyuDB` (branch `i03-identity-batch-profile`).

Nothing in this file is a conclusion. It is a list of claims to falsify.

---

## Role

You are an independent reviewer auditing a frozen format specification against
its normative source, before the project declares that phase closed. You did not
write any of this and you owe it no deference. Your job is to find places where
the specification is wrong, incomplete, self-contradictory, or where the code and
the corpus agree with each other but not with the spec.

**The single most important instruction:** the committed fixtures and the Rust
tests are *not* authority. They share an author. A previous round produced a
bug where a validator derived actor names wrongly, compared them against its own
wrong output, and 23 tests stayed green. Self-consistency is cheap. Derive every
expectation from the specification text yourself.

## Read fully before judging

- `docs/spec/canonical-profile-v1.md` — the frozen spec under audit.
- `docs/adr/01-native-text-evidence.md` — the accepted engine decision and
  **Amendment 1**, which is supposed to re-specify Automerge-shaped semantics for
  diamond-types.
- `docs/plans/I03-identity-and-batch-profile.md` — what I03 promised.
- Normative source: `/Users/pixel/Downloads/puyu/PUYUDB-BLUEPRINT.md`, sections
  §5.1–§5.6, §15.3, §17.1, §20.2, §22.1, §22.2. Read them, not summaries.
- Implementations: `scripts/i03_canonical_vectors.py`,
  `crates/puyudb-core/src/canon.rs`, `crates/puyudb-core/src/identity.rs`,
  `crates/puyudb-core/src/profile.rs`,
  `crates/puyudb-core/tests/canonical_vectors.rs`.
- Claims to verify: `docs/IMPLEMENTATION_STATUS.md` (section "I03 T1–T4").
- Fixtures: `tests/fixtures/canonical-v1/{vectors.json,negative.json,README.md}`.

## Checks, in this order

1. **Record-by-record diff against §22.1.** For every record (batch body, batch
   ID, signature, frontier, version ID, chunk ID, artifact manifest, artifact ID,
   raw content digest, epoch descriptor, epoch ID, Git OID, native artifact
   payload, metadata artifact, semantic manifest, operation registry, mode/epoch/
   context tags, branch descriptor): field order, field meaning, widths,
   null-vs-absent, and every place the spec deviates from the blueprint. Classify
   each deviation as *documented and justified* or *undocumented drift*.
2. **Tag registries.** Gaps, overlaps, reserved tags, and whether "unknown tag is
   `unsupported_capability`" is actually implemented for every registry in both
   implementations.
3. **Domain separators.** For each digest in the spec's table: does the field
   list match the bytes actually hashed in Python and in Rust? Is any separator
   missing, reused for two meanings, or shaped so two record kinds can collide?
4. **Declared constants.** They are chosen, not derived: batch body 1 MiB,
   frontier 65 536 heads, 4 096 native artifacts, 65 536 metadata operations,
   4 096 chunks, 256 KiB chunks, 1 GiB artifact cap, 8 MiB text epoch, path
   4 096/255. Check them against each other and against §20.2/§22.1: is the
   arithmetic consistent (chunks × chunk size vs 1 GiB)? Does any cap make a
   legal blueprint case impossible? Is any cap unnecessary — state the removal
   argument if you think it is over-engineered.
5. **`SemanticManifestV1` and `ConflictV1`.** §22.1 lists a semantic manifest;
   I03 T1 invented its exact layout. Is "maximal alternatives at the viewed cut"
   normative enough to implement twice identically? Does the record as specified
   actually exclude arrival order, cache contents and native re-encoding, as it
   claims? Can two replicas legitimately disagree?
6. **ADR-01 Amendment 1.** §22.2 requires seed ownership, actor derivation,
   empty-seed handling, intent base, and original operation-span/publication
   binding. Does the amendment re-specify all of them for diamond-types, or does
   it silently inherit an Automerge-only rule? Is any `am-*` leftover still
   load-bearing anywhere?
7. **Silent drops.** The spec defers positions/UTF-16, anchor statuses, receipts'
   storage meaning, CDDL. Is anything else from §5.5/§5.6/§22.2 dropped without
   being named as deferred? Conversely: is anything in the spec a *new* semantic
   that the blueprint never authorized (beyond the accepted amendment)?
8. **Spec ↔ code coverage.** For each rule in the spec, is there a validator
   branch? List rules with no code, and code branches with no spec line.
9. **Python ↔ Rust divergence.** Read both implementations of the same record
   side by side and hunt for the case where both agree with each other but
   disagree with the spec text. They are allowed to differ only where the spec
   allows it.
10. **Claim check.** Re-run the read-only commands the status document claims
    (do not take them on trust):

    ```sh
    cargo fmt --check
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    cargo test --workspace --frozen
    python3 -m unittest discover -s scripts -p 'test_i0*.py'
    target/tools/bin/cargo-deny --locked check
    ```

    Report any claim you could not verify, and why.

## Output format

1. **Findings table**: `ID | severity | location | spec § | what is wrong | why it
   matters | fix class`. Severity: `blocker` (wrong format, ambiguous identity, or
   a claim that cannot be implemented twice identically), `major` (undocumented
   drift, missing rule, unverified claim), `minor` (wording that will be
   misread), `nit`. Fix class: `spec change`, `new/changed vector`, `code change`,
   `doc only`.
2. **Areas checked and found sound** — one line each, so silence is unambiguous.
3. **Not checked** — explicit list with reasons (no hosted CI, no storage layer,
   no engine binding, etc.).
4. **Cheapest next action** — for a `blocker`, say exactly what to change and in
   which file.

## Constraints

- Read-only: change no file, add no dependency, create no fixture. Scratch files
  only outside the repository.
- Quote the exact line or field you rely on. No claim without a location.
- Do not propose new features or broader scope. If something is over-engineered,
  say so and give the removal argument, but stay inside I03.
- Do not re-litigate settled I02 findings unless the spec contradicts them.
- If you cannot decide between "spec is wrong" and "code is wrong", say both and
  give the discriminating test.
