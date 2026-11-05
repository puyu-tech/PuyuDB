# I06 validation — exposed embedded transaction interface (T04/T12 evidence through the public API)

**Status: I06 complete on `i06-embedded-transaction-interface`. Phase 1
(P1+P2) = commit `7a7308d`, phase 2 (P3+P4) = commit `25c6531`, phase 3
(P5, this file's evidence) = the branch-head commit with subject
`i06 phase 3: t04/t12 evidence through the public api and validation docs`.
All gates green at each checkpoint. The living plan and gate log stay in the
Maestri note `I06-IMPLEMENTATION-PLAN` (never committed).**

Normative sources: blueprint §3.1 R01/R05/R07, §6.4–§6.5, §8.1, §17.1
(T04/T12), §22.2; `docs/spec/canonical-profile-v1.md` §13–§14. Implementation:
`crates/puyudb-core/src/api.rs` (public facade — the only entry path the new
evidence drives), `crates/puyudb-core/src/publication.rs` (ledger, owner
lock, CAS, network-FS rejection). The plan lives only in Maestri; no
`docs/plans/I06-*.md` exists by design.

## What landed

- **P1+P2 (`7a7308d`)** — facade `puyudb_core::api::Database`
  (`create_space`/`open_space`, `DatabaseBusy` with a non-secret owner-lock
  hint, create-on-existing / open-of-missing = `precondition_failed`), the
  §14 stable codes on `api::Error::code()` (twelfth code `unsupported_version`
  added in phase 2 per Gate A), `Database::transact(caller, key,
  expected_frontier, publication)` with the branch-frontier CAS inside the
  `BEGIN IMMEDIATE` closure after the idempotency/visible-batch checks and
  before admission (a retry of an own committed batch returns the stored
  receipt before the CAS, spec §13.1). No Tokio, no network, no CLI in core;
  `Cargo.toml` untouched.
- **P3+P4 (`25c6531`)** — read API on the handle, all read-only, no engine
  lock across calls, every read takes an explicit cut `&[BatchId]` as the
  version identity until I08: `bootstrapped`, `branch_frontier`, `files_at`,
  `paths_at`, `prefix_conflicts_at`, `epochs_at`, `conflicts_at`,
  `semantic_manifest_at`, `text_at`, `artifact_availability`, `is_published`,
  `receipt_for`. `artifact_availability` documents §22.2 exactly: `Resident`
  is what local publication writes after the artifact's own chunks were
  verified in-commit; no closure-wide hydration pass is run, so `Resident`
  is not a `Hydrated` claim for the whole closure. Owner lock: flock-based
  (no PID text ever written or trusted — a stale PID can never break a live
  lock); drop/crash releases it.
- **P5 (phase 3)** — this document, the fault tests below, Gate B fixes C1/C2/N1/N2.

## T04 durability evidence (through `api::Database` only)

Invariant table (blueprint §17.1 T04: "local-origin durable receipt preserves
new content/batch/events/outbox; no partial publication"):

| Scenario | Ack? | After reopen |
|---|---|---|
| process kill before publish (`api_crash.rs::kill_before_publish_releases_lock_and_leaves_nothing_visible`) | no | owner lock released by the crash (fresh open succeeds — Gate B N2); `bootstrapped() == false`, frontier empty; a later keyed transact commits and is durable |
| process kill after ack (`api_crash.rs::kill_after_ack_keeps_the_batch_visible_and_the_receipt_stable`) | yes | batch fully visible after reopen (`is_published`, `branch_frontier == [batch]`, `receipt_for`); retry with same key+content returns the SAME receipt (§13.1 unknown-after-commit) |
| lost sync during transact (`api_power.rs::lost_sync_during_transact_...`) | no | error code `internal` (environmental, not §14, never mislabeled as corruption); nothing visible, no receipt row; key reusable (cancel-before-publish), retry commits |
| torn write during transact (`api_power.rs::torn_write_during_transact_...`) | no | same as above |
| disk full during transact (`api_power.rs::disk_full_during_transact_...`) | no | same as above |
| acked transact, later armed fault (`api_power.rs::acked_transact_survives_a_later_armed_fault`) | yes | ack implies durable: batch + receipt survive reopen; retry returns the same receipt |
| CAS mismatch (`api_facade.rs::cas_mismatch_fails_precondition_and_leaves_key_reusable`) | no | `precondition_failed`, nothing visible, key reusable |
| same key, different content (`api_facade.rs::same_key_different_content_fails_precondition`) | no | `precondition_failed`, original receipt untouched |

The fault-VFS harness is the existing `sqlite-fault` shim, now driven through
`Database::create_space_vfs`/`open_space_vfs` (doc-hidden hooks mirroring
`Store::open_vfs`); the Store-level corpus in `publication_power.rs`/
`publication_crash.rs` remains as I04 evidence.

## T12 offline embed acceptance (blueprint §17.1 T12, §3.1 R01/R05/R07)

`api_facade.rs::offline_embed_acceptance_no_network_no_account_no_daemon`:
a full create → keyed transact → frontier/bootstrapped reads → drop → reopen
→ `receipt_for`/`is_published` cycle in a fresh temp dir, plus a structural
dependency pin that reads `crates/puyudb-core/Cargo.toml` at test time and
asserts it declares no tokio/hyper/reqwest/tonic/async/socket/daemon
dependency — no declared dependency can open a socket on the embed path
(cargo deny's bans gate is the complementary CI check). No network mocks are
faked; the whole suite runs offline. Error-code stability: `api_facade.rs::error_code_strings_cover_the_twelve_stable_codes`
pins all twelve verbatim spec §14 strings plus the `internal` sentinel;
`text_at_error_paths_map_to_the_real_stable_codes` pins the real mappings
(Gate B C1): missing seed ⇒ `missing_dependency`, undeclared epoch at the cut
⇒ `invalid_operation` (the `text_at` doc comment now states exactly this).

## Gate B resolutions

- **C1** — `Database::text_at` doc fixed to the real mapping and pinned by
  test (above).
- **C2** — network-FS rejection for writable opens landed Linux-only and
  std-only in `publication.rs`: the space dir's device via
  `MetadataExt::dev()` (kernel major/minor decoding) matched against
  `/proc/self/mountinfo` with a longest mount-point prefix match on the
  canonicalized path; a conservative known-network list
  (`nfs, nfs4, cifs, smbfs, smb2, 9p, afs`) rejects the writable open with the
  environmental `publication::Error::NetworkFs` (facade: non-§14 `Internal`
  with detail). Unknown fstypes and absent mountinfo pass through — never
  mislabeled. Non-Linux platforms keep current behavior (comment at the call
  site). Parsing is factored into `network_fs_at`, unit-tested on Linux by
  `publication.rs::tests::network_fs_detection_matches_longest_mount_and_rejects_only_known_types`
  (longest-match, component-boundary prefixes, octal-escaped mount points,
  unknown-type pass-through).
- **N1** — `docs/IMPLEMENTATION_STATUS.md` phase-1 line corrected (eleven
  codes at `7a7308d`, twelfth with `25c6531`); I06 section extended to final
  state.
- **N2** — crash-releases-owner-lock pinned in the T04 fault tests above.

## Gate results (run at the phase-3 commit, from the repo root)

- `cargo fmt --check` — clean.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` — clean.
- `cargo test --workspace --frozen` / `--no-default-features` / `--all-features` —
  33 test suites, 245 passed, 0 failed in each config (14 api_facade +
  2 api_crash + 4 api_power + the existing suites).
- `python3 -m unittest discover -s scripts -p 'test_i0*.py'` — 67 tests, OK.
- `cargo deny --locked check` — advisories/bans/licenses/sources ok.

## Explicitly out of scope

Daemon/IPC (I17), CLI (I10), watcher (I11), branches UI (I12), version handles
(I08), restore (I09), Git import (I07), replication (I21), server (I23+).
Read APIs take an explicit cut of heads until I08 lands version handles.
The `publication` module stays `pub` because the I04/I05 fault tests drive
`Store` directly; tightening it to crate-private is deferred until those move
behind the facade. Non-Linux network-FS detection does not exist (portable
detection is not available); on those platforms writable opens keep the
flock + Busy behavior only.
