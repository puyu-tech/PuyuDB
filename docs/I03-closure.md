# I03 closure

**Closed 2026-10-28 on branch `i03-identity-batch-profile`.** This document is the
T6 record. It does not freeze a storage format, a protocol, or a native encoding
beyond the already frozen [canonical profile](spec/canonical-profile-v1.md).

## What closed

- T1: ADR-01 amendment 1 and the frozen profile.
- T2: golden vectors and the independent Python validator.
- T3a: Ed25519 and fixture dependencies.
- T3b: `puyudb-crdt` depends on the patched fork, not on upstream and not on a
  vendored copy.
- T4: the `puyudb-core` codec, including the audit fixes through `72d7ed7`.
- T5 for this slice: hosted CI on the pin commit `5266c65` (below).
- This T6 record and the dependency/provenance audit.

PR #1 remains open and unmerged. No GitHub issue was opened.

## Pin provenance

| Item | Value |
| --- | --- |
| Fork | `https://github.com/puyu-tech/diamond-types` |
| Working branch | `puyudb/dt-text-v1` |
| Immovable pin branch | `puyudb/pin-0d5008b` |
| Both branches | `0d5008bc89b6a2470474711fabe3c8f8b3136e9e` |
| Upstream base | `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922` |
| `anchors.patch` | blob `8f8545480978e54a281bf863deaa343a6fd56b37`, commit `41b89d3bc61ac8af63f18bff5b222c664dbc97b6` |
| `strict.patch` | blob `bf6611ffc1162d2ca42430bdb6044336193d6f20`, commit `0d5008bc89b6a2470474711fabe3c8f8b3136e9e` |

`Cargo.lock` resolves both `diamond-types 2.0.0` and `rle 0.2.0` to
`git+https://github.com/puyu-tech/diamond-types?rev=0d5008bc89b6a2470474711fabe3c8f8b3136e9e#0d5008bc89b6a2470474711fabe3c8f8b3136e9e`.
No other lock entry uses a git source. Default features stay off.

Reapplying both patches with `git apply` onto a clean `89ae3a0` tree produced a
working tree identical to `0d5008bc`. The textual `git diff 89ae3a0 0d5008bc`
differs from the concatenated patch files only in file order and index headers.
No other source change is in the fork.

## Dependency audit

The pin added no new package versions to `Cargo.lock`. It changed two git source
URLs. These are the normal and build dependencies of that pin
(`cargo tree -p diamond-types --edges normal,build`). Crates already used by
other workspace packages are marked.

| Crate | Version | License | New to the normal graph |
| --- | --- | --- | --- |
| diamond-types | 2.0.0 | ISC | yes |
| rle | 0.2.0 | ISC OR Apache-2.0 | yes |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | yes |
| crc | 3.4.0 | MIT OR Apache-2.0 | yes |
| crc-catalog | 2.5.0 | MIT OR Apache-2.0 | yes |
| humansize | 2.1.3 | MIT/Apache-2.0 | yes |
| jumprope | 1.1.2 | ISC OR Apache-2.0 | yes |
| num_enum | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | yes |
| num_enum_derive | 0.7.6 | BSD-3-Clause OR MIT OR Apache-2.0 | yes |
| smallvec | 2.0.0-beta.3 | MIT OR Apache-2.0 | yes |
| smartstring | 1.0.1 | MPL-2.0+ | yes |
| str_indices | 0.4.4 | MIT OR Apache-2.0 | yes |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | yes, via jumprope |
| rand | 0.8.8 | MIT OR Apache-2.0 | yes, via jumprope |
| rand_chacha | 0.3.1 | MIT OR Apache-2.0 | yes, via jumprope |
| rand_core | 0.6.4 | MIT OR Apache-2.0 | yes, via jumprope |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 | yes, via jumprope |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | yes |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | yes |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | yes |
| libc | 0.2.191 | MIT OR Apache-2.0 | already present |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | already present |
| libm | 0.2.16 | MIT | already present |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | build |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | build |
| quote | 1.0.47 | MIT OR Apache-2.0 | build |
| syn | 2.0.119 | MIT OR Apache-2.0 | build |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | build |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | build |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | build |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | build |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 | build |
| toml_edit | 0.25.17+spec-1.1.0 | MIT OR Apache-2.0 | build |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | build |
| version_check | 0.9.5 | MIT/Apache-2.0 | build |
| winnow | 1.0.4 | MIT | build |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | build |

`cargo-deny` accepts every license in that tree. `num_enum` and `zerocopy` match
through MIT or Apache-2.0, not through a new license allow. `smartstring` keeps
its existing MPL exception.

`RUSTSEC-2026-0249` is an unmaintained-crate advisory, not a vulnerability. The
v1 ignore is acceptable because the fork uses `smartstring` only as a string
type. Drop it when the fork no longer pulls `smartstring`, or if the advisory
starts reporting a vulnerability or is withdrawn. The `rand 0.8.8` /
`rand_core 0.6.4` / `getrandom 0.2.17` skips are duplicate-version skips for
`jumprope`, not vulnerability waivers.

Upstream still ships no root LICENSE/NOTICE file. A distributed binary that
contains this fork must carry the ISC notice and this provenance. That
distribution audit is not done here.

## Hosted CI on the pin commit

Both runs of `5266c65b11ea72234f6ba150fa4fa879542bf110` completed with success
before this closure was written:

- push: `https://github.com/puyu-tech/PuyuDB/actions/runs/40783464094`
- pull_request: `https://github.com/puyu-tech/PuyuDB/actions/runs/40783471199`

Each run passed Formatting and Clippy, Dependency policy, and Rust 1.99.1 tests
on ubuntu-24.04, ubuntu-24.04-arm, macos-15, macos-15-intel, and windows-2025.
WSL2 is not in that matrix. This closure commit is a later commit; its hosted
CI is not claimed.

## Accepted v1 limits

- The profile is a document plus a codec. There is no storage, publication,
  protocol, daemon, or CLI behavior.
- Native DT bytes are not a canonical encoding. A replica must not re-encode a
  history and claim the producer's artifact ID.
- One tested deleted-gap cursor still depends on import order. No replacement
  merger and no actor-order override is authorized.
- `relation_tag` on metadata operations 6 and 9 is opaque in v1.
- `validate_record` is not admission. `verify_batch` is the function that binds
  the signature key to the body key.
- The fork is unpublished upstream plus two local patches. Rebase and notice
  obligations remain.
- GitHub-hosted Linux is not a substitute for a later independent native Linux
  performance or support claim.
