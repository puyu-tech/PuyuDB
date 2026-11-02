# I05 phase 2 validation — file projections, conflict derivation, semantic manifest

**Status: phase 2 (plan P3–P5) landed as commit `dfd5a73` on
`i05-native-text-projections`; all gates green at that commit. Phase 3
(plan P6–P8, native text admission/reads) has not started: `publish` still
refuses native artifacts (`NativeUnchecked`).**

Normative sources: `docs/spec/canonical-profile-v1.md` §1.1 (canonical CBOR
subset), §6.3, §7/§7.1 (metadata artifact and operation registry), §7.2
(semantic manifest), §9 (conflicts), §11 (paths);
`docs/plans/I05-native-text-projections.md`. Implementation:
`crates/puyudb-core/src/projection.rs` and `crates/puyudb-core/src/profile.rs`;
admission and tests: `crates/puyudb-core/src/publication.rs`.

## What landed (commit `dfd5a73`)

`crates/puyudb-core/src/projection.rs` (+620), `crates/puyudb-core/src/publication.rs`
(+475), `crates/puyudb-core/src/profile.rs`, `docs/IMPLEMENTATION_STATUS.md`.

- **P3 file projections at a cut** (all in `projection.rs`):
  - `files_at` (projection.rs:850) — per-file maximal register alternatives
    (`live` / `paths` / `epochs` / `contents` / `modes`), entries sorted by raw
    `FileId` bytes, inner arrays sorted and deduplicated.
  - `paths_at` (projection.rs:900) — path → sorted collision set of currently
    alive files. Collision sets are never auto-resolved (§7.1: two files with
    one path are a collision set, not a resolution).
  - `prefix_conflicts_at` (projection.rs:927) — file/directory prefix pairs
    (`a` vs `a/b`), reported, never auto-resolved or renamed.
  - `branch_heads` (projection.rs:953) — branch tips computed on read.
  - Liveness has no implicit resurrection (§7.1 tag 2): a deleted file keeps
    its semantic-manifest entry and content but leaves the current path view.
- **P4 conflict derivation** — recomputed from history at a cut, never
  persisted as arrival-dependent flags (`conflicts_at`, projection.rs:988;
  `Conflict`/`ConflictId` at profile.rs:925/:970; spec §9):
  - kind 0 register alternatives, kind 1 `delete_edit` (maximal deletion vs a
    causally concurrent edit-producing write whose base cut saw the file
    alive; `edit_base_saw_alive` projection.rs:1128), kind 2 `epoch_edit`
    (`ConflictKind` profile.rs:568).
  - Stable `ConflictId`s: participants sorted ascending by
    `(batch32, operation_index)`, epochs sorted unique, so the ID is
    independent of delivery order (spec §9).
  - Tag-8 resolution admission (`validate_resolutions`, projection.rs:418): the
    named conflict must be visible at the parent cut, the parents must observe
    every named participant, and the same batch must publish the fresh
    corresponding field assignment (spec §9 resolution table; acceptance-only
    records are insufficient). The resolution's write then supersedes the
    alternatives through normal causal supersession.
- **P5 semantic manifest** — `semantic_manifest_at` (projection.rs:1151) builds
  the canonical `SemanticManifestV1` at a cut (spec §7.2; `FileEntry`,
  `SemanticManifest`, `SemanticManifestId` at profile.rs:1039/:1109/:1144,
  encoded in the §1.1 canonical CBOR subset). Golden-pinned
  `SemanticManifestId`
  `179f197962d2e51b23521c193c2bd81b4b3d45a826dbd80b080a5e68229766fb`
  (publication.rs:3528), identical across two independent replicas in both
  arrival orders.

## Gate results (run at `dfd5a73`)

- `cargo fmt --check` — clean.
- `cargo clippy --workspace -- -D warnings` — clean.
- `cargo test --workspace` — 29 suites, 0 failed; `puyudb-core` lib 70 passed.
- `python3 -m unittest discover -s scripts -p 'test_i0*.py'` — 44 tests, OK.
- `cargo deny check` — clean.

## Evidence tests (`crates/puyudb-core/src/publication.rs`)

Phase 2:

- `paths_at_reports_collision_sets_and_prefix_conflicts` (:3183)
- `deletion_hides_current_path_but_never_implicitly_resurrects` (:3235)
- `branch_heads_track_the_branch_tip` (:3269)
- `register_conflicts_have_stable_ids_across_arrival_orders` (:3286)
- `delete_edit_conflict_is_derived_from_concurrent_delete_and_edit` (:3319)
- `tag8_resolution_publishes_fresh_assignment_and_supersedes_alternatives` (:3369)
- `tag8_resolution_negatives_are_rejected` (:3412)
- `semantic_manifest_id_is_identical_across_two_independent_replicas` (:3489;
  golden pin at :3528)

Phase 1 carry-over that phase 2 builds on:

- `metadata_create_is_stored_as_register_writes_with_implied_alive` (:2727)
- `register_is_independent_of_arrival_order` (:2836)
- `quarantined_batch_writes_are_not_visible_in_the_register` (:2990)

## Design decisions

- `files_current`/`paths_current` branch views are computed on read from
  `branch_heads` (projection.rs:953) and remain rebuildable accelerations per
  §6.3 until a frontier table lands in I08/I12; they are never authoritative
  state.
- Create implies `Alive` at the same write identity: `record_batch`
  (projection.rs:233) stores the initial liveness assignment with the create
  operation because liveness is a register and a created file is alive; later
  content edits never implicitly resurrect a deleted file.
- Conflicts are recomputed from history at every cut (§9), never persisted.
  Resolution admission derives them at the parent cut only when the batch
  carries a tag-8 operation, so the common publish path pays no
  conflict-derivation cost (`validate_against_history`, projection.rs:320).
- Edit-producing operations for `delete_edit`/`epoch_edit` derivation are
  metadata tags 3 (epoch select/reset) and 4 (opaque assignment)
  (`conflicts_impl`, projection.rs:1043).

## Known limitations

- Register evaluation is O(closure × writes) per read; indexed causal dominance
  is deferred per blueprint §5.8 and deliberately not built (projection.rs
  module docs). Closure walks run inside SQL recursive CTEs with seed lists
  chunked at `SEED_CHUNK` = 16,384 (projection.rs:689), well under the bundled
  SQLite's 32,766 bind-variable limit; the ancestor closure itself is never
  turned into bind parameters.
- `epoch_edit` (kind 2) is derived from opaque assignments (tag 4) only;
  native text edits do not exist yet and land in phase 3 (P6–P8).
- A quarantine inside an otherwise visible closure cuts causality at that
  node: the quarantined batch's exclusive ancestors become invisible below any
  descendant cut, while writes reachable through another visible head still
  surface as concurrent alternatives. This semantic is deliberate
  (quarantine = not visible history), documented in the projection.rs module
  docs (:24) and pinned by
  `quarantined_batch_writes_are_not_visible_in_the_register`
  (publication.rs:2990).
