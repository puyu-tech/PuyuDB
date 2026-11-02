# PuyuDB — Agent Guide (AGENTS.md)

This file is the entry point for any AI agent or contributor working in this
repository. Read it before making changes. Keep it short and accurate: if a
command or rule here is wrong, fix the file, not the exception.

## Project overview

PuyuDB is a local-first, Git-compatible versioned database engine written in
Rust. Workspace layout:

- `crates/puyudb-core` — domain codec (`canon`, `identity`, `profile`) and the
  atomic SQLite publication ledger (`publication`, `projection`, `native`)
- `crates/puyudb-crdt` — native text binding (`dt_text`) plus I02 experiment evidence
- `crates/puyudb-local`, `crates/sqlite-fault` — local facade and fault-VFS test shim
- `apps/puyu-cli` — help/version-only CLI for now
- `scripts/` — independent Python validators and their unittests
- `tests/fixtures/` — golden vectors (canonical-v1, metadata-v1)
- `docs/` — `spec/` (frozen canonical profile), `plans/` (per-issue plans),
  `validation/` (evidence), `adr/` (decisions), `IMPLEMENTATION_STATUS.md`

Normative sources for behavior are the blueprint (`../PUYUDB-BLUEPRINT.md`),
`docs/spec/canonical-profile-v1.md`, and the issue plan in `docs/plans/`.
Never change frozen profile bytes or IDs.

## Toolchain

- Rust is pinned by `rust-toolchain.toml` (currently 1.99.1) — do not upgrade
  it casually; CI builds with `--locked`/`--frozen`.
- New source files start with `// SPDX-License-Identifier: Apache-2.0`.

## Build, test, and gates

Run from the repository root. All of these must pass before every commit
(they are also the CI checks):

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --frozen
cargo test --workspace --no-default-features --frozen
cargo test --workspace --all-features --frozen
python3 -m unittest discover -s scripts -p 'test_i0*.py'
cargo deny --locked check
```

## Code style

- English only for code, comments, tests, commit messages, and docs.
- `cargo fmt` formatting is mandatory; clippy warnings are errors (`-D warnings`).
- SQL is always parameterized — never build SQL strings from data.
- Check size caps before allocating; guard every `u64`→`i64` cast.
- Keep the smallest change that fully solves the task; no speculative
  abstractions. Comment only the "why" the code cannot show.
- One small test (or assert) for any new non-trivial logic.

## Commit and PR policy

Commit messages are in English and end with a trailer block typed as plain
message text:

```text
Signed-off-by: <your own name> <your own email>
```

- `Signed-off-by` must be the identity of the account making the commit, e.g.
  `Signed-off-by: p1xelll <81173399+p1xelll@users.noreply.github.com>` for the
  maintainer. Never hardcode a shared name or email — other people's commits
  must never carry someone else's sign-off.
- Add `Co-Authored-By: <tool name> <tool email>` only if that AI tool actually
  co-authored the commit (e.g. `Co-Authored-By: Mistral Vibe <vibe@mistral.ai>`
  for Mistral Vibe; another model uses its own identity the same way).
- Do not use `git commit -s`.
- Local commits are the default. Never push, force-push, merge, or open a PR
  without explicit maintainer approval. Push only to the issue branch you are
  working on.
- `main` is protected: no force-push, required CI checks must pass, and PRs
  are required for merges.

## Documentation

- Update `docs/IMPLEMENTATION_STATUS.md` when an issue phase lands.
- Plans live in `docs/plans/`; acceptance evidence in `docs/validation/`;
  decisions in `docs/adr/`. Link them from the PR description.
- Do not edit frozen spec sections unless the issue explicitly says so.

## Security

- Never commit secrets, keys, tokens, or personal data. See `SECURITY.md` for
  reporting vulnerabilities (use the private channel listed there).
- Do not weaken validation, error handling, or durability checks to make a
  test pass.
