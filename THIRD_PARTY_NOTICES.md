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
above. These are research-only dependencies, not a production adoption decision.

Source-informed Automerge regression specifications and paper-informed original
tests are attributed in [the fixture provenance record](docs/validation/I02-sources.md).
No upstream implementation, test source, paper presentation, or corpus was copied
into the project.
