# Compatibility and development targets

There are no releases, supported database formats, stable Rust APIs, or
production-supported platforms yet. Package version `0.1.0-dev` identifies
unreleased scaffolding, not a usable database.

## Rust baseline

- Edition: **2024**.
- MSRV and development/lint toolchain: **1.99.1**, pinned in
  `rust-toolchain.toml`. The official stable Rust manifest reported 1.99.1
  (2026-10-15); it was installed and exercised locally.
- By explicit maintainer direction, use the latest stable compiler and reviewed
  latest stable direct dependency releases, retaining exact pins and lockfiles.
  The former I01 1.85.0 floor is historical evidence, not current support.
- Upstream transitive constraints remain real: a stable dependency can itself
  require an older or prerelease internal package. Do not bypass that contract
  with an unreviewed fork; record it in the dependency ledger.
- The root manifest owns shared package metadata and lints. Every member inherits
  them. All packages have `publish = false`; publication requires a separate review.
- `Cargo.lock` belongs in source control. CI uses locked/frozen resolution.
  Compiler and dependency upgrades require fixture compatibility checks before
  adopting new semantics.

To reproduce the MSRV checks after installing it with rustup:

```sh
rustup toolchain install 1.99.1 --profile minimal
cargo +1.99.1 build --workspace --all-targets --frozen
cargo +1.99.1 test --workspace --frozen
```

`--frozen` combines locked resolution and offline dependency access. Install the
toolchain and run `cargo fetch --locked` online first.

## OS matrix

| Environment | Evidence / configuration | Support meaning |
| --- | --- | --- |
| macOS ARM64 (`aarch64-apple-darwin`) | Native experiment tests on 1.99.1; I01 foundation also ran on 1.85.0/1.99.0 | Local research evidence, not product support |
| Ubuntu 24.04 x86-64 (`ubuntu-24.04`) | CI configured on 1.99.1; not run locally | Candidate development target |
| Ubuntu 24.04 ARM64 (`ubuntu-24.04-arm`) | CI configured on 1.99.1; not run locally | Candidate development target |
| macOS ARM64 (`macos-15`) | CI configured on 1.99.1; not run here | Candidate development target |
| macOS x86-64 (`macos-15-intel`) | CI configured on 1.99.1; not run locally | Candidate development target |
| Windows x86-64 (`windows-2025`) | CI configured on 1.99.1; not run locally | Early portability check, not product support |

Runner labels and architectures were checked against the upstream
[GitHub runner inventory](https://github.com/actions/runner-images#available-images).
The CI matrix runs native process tests, an all-target build, and
default/no-default/all-feature tests. The workspace currently has no project
feature flags; these configurations intentionally coincide until real
capabilities are introduced. Local passing checks do not establish a passing
GitHub Actions run. Snapshot files enforce LF through `.gitattributes` so Git
checkout line-ending settings do not change expected CLI bytes.

Filesystem locking, durable flushes, paths, watchers, IPC, WSL, network
filesystems, and installation/release behavior have **not** been validated.
Native product support and distribution remain gated by I36.

## Compatibility boundaries

There is no persisted PuyuDB history to migrate. I02's raw native fixtures are
experimental upstream-format evidence, not a supported PuyuDB data format.
Native CRDT, canonical history, physical storage, replication, REST, and export
versions remain separate from the Cargo package version. No format is frozen.

Before persisted alphas, changes must document readable/writable format versions,
backups, explicit migrations, and retained compatibility fixtures. CLI help text is
for humans, not a machine-readable database contract. JSON output and stable
database-command exit contracts begin with their implementing milestones.
