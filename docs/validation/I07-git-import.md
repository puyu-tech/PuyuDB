# I07 validation — lossless Git baseline import (T05/T08 evidence)

**Status: I07 complete on `i07-git-baseline-import`. Phase 1 (P1+P2) =
`ce0dc3a` (+ Gate A fixes `23fee00`, `8f02011`), phase 2 (P3) = `1598255`,
phase 3 (P4+P5) = `57ff8a0` plus this doc's commit. All gates green at each
checkpoint. The living plan and gate log stay in the Maestri note
`I07-IMPLEMENTATION-PLAN` (never committed; `docs/plans/` untouched).**

Normative sources: blueprint §3.1 R04/R06, §4.3, §5.4–§5.6, §7.1–§7.3, §17.1
(T05/T08), §18.3 row I07; `docs/spec/canonical-profile-v1.md` §7, §8.1, §8.2,
§10, §11, §12.4, §14; decisions D1–D5 recorded in
[`docs/adr/04-i07-git-plumbing.md`](../adr/04-i07-git-plumbing.md).
Implementation: `crates/puyudb-local/src/git.rs` (discovery/capture/preview),
`crates/puyudb-local/src/import.rs` (execution through
`puyudb_core::api::Database` only), integration suite
`crates/puyudb-local/tests/git_import.rs`.

## What landed

- **P1+P2** — read-only system-git plumbing (D1): `discover()` (git-dir /
  common-dir / gitfile / object format sha1+sha256 / HEAD commit vs unborn),
  `capture()` (`ls-files -s -t -z`, `status --porcelain=v2 -z -uall
  --ignored=matching --no-optional-locks`, fingerprint), `BlobReader`
  (`cat-file --batch -z`, size-framed, header-verified caps), preview with
  inclusion/exclusion policy and report (no publication).
- **P3** — `import()` through the facade: space at `<toplevel>/.puyu`,
  baseline artifacts (256 KiB chunks; `text-baseline-v1` /
  `opaque-baseline-v1` / `symlink-target-v1`), create ops with §8.2 epoch
  descriptors carrying `git_oid` + mode, bounded batches
  (`IMPORT_BATCH_FILES = 256`, `IMPORT_BATCH_BYTES = 64 MiB` — Q2), D4
  idempotency keys = `raw_digest(fingerprint ‖ batch index)` with fully
  deterministic batch bodies, final import-root publication as the ready
  marker, §7.2/6 recheck (`RepoChangedDuringImport`), lazy seeds (D3),
  empty-cap allowed.
- **P4** — the fixture matrix below.

## T05 fixture matrix (real system git 2.54, `cargo test -p puyudb-local --test git_import`)

| Fixture | Test | Result |
| --- | --- | --- |
| unborn HEAD + committed HEAD | `t05_unborn_head_and_committed_import` | ok |
| sha256 object format (typed 32-byte OIDs, §8.1 tag 2) | `t05_sha256_repo_import` | ok |
| linked worktree / gitfile (`.git` is a file; git-dir ≠ common-dir) | `t05_worktree_gitfile_import` | ok |
| skip-worktree (sparse omission — included from index, flagged, not a deletion) | `t05_sparse_skip_worktree_and_gitlink_submodule` | ok |
| gitlink/submodule (mode 160000 → typed exclusion, no nested import) | same | ok |
| unmerged mixed index (stages 1/2/3, no stage 0 → Unmerged conflict metadata) | `t05_unmerged_mixed_index_import` | ok |
| non-UTF-8 path → explicit `NonUtf8Path` exclusion (Linux-only fixture; APFS rejects raw-byte names) | `t05_non_utf8_path_excluded` | ok (skips on macOS, runs on Linux CI) |
| EOL-CRLF via `.gitattributes` + config-driven `core.autocrlf` | `t05_eol_crlf_attributes_and_config_autocrlf` | ok |
| LFS pointer captured opaque (pointer bytes, never hydrated) | `t08_user_filter_and_hooks_never_executed_and_lfs_pointer_opaque` | ok |
| source unchanged (R04): HEAD, refs, physical index bytes, status, worktree hashes byte-compared before/after import | `t05_source_unchanged_by_import` | ok |
| gc cannot break history (§5.4): after `git gc --prune=now` removed the blob (`cat-file -e` fails), the baseline artifact stays `Resident` and the epoch keeps its typed `git_oid` | `t05_gc_cannot_break_history` | ok |
| crash mid-import (doc-hidden `import_partial` stops before the root): no import root, retry per D4 reuses committed batches and publishes the same root; second retry is an idempotent no-op | `t08_crash_mid_import_then_retry_per_d4` | ok |
| changed repo: re-import with the new capture fails the branch-frontier CAS (`precondition_failed`); recovery today = remove `.puyu` (I11/I14 reconcile later) — pinned | `t08_changed_repo_retry_fails_cas_recovery_is_remove_puyu` | ok |
| empty-cap import (only the import root published) | unit `smoke_empty_cap_import` | ok |

## T08 hostile fixtures

| Fixture | Test | Result |
| --- | --- | --- |
| user clean/smudge/process filters armed with marker-writing commands: never executed (every plumbing call overrides `filter.<driver>.*` with empty `-c`, which makes git's `apply_filter` spawn nothing); no marker files | `t08_user_filter_and_hooks_never_executed_and_lfs_pointer_opaque` | ok |
| `core.hooksPath` armed with a marker-writing pre-commit: never executed by read-only plumbing | same | ok |
| unsafe path names (`a;touch pwned1.txt`, `$(touch …)`, backticks, `x\|…`): imported as literal §11 paths, no shell anywhere, no `pwned*` files created | `t08_unsafe_path_names_no_shell_interpolation` | ok |
| inert symlink escaping the worktree (`../secret`): classified `SymlinkTarget`, target bytes read via `read_link`, never followed (`read_worktree_file` returns `None` for links) | `t08_inert_symlink_escaping_worktree` | ok |
| pre-existing user `.puyu/` with content: explicit `PreExistingPuyuDir`, sentinel untouched; empty `.puyu/` is created into | `t08_preexisting_puyu_dir_rejected_not_clobbered` | ok |
| §11 validation on every path (reserved `.git`/`.puyu` segments, dot components, caps) | unit tests + `path_validation_surface_is_exercised` | ok |

## Decisions forced by the fixtures (Gate B items)

1. **Recovery after `RepoChangedDuringImport`**: re-import with the new
   capture fails the CAS against the old import's batches; recovery today =
   remove `<toplevel>/.puyu` and import fresh. Pinned by
   `t08_changed_repo_retry_fails_cas_recovery_is_remove_puyu`. Reconciliation
   of a changed repo onto an existing import is I11/I14 work (§7.2/6).
2. **Pre-existing `.puyu`**: non-empty and not our space (no manifest for
   our derived space id) → `ImportError::PreExistingPuyuDir`, never created
   over or clobbered. Known blind spot, documented: user-tracked files under
   `.puyu/` stay in the ls-files fingerprint but untracked `.puyu/` status
   records are filtered (the reserved control segment is never user
   content); a tracked `.puyu` path is excluded as `InvalidPath` in the
   preview (§11 reserved segment).
3. **Conversion-profile hint after preview (frozen §8.2 has no field)**:
   decision = **documented limitation, no per-file op-5 context assertion**.
   Tracked baselines are the Git blob bytes captured verbatim; the
   conversion profile (`None`/`BuiltIn`/`Unsupported`) is reported per entry
   in the preview and config-driven conversion (`core.autocrlf`/`core.eol`)
   adds a report-level limitation. The CRLF fixtures pin that the baseline
   is the LF blob while the worktree holds CRLF. Reproducing built-in
   conversions happens only through the explicitly tested adapter later
   (§7.3); unsupported filters capture opaque and are reported (D5).
4. Vacuous assert dropped from `excluded_entries_are_never_imported`.

## Exact limits

- `MAX_PLUMBING_OUTPUT` = 64 MiB per non-streaming plumbing call; stderr
  retained 4 KiB (drained to EOF, no deadlock); `cat-file` header cap 256 B.
- Blob/worktree reads capped at `MAX_ARTIFACT_RAW_BYTES` (1 GiB, spec §10);
  classification reads capped at `MAX_TEXT_EPOCH_BYTES` (8 MiB); entries
  above the text cap classify Opaque by size; above 1 GiB → `Oversize`
  exclusion. Both reported, never fatal to the preview.
- Q2 batch constants: `IMPORT_BATCH_FILES = 256`, `IMPORT_BATCH_BYTES =
  64 MiB`; a full 256-create-op metadata body encodes < 512 KiB, well under
  `MAX_BATCH_BODY_BYTES` = 1 MiB (pinned by
  `q2_batch_plan_keeps_1000_files_under_the_body_cap`: 1000 files → 4
  batches).
- Idempotency caller/key ≤ 1024 B each (facade cap); keys are 32-byte
  `raw_digest("puyudb/import-key/v1", fingerprint ‖ batch_index)`.

## Platform notes (Windows semantics)

- The executable bit is POSIX-only. On Windows, git defaults
  `core.fileMode=false`, so even the index records 100644 for everything;
  untracked worktree files additionally have no exec bit in fs metadata.
  PuyuDB preserves the exec bit exactly where it is representable: tracked
  entries take the mode from the index (100755 works when
  `core.fileMode=true`), untracked entries stat POSIX bits on unix and fall
  back to Regular elsewhere (§7.3: exec bits are metadata; arbitrary
  permission bits are not replicated). The exec-bit test assertions are
  `#[cfg(unix)]`; on Windows they assert Regular.
- Symlink creation/stat/read (`read_link`, `symlink_metadata`) is
  `#[cfg(unix)]` in the fixtures; on Windows symlinks need privileges and are
  not exercised. Non-UTF-8 path fixture is Linux-only (APFS rejects raw-byte
  filenames; Windows filenames are WTF-16).

## Gates (repo root, all green at the phase-3 commit)

`cargo fmt --check`; `cargo clippy --workspace --all-targets --all-features
--locked -- -D warnings`; `cargo test --workspace` in all three feature
configs (`--frozen`, `--no-default-features --frozen`, `--all-features
--frozen`) — 34 suites each, `puyudb-local` 36 tests (20 unit + 16
integration); `python3 -m unittest discover -s scripts -p 'test_i0*.py'` —
67 tests OK; `cargo deny --locked check` OK. Integration tests skip silently
where no `git` binary exists; the non-UTF-8-path fixture is Linux-only
(APFS enforces UTF-8 filenames).
