# Changelog

No releases have been published.

## Unreleased

- I01: four-package Rust workspace, shared Apache-2.0 metadata/lints, a pinned
  development toolchain, declared MSRV, and workspace dependency lockfile.
- Help/version-only `puyu` CLI with the `db` namespace and process-level tests.
- Initial CI, dependency policy, contributor/security/provenance policies, and
  implementation/compatibility tracking.
- Confirmed GitHub Private Vulnerability Reporting enabled, closing I01.
- Follow the latest stable toolchain policy: Rust 1.99.1 is now both the pinned
  compiler and minimum; the former 1.85 floor is no longer supported.
- I02 in progress: pinned native validation experiments, byte/hash seed fixtures,
  history/convergence/Unicode/scope tests, executable Automerge/Yrs/Loro
  comparisons, and native-only W01/W02 workloads. No native format is frozen.
- I02 follow-up: safe no-LZ4 upstream DT comparison pin, larger mixed-observation
  delivery/reload corpus with an explicit long release test, expanded historical
  anchors and native-wire negatives, attributed adversarial diagnostics,
  calibrated allocator measurements, and a proposed ADR evidence addendum.
- Optional, tested source-transfer and baseline-collection helpers for native
  Windows and Debian WSL2; no automatic upload or I02 acceptance.
- Executed native Windows, Debian WSL2 and refreshed macOS validation/resource
  baselines; fixed canonical `puyu` help naming exposed by Windows snapshots.
- Maintainer-approved provisional WSL2 scope for I02, retained WSL2 W01/allocation
  samples and a descriptive native-cost assessment. Independent reference Linux
  is deferred; native-semantic/cost acceptance still gates I02 closure and I03.
- Common four-candidate W01/W02 runner with separate translation/native/text and
  allocator stages, 11 saved cuts, fresh-process seed artifacts, resource/failure
  capture and retained comparative evidence from WSL2/macOS. Alternatives remain
  under evaluation; no engine selection, production binding or format freeze.

- I03: frozen canonical profile v1 (spec plus golden vectors) and the
  `puyudb-core` codec reproducing it byte-for-byte; diamond-types pinned to the
  patched puyu-tech fork. No storage or native format acceptance yet.
- I04 in progress on `i04-atomic-sqlite-publication`: atomic SQLite publication
  ledger (rusqlite 0.41.1 bundled SQLite 3.54.1, WAL/FULL, owner lock) with
  manifest + `format_meta`, artifact layer with 256 KiB chunks and hydration
  tracking, the `batch_parents` DAG with closure/minimal-frontier admission,
  idempotent keyed publication, replication staging schema, and a
  fault-injection VFS covering lost sync, torn write, disk-full and a
  WAL-reset regression. Still no projections, branches, native scope,
  checkout, replication, or CLI surface.
- I05 phases 1-2 in progress on `i05-native-text-projections`: metadata
  artifact admission with MV-register storage (`field_writes`, write identity
  `(batch_id, op_index)`, causal supersession, arrival-order-independent
  maximal alternatives), file projections at a cut (`files_at`/`paths_at`/
  `prefix_conflicts_at`/`branch_heads`; `files_current`/`paths_current` stay
  read-time rebuildable accelerations until I08/I12), liveness with no
  implicit resurrection, collision and prefix-conflict sets reported but
  never auto-resolved, conflict derivation (kinds 0/1/2 with stable
  `ConflictId`s, recomputed from history, never persisted), tag-8 resolution
  admission, and the canonical `SemanticManifestV1` at a cut, golden-pinned
  and identical across two independent replicas. Native admission with the
  §12.5 scope checks (`invalid_native_scope`), epoch-owned seeds, the
  `native_changes` table, and native text reads at a cut (`text_at`) have since
  landed as phase 3 — `publish` no longer refuses native artifacts. See
  `docs/validation/I05-phase2-projections.md`.

There is no working database engine, persisted format, server, or SDK.
