# ADR-04 record for I07: read-only system-git plumbing, no `puyudb-git` crate

**Status: implemented on branch `i07-git-baseline-import` (I07 phases 1–3,
commits `ce0dc3a`, `23fee00`, `8f02011`, `1598255`, phase-3 commit).** The
issue plan note calls this record "ADR-03"; the decision index reserves
ADR-03 for immutable-history-vs-caches, and ADR-04 ("System Git mutations;
tested read-only library plumbing — I07/I14 real-Git fixtures") is the slot
this record fills.

I07 fixed decisions D1 and D2, as landed:

- **D1 — Git access is read-only system-git plumbing via `std::process`.**
  Commands: `rev-parse` (`--git-dir`, `--git-common-dir`,
  `--show-object-format`, `--show-toplevel`, `--verify -q HEAD`),
  `symbolic-ref -q HEAD`, `ls-files -s -t -z`, `status --porcelain=v2 -z
  -uall --ignored=matching --no-optional-locks`, `cat-file --batch(-check)
  -z`, `check-ignore -z --no-index --stdin`, `check-attr -z --stdin`,
  `config --get` / `--get-regexp` (read-only). Rules: NUL-delimited output
  only; `--end-of-options` where the command accepts it; untrusted paths
  only via NUL-delimited stdin, never as CLI arguments; typed output
  verification (OID length/charset per the object format, record shapes,
  echoed-OID equality); size caps on every read (64 MiB plumbing output,
  profile artifact cap on blobs, text-epoch cap on classification reads);
  no shell anywhere. Every configured `filter.<driver>.*` is overridden
  with an empty `-c` for our invocations, which makes git's `apply_filter`
  spawn nothing (D5: filters/hooks/textconv never execute — pinned by the
  T08 fixture where a hostile clean filter would create a marker file).
  A missing `git` binary is an environmental error, never a §14 code.
  `gix` remains the documented fallback and would require a new ADR.
  No new crate dependency was added (R10): `puyudb-local` depends only on
  the path dependency `puyudb-core`.
- **D2 — crate placement.** New modules live in `crates/puyudb-local`
  (`src/git.rs` discovery/capture/preview, `src/import.rs` execution).
  No `puyudb-git` crate is extracted (§4.3 "if extracted" — extraction needs
  its own justification/gate). Import execution reaches the engine only
  through `puyudb_core::api::Database` (`create_space`/`open_space`/
  `stage_chunk`/`stage_artifact`/`transact`/read APIs); the typed values
  (`GitOid`, `Path`, `ModeTag`, limits) come from `puyudb-core`'s public
  `identity`/`profile` modules. No dependency points from core toward
  local.

Evidence: `docs/validation/I07-git-import.md` (T05/T08 fixture matrix,
source-unchanged, gc-retention, crash/retry), `cargo test -p puyudb-local`
(unit + `tests/git_import.rs` integration suite).
