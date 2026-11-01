# I03 — Freeze identity and batch profile: implementation plan

**Status:** Proposed, awaiting maintainer go-ahead (see Gate 0).
**Blueprint:** `PUYUDB-BLUEPRINT.md` §18.3 row I03 — "Freeze identity and batch profile:
canonical CBOR/schema/tags, typed IDs, actor streams, frontier normalization, artifacts,
signed test vectors". Depends on I02 (closed 2026-10-26). Placement: `core/types` later.
Acceptance: T02, T03, T08, T13; no implicit algorithm choices. Estimate: **L**
(1–3 engineer-weeks).
**Branch:** new `i03-identity-batch-profile` off `i01-repository-foundations` @ `78a4fb2`.

## 1. Current state (verified 2026-10-26)

- Branch `i01-repository-foundations`, clean tree, up to date with origin. No GitHub
  issues exist; issue IDs are plan IDs on the branch (I01/I02 convention).
- Workspace: `puyudb-core` (documented boundary only), `puyudb-crdt` (I02 experiments;
  DT/Loro/Yrs/AM are dev-only), `puyudb-local` (boundary only), `puyu-cli` (help/version).
- I02 closed with **diamond-types (DT)** selected (ADR-01 accepted): pinned upstream
  rev `89ae3a0…` (manifest 2.0.0, unpublished, no-LZ4), small reviewed patch set in
  `experiments/diamond-types-interfaces/` (`anchors.patch`, `strict.patch`) **not yet
  promoted into a production binding**.
- Toolchain Rust 1.99.1 (also MSRV), edition 2024, `unsafe_code = "forbid"`,
  `missing_docs = "warn"`, clippy `-D warnings`, `cargo deny --locked`, locked offline
  build. CI: fmt, clippy, `python3 -m unittest discover -s scripts -p 'test_i02*.py'`,
  test matrix, dependency policy.
- I02 recorded NO-GO: starting I03 without an explicit maintainer go-ahead, or freezing
  a native format without the stop/go evidence. This plan is the vehicle for that
  go-ahead.

## 2. Key decision to ratify first (ADR-01 amendment)

The blueprint text still says Automerge (`am-text-v1`, `puyudb/am-seed-actor/v1`,
§5.1/§5.4/§22.1/§22.2). I02 decided **DT**. I03 therefore starts with an ADR-01
amendment (single approved semantic decision, per §18.4 "do not parallelize tightly
coupled semantic choices across agents without a single approved ADR and test corpus"):

1. Native text kind tag is frozen as `dt-text-v1`, bound to the pinned DT revision plus
   the reviewed patch set as the production binding.
2. Seed/actor derivation namespaces are re-specified engine-bound for DT
   (`puyudb/dt-seed-actor/v1`, `puyudb/dt-actor/v1` replacing the `am-*` drafts), keeping
   the §22.2 ownership rules: seeds are epoch-owned baseline dependencies, never owned by
   the first editing batch; the leading `0x00`/`0x01` tag prevents namespace overlap.
3. Blueprint references to `am-*` are superseded; a blueprint rev 0.3 note records this
   (blueprint lives outside the repo; maintainer applies it).
4. The DT lazy-seed profile is re-frozen for DT with golden cross-process seed fixtures
   (I02 `docs/validation/diamond-fit/` evidence is the starting point; the §5.4
   lazy-seed stop/go gate applies to the DT binding).

## 3. Scope

**In scope (I03):**

- Frozen canonical profile spec `docs/spec/canonical-profile-v1.md`: record layouts,
  ID derivations, domain-separated SHA-256 preimages, signature preimage, tag
  registries (metadata ops 0–9, mode tags, epoch kind tags, context schema tags 0–7),
  branch-descriptor record, conflict-ID derivation and resolution tags 0–4, receipt
  vocabulary (`Stored`/`Applied`/`Hydrated`), ID textual presentation (`batch1:…`
  base32/hex), path validation rules, writer-stream/sequence/Lamport/parent rules,
  frontier normalization, intent-base publication-ancestry rule (`causal_fork`),
  chunk/artifact manifest framing (256 KiB chunks, 1 GiB initial opaque cap as a
  *declared constant*, not a quota system).
- Rust implementation in `puyudb-core` as internal modules (no `puyudb-types` crate yet
  — extraction gate per §4.3/§15.2): `identity` (typed IDs + presentation),
  `canon` (strict RFC 8949 core-deterministic CBOR subset for the fixed array schemas:
  definite lengths, shortest ints, no floats, no indefinite values, no duplicate keys —
  all statically enforceable for fixed schemas, so **hand-rolled, no new CBOR
  dependency**), `profile` (records, encode/decode/validate/hash), Ed25519
  sign/verify (new pinned `ed25519-dalek` dependency; OS RNG via `rand` `OsRng` —
  §15.3 "prefer Ed25519, do not invent crypto"), path validation, writer-stream
  sequence/Lamport/equivocation checks.
- Golden test vectors + **independent codec validator**: `tests/fixtures/canonical-v1/`
  (hex bodies, expected IDs/digests, Ed25519 signing vector) and a Python
  generator/validator pair `scripts/i03_canonical_vectors.py` +
  `scripts/test_i03_canonical_vectors.py`, following the `test_i02*.py` convention and
  wired into CI (extend the unittest pattern to `test_i0*.py`). This is the T13
  "independent codec validator".
- DT production binding: patch set promoted to a pinned production dependency of
  `puyudb-crdt` (decision needed: puyu-tech fork rev with patches applied vs. upstream
  PR — default is the fork rev, recorded with provenance); DT moves from dev-dependency
  to production dependency; `automerge`/`loro`/`yrs` stay dev-only. Lockfile,
  `deny.toml`, `THIRD_PARTY_NOTICES.md`, `docs/DEPENDENCIES.md` updated per the
  dependency change policy.
- Tests mapping: **T02** (canonical re-encode equality, reload/round-trip fixture
  stability, old-version identity preservation), **T03** (path-rule negatives,
  metadata op tag validation, conflict-ID stability, MV write identity
  `(batch32, operation_index)`), **T08** (tampered body, wrong key, unknown
  tag/version, non-canonical CBOR rejection, `(stream, sequence)` equivocation),
  **T13** (golden byte corpus, Rust↔Python validator agreement, N/N-1 unknown-tag
  negotiation).

**Explicitly out of scope (later issues):** SQLite schema/publication (I04, ADR-02),
metadata/text projections (I05), engine publication and single-writer (I06), Git
import (I07), queries (I08), CLI commands, daemon/IPC, replication protocol (I20+),
quotas/reservations and large-blob reservation mechanics, credential/auth ordering and
remote signing, checkout/external-effect state (§22.2 items owned by I15/I19/I24/I11).
No CLI surface changes in I03.

## 4. Work breakdown and subagent orchestration

Tightly coupled semantics stay single-authored (§18.4). Parallel only where file
boundaries are disjoint.

| # | Ticket | Owner | Depends | Files |
|---|---|---|---|---|
| T0 | Record maintainer go-ahead; create branch `i03-identity-batch-profile` off `78a4fb2` | maintainer + coordinator | — | git only |
| T1 | ADR-01 amendment + frozen spec `docs/spec/canonical-profile-v1.md` (single author — the semantic spine) | coordinator | T0 | `docs/adr/01-native-text-evidence.md`, `docs/spec/`, `docs/adr/README.md` |
| T2 | Golden vectors + independent Python validator + CI pattern | subagent A | T1 | `tests/fixtures/canonical-v1/`, `scripts/i03_*`, `scripts/test_i03_*`, `.github/workflows/ci.yml` (pattern only) |
| T3 | Dependency plumbing: DT production binding + `ed25519-dalek` for core + lock/deny/notices/DEPENDENCIES | subagent B | T1 | `Cargo.toml`, `Cargo.lock`, `crates/*/Cargo.toml`, `deny.toml`, `THIRD_PARTY_NOTICES.md`, `docs/DEPENDENCIES.md` |
| T4 | Rust implementation in `puyudb-core` (`identity`, `canon`, `profile`) + unit/integration tests | subagent C | T1, T3 | `crates/puyudb-core/src/**`, `crates/puyudb-core/tests/**` |
| T5 | Integration: Rust↔golden-vector cross-tests (T13), full CI green (fmt, clippy `-D warnings`, tests, deny, unittest) | coordinator | T2, T4 | as needed |
| T6 | Closure docs: `docs/IMPLEMENTATION_STATUS.md` (I03 closure), CHANGELOG, GLOSSARY additions | coordinator | T5 | `docs/`, `CHANGELOG.md` |

Sequencing: T0 → T1 → (T2 ∥ T3) → T4 → T5 → T6 → audits (§6).
T2 and T3 are genuinely independent (Python-from-spec vs. dependency plumbing) and touch
disjoint files. T4 must wait for T3 because both would otherwise edit
`Cargo.lock`/`Cargo.toml`. No git worktree needed: subagents share the branch
sequentially in phases, not concurrently.

Commit policy: checkpoint commits on the branch in the existing style
(`chore:`/`feat:` + English message + Mistral Vibe co-author trailer). **No push, no
merge, no PR, no GitHub issue creation without maintainer approval** (I01/I02
convention; push is a critical action).

## 5. Definition of done (I03 closure)

- ADR-01 amendment approved; spec `docs/spec/canonical-profile-v1.md` frozen and
  reviewed; no implicit algorithm choices (no SQLite/engine selection in this issue).
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`,
  `cargo test --workspace --frozen` (also `--no-default-features` and `--all-features`),
  `cargo deny --locked check`, and the Python unittest suite all green on Rust 1.99.1.
- Golden vectors exist for every §22.1 record, and the independent Python validator
  agrees with the Rust codec byte-for-byte (T13); negative corpus covers T08.
- DT production binding pinned with provenance; `docs/DEPENDENCIES.md`, `deny.toml`,
  `THIRD_PARTY_NOTICES.md` updated; locked offline build verified.
- Phase-0 done criteria from §18.2: written stop/go results and fixed semantic profiles.
- Audits (§6) completed; findings resolved or recorded as accepted debt.

## 6. Audit phase (after T6, before closure)

Audits run in subagents on model **glm-5.3 max** (`zai-glm-5-3`, alias `glm-5-3`,
`thinking = "max"`). Mechanism verified empirically on 2026-10-26 against the
installed `mistral-vibe` / `mistralai_vibe_local_harness`:

- `agents.spawn` takes only `name`/`message`/`config_name`; `config_name` is a
  **subagent type** (an agent profile), not a model. `config_name: "zai-glm-5-3"`
  fails with `subagent_type_not_found`; even the builtin `explore` is not bound in the
  current session — only a generic spawn (no `config_name`) works.
- The local harness's `DeclaredAgentTypeProfile` has **no model or thinking field**;
  the child config is a deep copy of the parent session config
  (`_subagents/_configuration.py::_child_core_config`). Verified empirically: a probe
  subagent reported "Mistral Large 4" — the session's active model. **A subagent
  always inherits the session's active model and its thinking level.** No custom
  subagent profile can pin a different model or thinking level.

Consequence: to audit on glm-5.3 max, the maintainer switches the session to
`glm-5-3` with `thinking = "max"` **before** the audits — `/model glm-5-3` +
`/thinking max` in the TUI, or `active_model = "glm-5-3"` plus `thinking = "max"`
on the `zai-glm-5-3` entry in `~/.vibe/config.toml` (currently `"high"`; back up
the file before editing). Audit subagents are then plain generic spawns (no
`config_name` needed) and inherit glm-5.3 max from the session; the coordinator
spawns them on request, or the maintainer runs them directly. Switching the
session model is a session-level action the coordinator will not perform
unilaterally. Planned audits:

1. **Spec-conformance audit:** frozen spec + Rust profile vs. blueprint §5.2, §5.5,
   §5.6, §22.1, §22.2; ADR-01 amendment consistency; tag registries complete and
   reserved tags honored.
2. **Security audit:** canonical CBOR decoder (malformed/truncated/non-canonical
   inputs, oversized lengths before allocation), signature preimage and verification,
   equivocation handling, path validation, ID length checks before allocation (T08
   corpus review; ponytail-review style).
3. **Dependency/provenance audit:** DT patch-set promotion (fork rev vs. upstream),
   license/notice coverage for the ISC gap recorded in I02, `deny.toml` exceptions,
   lockfile reproducibility.

Findings are fixed or recorded in `docs/IMPLEMENTATION_STATUS.md` before I03 is marked
closed.

## 7. Risks and open questions

- **DT patch-set promotion route** (fork rev vs. upstream PR) needs a maintainer
  decision; default is a pinned fork rev with provenance in `docs/DEPENDENCIES.md`.
- **DT seed determinism:** the §5.4 lazy-seed gate was written for AM; the DT seed
  profile must be re-frozen with golden cross-process fixtures (I02 diamond-fit seed
  evidence is positive but was not a frozen profile).
- **Blueprint drift:** `am-*` namespaces remain in the blueprint until rev 0.3; the
  spec file is normative for I03 and says so explicitly.
- Hand-rolled CBOR subset is acceptable only because schemas are fixed arrays; if I20
  later needs generic CDDL, re-evaluate a pinned maintained CBOR library (§15.3).
- Estimate L assumes the ADR-01 amendment is approved promptly; contested semantics
  re-open Phase 0, not I04.
