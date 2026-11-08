# I08 validation — history, versions, checkpoints (T02/W07 evidence)

**Status: I08 Phase 1–3 landed on `i08-history-versions-checkpoints` — Phase 1
(P1+P2) = `eef3731` + ruling swap `1c5284a` + Gate A fixes `520abec`; Phase 2
(P3) = `f3d80d1`; Phase 3 (P4+P5) = this doc's commit. All gates green at each
checkpoint. The living plan and gate log stay in the Maestri note
`I08-IMPLEMENTATION-PLAN` (never committed; `docs/plans/` untouched).**

Normative sources: blueprint §5.3/§5.7/§5.8, §6.2–§6.6, §16.2/§16.3 (W07,
targets), §17.1 T02, §18.3 row I08, §2 F06/F07/F10/F73; spec
`docs/spec/canonical-profile-v1.md` §5, §8.3, §10, §11, §13, §14.
Decisions D1–D7 per the plan note; maintainer rulings recorded there
(budget/cancellation = `precondition_failed`; derived `frontiers`/`branches`
tables in the publish commit; `initial_version` from the genesis parent cut;
facade on `puyudb_core::api::Database`; Q3 global snapshot names).

Implementation: `crates/puyudb-core/src/version.rs` (frontier
decode/validate/normalize, VersionId resolve),
`history.rs` (frontiers/branches/snapshots tables, closure/ancestry,
native-head mapping, snapshots), `replay.rs` (checkpoint-first budgeted old
reads), `checkpoint.rs` (bounded hash-checked `cache/` checkpoints),
plus Store/facade wiring in `publication.rs`/`api.rs` and the native
budget/topo seams in `native.rs`. Tests: `crates/puyudb-core/tests/
history_versions.rs` (12), `checkpoints.rs` (4), `history_fidelity.rs` (6),
`w07_harness.rs` (1 gate + 3 ignored evidence runs), `history_common/mod.rs`,
and `crates/puyudb-local/tests/history_fidelity_local.rs` (I07-import fixture
as generator).

## T02 history-fidelity battery (`history_fidelity.rs`, all green)

| Invariant (T02: every retained version survives reload/checkpoint/
compaction/export; no future leak) | Test | Result |
| --- | --- | --- |
| All 6 branch-frontier versions of a create/rename/edit/fork-conflict/join/delete/edit history fingerprinted (semantic-manifest hash, text, paths, conflicts, files, native heads) — identical after reload | `t02_every_frontier_survives_reload_checkpoints_cache_discard_and_pins` | ok |
| … identical with hash-checked checkpoints cached for every pinned version | same | ok |
| … identical after `rm -rf cache/` (cache-discard invariant, D3, test-pinned) | same | ok |
| … identical read through snapshot pins after cache discard | same | ok |
| Future-leak battery: post-cut batches (edit, rename, relive) alter NO old view | `future_leak_battery_no_post_cut_batch_alters_any_old_view` | ok |
| Deleted content queryable at old versions (Deleted liveness + text + register) | `deleted_content_queryable_at_old_versions` | ok |
| Real I07-imported space (3-commit git repo): imported version survives reload, pin, cache discard byte-identically | `puyudb-local/tests/history_fidelity_local.rs` | ok |
| GC-relevant fixture: after `rm -rf cache/`, `native_heads(old)` and text are unchanged — a checkpoint never replaces immutable changes (§6.6) | `gc_relevant_fixture_checkpoint_never_replaces_immutable_changes` | ok |

Spec-negative battery (mandatory): unsorted/duplicate frontier bodies fail
`Frontier::decode`; redundant (dominated) head fails consumer validation;
over-cap (`MAX_FRONTIER_HEADS + 1`) frontier rejected before allocation; all
`invalid_encoding`. Unknown cut → `invalid_encoding`; unknown VersionId →
`content_unavailable` (VersionUnavailable, spec §5). Quarantined head →
`content_unavailable` from resolve/closure/native_heads/is_ancestor, never a
silently smaller set. Budget/cancellation → `precondition_failed`
(maintainer ruling). `invalid_encoding_negative_battery`,
`broad_dag_and_budget_cancellation`, Gate A regression tests in
`history_versions.rs` (12/12).

Broad DAG: 32 concurrent fork heads + join; closure 33/34; dominance and
budget cancellation asserted.

## Bug found and fixed during P4

`native::topo_order` ordered pending native producers by DIRECT parents only;
a change whose native base was produced by an ancestor (not a direct parent)
could be merged first → `invalid_native_scope: base version unknown`. Fixed
to Kahn over pending ANCESTORS within the closure (`ancestor_ids`); regression
covered by `history_fidelity.rs` (dependent chain where the later batch id
sorts first). ReadBudget deadline sampling moved from every-64-steps to every
step (closes the Gate A ledger item; `Instant::now` is vDSO-cheap).

## W07 harness (`w07_harness.rs`; §16.2 historical queries)

Run: `cargo test -p puyudb-core --test w07_harness -- --ignored --nocapture`
(1k/100k/1m are ignored evidence runs; the gate runs the small variant).
Metrics per run: publish wall time, warm-read p95 (hash-checked checkpoint),
cold-read p95 (`rm -rf cache/` → full replay), mid-history replay cost
(closure size), cache bytes, pinned retained versions.

Measured on the maintainer workstation (macOS ARM64, debug builds; CI
runners are regression signals only, §16.1):

| Run | n batches | publish | warm p95 | cold p95 | replay cost (mid) | cache bytes | pinned |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| gate (`w07_small_warm_cold_correctness`, runs in CI gates) | 24 | 677 ms | 275 µs | 6.0 ms | 13 | 6,333 B | 4 |
| exploratory | 120 | 52.3 s | 456 µs | 756 ms | 61 | — | 8 |

Warm reads are O(checkpoint load): 275–456 µs p95 regardless of history
length while cached; cold reads are the documented O(n) replay (§5.8, explicit
not hidden) and byte-identical to warm (asserted in the harness).

§16.3 target check: "warm file read p95 < 20 ms … after view hydration" —
warm version reads measured at 275–456 µs p95, well inside the 20 ms budget.
Cold replay at these sizes is 6 ms–756 ms p95 and grows with closure size, as
§5.8 documents.

WATCH (Gate A/B ledger, now measured): publish time is superlinear in chain
length — 677 ms for 24 batches vs 52.3 s for 120 — because
`projection::branch_heads` computes O(n²) reach pairs per call and is called
several times per publish (CAS check, `record_branch_frontier`, post-commit
checkpoint cost). The 1k/100k/1m harness runs (`w07_1k_batches`,
`w07_100k_batches`, `w07_1m_batches`, ignored, release-profile) are ready but
were NOT run to completion in this session — a 1k linear chain did not finish
within ~15 min on this workstation, consistent with the cubic publish path.
Recorded as the remaining W07 evidence gap and the top I18/W07 tuning item
(branch-head computation should become incremental, O(new ancestors + edges)
per §5.8). Separately, with an auto-checkpoint threshold below the frontier
cost, every commit past the threshold regenerates the frontier checkpoint;
the Q1 default 1024 avoids this at W07-1k scale, and regenerating only at
threshold crossings (or per pinned version) is the suggested follow-up.

Index size: one `frontiers` row per published branch frontier (32 B key +
body) plus one `branches` row; measured cache bytes above. No full text is
duplicated in SQL (§6.3).

## D-decision record

- D1 closed-frontier reads only; missing head = `content_unavailable`.
- D2 decoders validate, never normalize (producer-side normalize only in
  `version::normalize`).
- D3 caches/checkpoints disposable, hash-checked, under `cache/`, bounded;
  `rm -rf cache/` re-derives identical reads (test-pinned).
- D4 named snapshots pin name → VersionId (global names, Q3 resolved),
  CAS on take-over (`precondition_failed`).
- D5 per-read `ReadBudget` (steps/bytes/deadline); cancellation =
  `precondition_failed` (maintainer ruling, final).
- D6 read surface on `puyudb_core::api::Database` (the facade puyudb-local
  consumes); no CLI/IPC.
- D7 no Git writes (R04); I08 touches only PuyuDB state.
- Q1 default: `DEFAULT_REPLAY_COST_THRESHOLD = 1024` (replay cost = closure
  size), pinned by test, local-configurable + persisted in `format_meta`;
  §16.3 revision policy applies (W07 evidence may revise via ADR).
- Q2: checkpoint key = VersionId hex, internal/private (no wire format).
