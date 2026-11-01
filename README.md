# PuyuDB

**Local-first history and synchronization for software development.**

PuyuDB is an independent, open-source project building a Rust-native engine for recording changes to files and the development context around them. It aims to preserve the work between Git commits and make that history accessible to applications, developers, and coding agents.

PuyuDB is the first project in the **Puyu** ecosystem. It is designed as standalone infrastructure, not as a feature tied to a particular editor, agent, or hosted service.

## Project status

**Early development — not yet released.**

This repository contains Rust workspace foundations, experimental native text/history validation, and a CLI entry point with help and version output only. It does not yet contain a working database engine, database commands, server, or SDK. The capabilities described below are project goals, not currently available features. PuyuDB is not ready for production use.

See [implementation status](docs/IMPLEMENTATION_STATUS.md) for completed work, verification evidence, and the next milestone.

## Why PuyuDB?

Git preserves the snapshots developers choose to commit. The work leading to those snapshots often includes smaller edits, discarded approaches, review comments, conversations, and agent activity that are difficult to recover later.

PuyuDB aims to retain that intermediate history without replacing Git or requiring a specific development environment.

The goal is to help answer questions such as:

- What changed between these two points in a session?
- Which person or agent contributed a change?
- What discussion or tool activity was associated with it?
- Can an earlier state be inspected or restored without losing the current one?
- Can the same history remain available across offline work and collaboration?

## Design goals

- **Local-first:** Core functionality should work offline, without an account or remote server.
- **Durable history:** Accepted changes should be recorded reliably, with explicit recovery and integrity guarantees.
- **Deterministic collaboration:** CRDT-based editing should converge while preserving conflicts that need human decisions.
- **Git compatibility:** Git should remain the conventional system for commits, branches, remotes, and publication.
- **API-first integration:** Applications should access the same capabilities as the CLI through supported interfaces.
- **Application independence:** Editors, agents, and other tools should integrate without adopting a Puyu-specific interface.
- **Secure defaults:** Recording and sharing source code or conversations should be explicit, scoped, and transparent.
- **Rust-native implementation:** The core engine, local tooling, and optional server should not require a JavaScript runtime.

Continuous filesystem recording has practical limits: an external change must be observed before it can be retained. PuyuDB will distinguish captured history from changes it could not observe, rather than promise recovery of every unsaved keystroke.

## Planned interfaces

| Interface | Purpose |
| --- | --- |
| Embedded Rust library | Use the engine directly within an application. |
| `puyu db` CLI | Manage local history, inspect changes, restore states, and control synchronization. |
| Local API | Connect applications to an optional local process without direct embedding. |
| `puyudb-server` | Synchronize and retain shared history on self-hosted infrastructure. |
| Remote API and SDKs | Integrate external applications, services, and automation. |

Rust integration comes first, followed by TypeScript and Python clients. Network synchronization and the server will remain optional for local use.

## How it fits with Git

PuyuDB is intended to complement Git, not replace it.

- **Git history** records commits and their relationships.
- **PuyuDB history** records finer-grained development activity between commits.

Developers should be able to continue using ordinary Git workflows. Recording history must not silently stage files, create commits, push branches, or rewrite Git history.

Historical restoration should create a new recorded change, preserving the state it replaces. Concurrent file, path, or binary conflicts should remain visible rather than be silently overwritten.

## Development direction

The initial focus is a small, verifiable local engine:

1. Durable change recording, historical reads, and an embedded Rust interface.
2. Git import, local file capture, restoration, and initial CLI workflows.
3. Reliable filesystem integration, recovery, backups, and integrity checks.
4. Optional replication, a self-hosted server, and authenticated collaboration.
5. Public SDKs and generic conversation and agent-session integration.

Correctness, recovery, and compatibility take priority over adding integrations. AI agents are optional consumers of PuyuDB, not a requirement for using it.

## Installation

There is no installable release yet. To build the development scaffolding, install [Rust via rustup](https://rustup.rs/), then run from the repository root:

```sh
cargo build --workspace --locked
cargo run --locked -p puyu-cli -- --help
cargo run --locked -p puyu-cli -- --version
cargo run --locked -p puyu-cli -- db --help
```

The workspace pins Rust 1.99.1 for development and as its minimum supported Rust version (MSRV), using edition 2024. The project follows reviewed, pinned latest stable releases rather than preserving older compiler support. See the [compatibility matrix](docs/COMPATIBILITY.md) for the distinction between local verification, configured CI targets, and product support.

After installing the toolchain and fetching dependencies with `cargo fetch --locked`, `cargo build --workspace --locked --offline` builds without dependency downloads. No account, server, or Puyu-hosted infrastructure is needed.

Only help and version output are available. Bare `puyu` / `puyu db`, unimplemented commands, and unknown flags return usage error code 2; they do not initialize or modify a database.

## Contributing

Contributions are welcome as the project takes shape, especially around correctness, durability, Git interoperability, security, and integration requirements.

Before starting a substantial implementation, discuss its scope and expected behavior so that changes follow a consistent design. Future contributions should include relevant tests and documentation, particularly when they affect stored history or compatibility.

Read [CONTRIBUTING.md](CONTRIBUTING.md) for development checks, DCO sign-offs, and provenance requirements, and the [code of conduct](CODE_OF_CONDUCT.md) for participation expectations. Use GitHub Private Vulnerability Reporting as described in [SECURITY.md](SECURITY.md); the channel is enabled, but there is no staffed security response service or guaranteed response time. Do not post credentials, private source code, or sensitive conversation histories in public reports.

## License

Unless otherwise noted, the original source code and documentation in this repository are licensed under the **Apache License, Version 2.0** (`Apache-2.0`).

See [LICENSE](LICENSE) for the full license text. Third-party components retain their respective licenses.
