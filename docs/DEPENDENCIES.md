# Dependency policy and ledger

## Locked application graph

`Cargo.lock` is the source of exact registry versions and checksums. Released direct
pins were checked against latest stable versions during I02. diamond-types is
pinned to an exact revision of the puyu-tech fork, not a moving branch.
Rust 1.99.1 is the current compiler/floor. Exact pins make selections reproducible;
they do not mean Cargo fetches moving latest versions on every build.

The resolved metadata graph contains **227 registry package versions and two
same-revision Git packages**, including development, build, and target-specific
dependencies. The ledger below records
direct selections and license/maintenance-sensitive transitive dependencies.
`cargo metadata --locked --format-version 1` provides the complete per-package
version, license, source, and declared Rust-version inventory.

Engine selection is closed (2026-10-26): diamond-types is the selected native
text engine with the small reviewed patch set; Loro, Yrs and Automerge remain
dev-only comparison/conformance dependencies. See
[ADR-01](adr/01-native-text-evidence.md).

| Crate | Locked version | Relationship / purpose | Upstream license | Declared MSRV |
| --- | --- | --- | --- | --- |
| [clap](https://crates.io/crates/clap/4.6.7) | 4.6.7 | Direct; CLI argument parsing and help | MIT OR Apache-2.0 | 1.85 |
| [clap_builder](https://crates.io/crates/clap_builder/4.6.7) | 4.6.7 | Transitive; command definitions and rendering | MIT OR Apache-2.0 | 1.85 |
| [clap_lex](https://crates.io/crates/clap_lex/1.1.1) | 1.1.1 | Transitive; argument lexing | MIT OR Apache-2.0 | 1.85 |
| [anstyle](https://crates.io/crates/anstyle/1.0.14) | 1.0.14 | Transitive; help style representation, even without colored output | MIT OR Apache-2.0 | 1.66.0 |
| [automerge](https://crates.io/crates/automerge/0.12.0) | 0.12.0 | Direct in CRDT experiment; conformance reference, not selected | MIT | 1.90.0 |
| [sha2](https://crates.io/crates/sha2/0.11.1) | 0.11.1 | Direct; specified actor namespace derivation | MIT OR Apache-2.0 | 1.88 |
| [ed25519-dalek](https://crates.io/crates/ed25519-dalek/3.0.0) | 3.0.0 | Direct in core; strict Ed25519 verification and deterministic fixture signing for canonical records | BSD-3-Clause | 1.85 |
| [curve25519-dalek](https://crates.io/crates/curve25519-dalek/5.0.0) | 5.0.0 | Transitive of ed25519-dalek; Ed25519 group arithmetic | BSD-3-Clause | 1.85.0 |
| [subtle](https://crates.io/crates/subtle/2.6.1) | 2.6.1 | Transitive of curve25519-dalek; constant-time primitives | BSD-3-Clause | Undeclared |
| [ed25519](https://crates.io/crates/ed25519/3.0.0) | 3.0.0 | Transitive of ed25519-dalek; key and signature types | Apache-2.0 OR MIT | 1.85 |
| [signature](https://crates.io/crates/signature/3.0.0) | 3.0.0 | Transitive of ed25519-dalek; signing/verification traits | Apache-2.0 OR MIT | 1.85 |
| [fiat-crypto](https://crates.io/crates/fiat-crypto/0.3.0) | 0.3.0 | Transitive of curve25519-dalek; generated field arithmetic | MIT OR Apache-2.0 OR BSD-1-Clause | 1.83.0 |
| [curve25519-dalek-derive](https://crates.io/crates/curve25519-dalek-derive/0.1.1) | 0.1.1 | Build/proc-macro transitive; selects field arithmetic backends | MIT/Apache-2.0 | Undeclared |
| [serde_json](https://crates.io/crates/serde_json/1.0.154) | 1.0.154 | Dev only in core; reads the committed canonical-profile fixtures | MIT OR Apache-2.0 | 1.71 |
| [thiserror](https://crates.io/crates/thiserror/2.0.21) | 2.0.21 | Direct; experimental error types | MIT OR Apache-2.0 | 1.77 |
| [proptest](https://crates.io/crates/proptest/1.12.0) | 1.12.0 | Dev only; deterministic generated native histories | MIT OR Apache-2.0 | 1.88 |
| [yrs](https://crates.io/crates/yrs/0.28.0) | 0.28.0 | Dev only; challenger trace, not selected | MIT | Undeclared |
| [loro](https://crates.io/crates/loro/1.17.1) | 1.17.1 | Dev only; challenger trace, not selected 2026-10-26 | MIT | Undeclared |
| [diamond-types fork](https://github.com/puyu-tech/diamond-types/tree/0d5008bc89b6a2470474711fabe3c8f8b3136e9e) | Git manifest 2.0.0, not a published stable release | Direct in `puyudb-crdt`; selected engine; no-LZ4; `puyudb/dt-text-v1` and immovable `puyudb/pin-0d5008b` both at `0d5008bc89b6a2470474711fabe3c8f8b3136e9e` | ISC | 1.96.0 |
| rle | Git manifest 0.2.0, same fork revision | DT transitive path dependency | ISC OR Apache-2.0 | See upstream manifest |
| [rand](https://crates.io/crates/rand/0.10.3) | 0.10.3 | Dev only; pinned seeded schedule generation | MIT OR Apache-2.0 | 1.85 |
| [stats_alloc](https://crates.io/crates/stats_alloc/0.1.10) | 0.1.10 | Dev only; measurement allocator and calibration | MIT | Undeclared |
| [smartstring](https://crates.io/crates/smartstring/1.0.1) | 1.0.1 | Production transitive of the pinned DT fork; MPL plus unmaintained advisory RUSTSEC-2026-0249 | MPL-2.0+ | 1.57 |
| [unicode-ident](https://crates.io/crates/unicode-ident/1.0.27) | 1.0.27 | Build/proc-macro transitive; Unicode data license required as well | (MIT OR Apache-2.0) AND Unicode-3.0 | 1.71 |
| [zlib-rs](https://crates.io/crates/zlib-rs/0.6.8) | 0.6.8 | Automerge compression transitive | Zlib | 1.75 |
| [bitmaps](https://crates.io/crates/bitmaps/2.1.0) | 2.1.0 | Loro dev transitive; scoped license and maintenance exception | MPL-2.0+ | Undeclared |
| [im](https://crates.io/crates/im/15.1.0) | 15.1.0 | Loro dev transitive; scoped license and maintenance exception | MPL-2.0+ | 1.46.0 |
| [sized-chunks](https://crates.io/crates/sized-chunks/0.6.5) | 0.6.5 | Loro dev transitive; scoped license and maintenance exception | MPL-2.0+ | Undeclared |
| [xxhash-rust](https://crates.io/crates/xxhash-rust/0.8.19) | 0.8.19 | Loro dev transitive; Boost license exception | BSL-1.0 | Undeclared |
| [lz4_flex](https://crates.io/crates/lz4_flex/0.11.6) | 0.11.6 | Loro dev transitive; patched resolution, not 0.11.5 | MIT | 1.81 |

Promoting the fork onto the normal `puyudb-crdt` graph makes `rand 0.8.8`,
`rand_core 0.6.4` and `getrandom 0.2.17` duplicate the newer versions already
pulled by Automerge and the workspace `rand 0.10.3` pin. Those older versions
come from diamond-types' `jumprope` dependency and were already locked. `deny.toml`
skips them rather than forking `jumprope`. They are not vulnerability waivers.

`RUSTSEC-2026-0249` reports that `smartstring 1.0.1` is unmaintained. It is not a
vulnerability advisory, and the ignore is not a vulnerability waiver. It is
acceptable for v1 because the pinned fork uses it only as a string type inside
diamond-types, the MPL notice obligation is already recorded, and replacing it
would require a further fork of diamond-types or jumprope. Drop the ignore when
the pinned fork no longer pulls `smartstring`, or when the advisory is withdrawn
or changed to report a vulnerability.

The Cargo pin is the full revision, not a branch name. Branch
`puyudb/pin-0d5008b` in `puyu-tech/diamond-types` also points at
`0d5008bc89b6a2470474711fabe3c8f8b3136e9e`, so a force-push of `puyudb/dt-text-v1`
cannot move the pinned object.

The only BSD-3-Clause components in the graph are the Ed25519 binding and its
arithmetic/constant-time transitives listed above; the license is allowed
globally in `deny.toml` with a notice obligation recorded in
[third-party notices](../THIRD_PARTY_NOTICES.md). `serde_json` was already present
as a transitive development dependency before it became a direct one.

All other packages resolve from crates.io. The sole Git source is allowlisted by
repository; both direct full revision and expected manifest version are constrained.
The CLI retains only
`std`, `help`, `usage`, and `error-context`; Automerge/Loro/Yrs default features
are disabled. Proptest enables `std` without fork/timeout infrastructure.
DT defaults (LZ4/storage) are disabled. Core/local remain standard-library-only;
the CLI does not link the native engines.

For dual MIT/Apache code PuyuDB selects Apache-2.0 where offered; the extra
Unicode-3.0 requirement remains mandatory. License texts were inspected for the
MPL/Boost/Unicode/zlib additions. Preserve their notices and any source-availability
obligations before distributing binaries or vendored sources; see
[third-party notices](../THIRD_PARTY_NOTICES.md). No dependency source was copied,
modified, or vendored. This is not a completed distribution/legal audit.

Latest direct releases do not imply unconstrained transitive upgrades: Automerge
0.12.0 requires `hexane 1.0.0-alpha.5`; Loro's `pest 2.9.4` transitive requires
exactly `psm 0.1.24` even though 0.1.32 exists. `cargo update --dry-run --verbose`
reported zero available compatible lockfile changes. Other multiple major versions
satisfy different upstream contracts. Do not silently replace them with incompatible
releases or forks.

`diamond-types 1.0.0` was investigated but **removed from the resolved graph**:
without default LZ4 it fails to compile; its default `lz4_flex 0.9.x` requirement
cannot resolve a fix for
[RUSTSEC-2026-0041](https://rustsec.org/advisories/RUSTSEC-2026-0041.html).
No vulnerability exception or speculative upstream patch was added.
The current upstream no-LZ4 revision fixes the build path and its comparison now
passes. Its source manifest is 2.0.0, not a crates.io release; its archive has no
root LICENSE/NOTICE text despite declaring ISC. Record that gap before any
distribution. See [exact provenance](validation/I02-sources.md).

SQLite, Git libraries, application transport, and signing are not dependencies.
Runtime/native format acceptance remains separate from pinning research code.

## Automated checks

Use **cargo-deny 0.20.2** as development tooling, not a runtime dependency:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --locked check
```

`deny.toml` checks RustSec advisories, licenses **including dev dependencies**,
duplicate versions, wildcard dependencies, and registry/Git sources. Global
licenses are Apache-2.0/MIT/Unicode-3.0/Zlib. Exact package/version exceptions
permit the three MPL Loro transitives, MPL smartstring, ISC diamond-types, and
Boost-licensed xxhash-rust; MPL/ISC are not globally allowed. Cargo-deny uses
`MPL-2.0` as the accepted licensee spelling for
the upstream `MPL-2.0+` declarations.

Six explicit **unmaintained-only** advisory exceptions allow research comparisons:

| Advisory | Package | Why present |
| --- | --- | --- |
| RUSTSEC-2023-0089 | atomic-polyfill 1.0.3 | Loro dev graph |
| RUSTSEC-2026-0247 | bitmaps 2.1.0 | Loro dev graph |
| RUSTSEC-2026-0248 | im 15.1.0 | Loro dev graph |
| RUSTSEC-2026-0251 | sized-chunks 0.6.5 | Loro dev graph |
| RUSTSEC-2026-0215 | smallstr 0.3.1 | Yrs dev graph |
| RUSTSEC-2026-0249 | smartstring 1.0.1 | Pinned DT dev graph |

These are maintenance risks, not claims of safe production adoption. Reassess
before promoting a challenger or distributing comparison binaries. No known
vulnerability is ignored. The one duplicate-version exception permits syn
2.0.119 beside syn 3 because current upstream macro packages require both.

Both PR CI and a weekly advisory workflow run the policy. Advisory freshness
requires public RustSec access; none of this tooling is an offline runtime
requirement.

CI action revisions are pinned by full commit ID. Cargo-deny installation is
version-pinned and uses the install action's default checksum verification.
CI tools, GitHub Actions, Rust toolchains, and their own dependencies are build
infrastructure, not entries in the application lockfile. Their pins still need
maintenance and review.

## Change policy

For each dependency addition or update:

1. Establish a concrete need; prefer existing code or the standard library.
2. Review upstream source/license/notices, maintenance and security status,
   enabled features, transitive graph, native build code, and MSRV.
3. Update the lockfile, this ledger, and notices. Do not fetch production code
   from moving Git branches or change conflict semantics through Cargo features.
4. Run checks on the pinned compiler and MSRV, including locked offline builds.
5. Once formats exist, retain old raw-byte fixtures and prove compatibility before
   upgrading native formats or replacing a dependency.

Advisory checks cannot prove absence of vulnerabilities or format correctness.
SBOM generation, vendored source archives, reproducible binaries, signing,
distribution notices, and independent security review are release gates, not
implemented I01 features.
