# Contributing to PuyuDB

Puyu is the umbrella brand; PuyuDB is the engine project. The CLI is `puyu db`;
`puyudb-server` is a future optional server, not an available service.

This repository contains early scaffolding and experimental native text/history
validation, not a working database engine or a production-ready release.
Only CLI help and version output are implemented as end-user commands. Check
[implementation status](docs/IMPLEMENTATION_STATUS.md) and
[compatibility](docs/COMPATIBILITY.md) before relying on a capability or format.

## Propose a focused change

- Discuss substantial changes before implementing them. Explain the problem,
  scope, expected behavior, and alternatives.
- Keep each pull request focused. Add relevant tests and documentation, especially
  for correctness, convergence, durability, offline behavior, and compatibility.
- Distinguish implemented behavior from goals. Do not imply support for an engine,
  release, data format, or integration that does not exist.
- Follow the [code of conduct](CODE_OF_CONDUCT.md). Report security concerns using
  [SECURITY.md](SECURITY.md), not public issues containing sensitive details.
- Write code, comments, tests, and documentation in English.

## Development checks

The baseline is pinned Rust **1.99.1**, also the MSRV, and edition **2024**.
The project follows the latest stable compiler and dependency releases, with
reviewed explicit pins and lockfile updates rather than floating build inputs.
Use the repository's pinned toolchain; older Rust compilers are not supported.

From the repository root:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

To check a locked, offline build, first fetch the locked dependencies while
online, then build without network access:

```sh
cargo fetch --locked
cargo build --workspace --locked --offline
```

Report the commands and results actually obtained, including checks not run and
why. Documentation-only changes need relevant whitespace, link, and content
checks, not invented runtime test results. Consult the
[dependency policy and ledger](docs/DEPENDENCIES.md) for dependency and license
checks.

## Contribution license and provenance

Original code and documentation are licensed under [Apache-2.0](LICENSE).
Contributions are inbound = outbound: submit original contributions under
Apache-2.0, with a [DCO 1.1 sign-off](DCO.md) on each commit. There is no CLA or
copyright assignment; contributors retain their copyright.

For copied, adapted, or generated material, identify its source or generation
tool, relevant version or revision, applicable licenses, and significant
modifications in the pull request. Preserve required attribution and notices.
Review generated output yourself; generation does not establish provenance or
permission to contribute. Do not include private inputs or secrets in provenance
records.

Do not copy proprietary or AGPL-only code into this Apache-2.0 project. Verify
permission and license compatibility before introducing third-party material.
For dependencies, update the [ledger](docs/DEPENDENCIES.md) and required
[third-party notices](THIRD_PARTY_NOTICES.md); include license and distribution
obligations, not just a package name.

PuyuDB is independently developed. Do not imply affiliation, endorsement, or
unverified compatibility with another project. Licensing does not grant trademark
rights; see [TRADEMARKS.md](TRADEMARKS.md).
