# Third-party notices

Third-party components remain the property of their respective copyright holders
and retain their own licenses. The project's Apache-2.0 license does not relicense
those components.

No third-party source code is currently vendored in this repository. Dependencies
can still be included in built binaries. See the
[dependency policy and ledger](docs/DEPENDENCIES.md) for exact dependency versions,
licenses, sources, and notice obligations.

Before distributing any dependency-containing binary, identify its included
components and include the required license texts, copyright notices, and other
attributions with the distribution. Preserve upstream notices and satisfy any
additional applicable distribution obligations. Update the ledger and this file
when adding or changing third-party material.

This file is not a completed legal audit or a claim that every distribution has
met its obligations. Dependency license texts are not reproduced here at this
scaffolding stage; required texts and notices must be assembled before distributing
binaries.

## I03 canonical-profile crypto dependency

The canonical profile binds Ed25519 signatures, so `puyudb-core` links
`ed25519-dalek 3.0.0`. It and its transitives `curve25519-dalek 5.0.0` and
`subtle 2.6.1` are **BSD-3-Clause**. No BSD source is vendored in this
repository.

BSD-3-Clause obliges a distributor of compiled binaries to reproduce the
upstream copyright notice, the license conditions and the disclaimer with the
distribution. Assemble those texts from the crates.io packages of the exact
locked versions before distributing any binary that contains them; this file is
not a substitute for them. The remaining transitive components of that binding
(`ed25519 3.0.0`, `signature 3.0.0`, `fiat-crypto 0.3.0`,
`curve25519-dalek-derive 0.1.1`) are Apache-2.0/MIT.

## I03 T3b diamond-types fork

`puyudb-crdt` links diamond-types from the patched fork
`https://github.com/puyu-tech/diamond-types`, branch `puyudb/dt-text-v1`,
revision `0d5008bc89b6a2470474711fabe3c8f8b3136e9e`. Upstream is ISC. The fork
applies two reviewed patches on upstream `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`:
`anchors.patch` blob `8f8545480978e54a281bf863deaa343a6fd56b37` as commit
`41b89d3bc61ac8af63f18bff5b222c664dbc97b6`, and `strict.patch` blob
`bf6611ffc1162d2ca42430bdb6044336193d6f20` as commit
`0d5008bc89b6a2470474711fabe3c8f8b3136e9e`. No diamond-types source is vendored
here. The upstream archive still has no root LICENSE/NOTICE text; distributing a
binary that contains this fork must include the ISC notice from the upstream
manifest/README and this fork provenance. `smartstring 1.0.1` remains an MPL-2.0+
transitive of that pin.

## I04 bundled SQLite

`puyudb-core` links `rusqlite 0.41.1` and its bundled `libsqlite3-sys 0.39.1`.
Those crates are MIT. The compiled amalgam is SQLite 3.54.1, whose author
disclaims copyright. No SQLite source is vendored in this repository. A
distributed binary that contains the amalgam must still carry the SQLite
notice from that locked package.

## I02 research dependency obligations

Original PuyuDB files remain Apache-2.0. The dev-only Loro comparison links
unmodified `bitmaps 2.1.0`, `im 15.1.0`, and `sized-chunks 0.6.5`, whose upstream
declarations are MPL-2.0+. No MPL source was copied into PuyuDB files.

If compiled comparison executables containing that code are distributed, preserve
the notices, provide the exact covered source and license, and tell recipients how
to obtain it under MPL 2.0. Independent Apache-2.0 files are not thereby relicensed.
Vendored archives must retain the upstream licenses/notices. See
[Mozilla's MPL FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/) and the
[versioned ledger](docs/DEPENDENCIES.md).

Other new obligations include Unicode-3.0 data notices (`unicode-ident`), Zlib
source notices (`zlib-rs`), and the Boost Software License 1.0 for `xxhash-rust`.
That Boost license is **BSL-1.0**, not the unrelated Business Source License.
No executable or vendored source archive was distributed by this milestone.

The follow-up diamond-types experiment uses an exact official Git revision whose
manifest declares ISC but whose archive lacks a root LICENSE/NOTICE text.
Distribution review must resolve that notice gap. Its unmodified smartstring
1.0.1 transitive is MPL-2.0+ and has the same covered-source obligations described
above. That paragraph records the pre-T3b research pin. The production pin is the fork section above.

Source-informed Automerge regression specifications and paper-informed original
tests are attributed in [the fixture provenance record](docs/validation/I02-sources.md).
No upstream implementation, test source, paper presentation, or corpus was copied
into the project.
