# I02 DT native-interface maintainability spike

**Status: bounded technical GO for the two small extensions.** The observed
anchor and strict-inspection gaps can be addressed without a replacement merger,
decoder or native byte format. This is not engine selection, dependency promotion,
I02 acceptance, production qualification or the start of I03.

Executed 2026-10-26 against DT
`89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`, manifest 2.0.0.

## Outcome and actual footprint

The maintainer prefers DT if doing so does not require developing and maintaining
a separate text engine. The [unmodified-pin experiments](I02-diamond-types-assessment.md)
found two concrete adapter limitations. This follow-up tests native seams instead
of adding more logic to those disproven adapters.

| Extension | Native files | Added / removed lines | What is reused |
| --- | ---: | ---: | --- |
| [Insertion-order query](../../experiments/diamond-types-interfaces/anchors.patch#L9) | 1 | 33 / 0 | Existing planner, merge iterator, tracker and range tree |
| [Strict new-update inspection](../../experiments/diamond-types-interfaces/strict.patch#L103) | 4 | 117 / 18 | Existing chunk reader, content iterators, overlap mapping and CRC |
| Total | 5 | 150 / 18 | No encoder or merge-rule change |

The footprint counts native modifications, **not** the independent test drivers,
runner, product integration, security work or future maintenance. Small line count
alone would not establish maintainability; the important result is that both seams
reuse existing native decisions rather than reimplement them.

Durable outputs live in
[the experimental directory](../../experiments/diamond-types-interfaces/README.md).
The runner generates a fresh non-Git source tree under ignored `target/` from the
exact Git object and applies the stored patches. Cargo's Git cache is not edited
and upstream source is not permanently vendored.

Main `Cargo.toml`, `Cargo.lock` and `rust-toolchain.toml` remain unchanged. The
separate core harness uses the same 41 registry versions/sources/checksums already
in the main lock, plus three local path packages: harness, DT and `rle`.
No permanent fork or new runtime dependency is introduced.

## Anchor seam

`ListOpLog::insertion_spans_at` exposes local insertion-ID ranges and visibility
in **native CRDT order**, retaining deleted insertions. It rebuilds the existing
root-to-target merge tracker with fast-forward disabled, excludes the underwater
placeholder and reads the tracker's **end-state** deletion flag.

It does not compute a new order or change tie-breaking. Local ranges and their
segmentation are not portable; the adapter converts through the existing agent
assignment. A returned range can cross agent boundaries, so a range converter's
shorter returned portion must be consumed before continuing. The test adapter
converts every LV individually.

The [test cursor](../../experiments/diamond-types-interfaces/tests/anchors.rs#L1)
stores a remote insertion ID and Before/After side, or Start/End. Counting visible
scalars before that native identity boundary resolves the earlier deleted-anchor
witness to **1 in all six import orders and after reload**. The old numeric
transform's **3 versus 1** remains as an explicit negative control.

Seven anchor tests include:

- Repeated Unicode, both affinities/sides, deleted neighbors, missing/unborn IDs,
  root/edges and historical sibling cuts.
- 540 exhaustive small concurrent histories with independent scalar-gap oracles.
- 24 branching histories of 12 batches, checking atom, author, root, tip and
  additional joined cuts across delivery orders and reload.
- An agent resuming after an interleaved import, including one native span that
  crosses three agent assignments.
- Four historical native byte vectors and query/re-encoding immutability.

The generated branching comparisons establish agreement for those executions,
not an independent general ordering specification or proof of convergence.
Visible-ID checks use public native patches, which share native merge machinery.

**Cursor policy is explicit, not silently identical to the old numeric policy.**
After deletion followed by replacement, native DT can order the replacement before
retained tombstones; the identity boundary then resolves after the replacement.
The test records that result rather than forcing the old numeric position.
This policy must be accepted or revised before a permanent PuyuDB cursor profile.

## Strict new-update seam

`decode_new_strict(&self, bytes)` returns a private staged clone and owned original
metadata values. It uses the existing native parser; it does not re-encode the
input to decide whether it is valid.

The deliberately narrow dialect requires uncompressed protocol 0, insertion
content, complete top-level/nested consumption and a checked final CRC. Unknown
or unsupported chunks and leftover streams are rejected. Original DocId, opaque
user data, agent declarations—including unused/duplicate declarations—optional
start text and mapped frontiers are exposed to caller policy.

**Every submitted operation must be new.** Known prefixes/suffixes, exact native
replays and repeated IDs inside one artifact return `KnownOperation`, not silently
trimmed input. Therefore the returned appended range covers every submitted
operation, allowing existing public iterators to check owners, parents and edits.
Exact outer-artifact duplicates must be recognized separately; this is not a
replacement for ordinary native replay.

Sixteen [strict tests](../../experiments/diamond-types-interfaces/tests/strict.rs#L1)
exercise metadata, known overlaps, CRC/unknown/trailing inputs, nested leftovers,
missing content, external owner/parent policy and caller immutability.

The **exact 461-byte Puyu witness** is reproduced with its full 45-byte names and
264-byte context. Both byte-different originals pass against both opposite-order
bases, preserving submitted range `5..7`, ownership, sequence and original parent
sets, with final text `aXZYc`. No canonicalization or whole-byte reconstruction
is assumed.

Authentication, epoch/writer policy, allowed fields, start text/cut, counters,
positions and atomic publication are still caller obligations. Strict consumption
is not resource/panic security; existing native arithmetic, assertions, indexing,
allocations and unsafe-reader invariants remain separate qualification work.

## Review finding and real feature check

Independent review found a genuine compatibility defect in the first borrowed
metadata report: enabling LZ4 made returned start text borrow a locally owned
decompression buffer, causing Rust E0515.

The fix returns **owned metadata values copied only in strict mode**. The ordinary
decoder keeps its existing parsing/decompression path and empty inspection fields,
avoiding new ordinary-path metadata copies. A report-outlives-input/stage regression
passes. Review independently confirmed the fix and closed the finding.

The initial diagnostic used a clearly labeled substitute library/type-check.
Final qualification uses the **actual published `lz4_flex 0.14.0` source**, with
its `twox-hash 2.1.2` dependency, not that substitute:

| Archive | Official registry SHA-256 |
| --- | --- |
| `lz4_flex 0.14.0` | `ecbdfe44b1bd960b68170b417450a628c43f7cf56bb3c5317e61cb230ee7f226` |
| `twox-hash 2.1.2` | `9ea3136b675547379c4bd395ca6b938e5ad3c3d20fad76e7fe85f9e0d011419c` |

The optional harness verifies/extracts these archives locally and builds offline.
They declare MIT and no build script; only this separate compatibility graph adds
their two path packages. The 41 registry selections and main graph remain unchanged.
This is not promotion or advisory/distribution clearance for optional dependencies.
Registry metadata was checked on 2026-10-26:
[LZ4](https://crates.io/api/v1/crates/lz4_flex/0.14.0),
[twox-hash](https://crates.io/api/v1/crates/twox-hash/2.1.2).

The [real compressed-content control](../../experiments/diamond-types-interfaces/tests/lz4.rs#L1)
checks an actual compressed wire chunk, ordinary update decoding and snapshot
reload, strict refusal of compression and unchanged base state.

Upstream defaults also enable DT's **Unix-only storage**. They pass this control
on macOS/WSL2 but fail compilation on native Windows in unchanged
`src/storage/page.rs` / `file.rs` Unix imports. That failure is retained, not fixed
by a storage rewrite or hidden as a successful default build. Native Windows passes
the intended defaults-off text profile and the explicit LZ4-only profile.
PuyuDB's storage design does not use DT's optional storage module.

## Reproducible evidence

The [runner](../../experiments/diamond-types-interfaces/run.py#L1) tests each patch
independently and together, checks source-extraction contracts, compares 15 native
text/encoded observations before/after patches, then runs debug/release tests,
harness Clippy with `-D warnings`, and harness formatting.
Native dependency smallvec deprecation warnings remain; this is not a warning-free
upstream build. No global `RUSTFLAGS` warning suppression was used in the parent
qualification runs.

| Environment / mode | New Rust tests, debug and release | Optional feature control |
| --- | ---: | --- |
| macOS ARM64, anchors alone | 7 | Not selected |
| macOS ARM64, strict alone | 16 | Not selected |
| macOS ARM64, combined | 23 | 1 passes with native defaults/LZ4/storage |
| Debian WSL2 x86-64, combined | 23 | 1 passes with native defaults/LZ4/storage |
| Native Windows x86-64/MSVC, combined | 23 | 1 passes with LZ4, storage disabled |

Each combined qualification has 19 recorded successful steps, including four
Python extraction contracts and both test profiles. Fifteen default text/encoded
byte observations are identical before/after patches and across the three platforms.
The baseline mode has no extension tests; its evidence is the executed compatibility
program, not a fabricated test count.

Raw evidence:

- macOS combined: `target/dt-interfaces/20261026T160302137317Z-combined`.
- macOS independent modes:
  `20261026T155213164064Z-anchors`, `20261026T155213231761Z-strict`.
- Remote source/run: `target/dt-interface-remote/20261026T160459117780Z/{windows,wsl2}`.
  Source package fingerprint
  `54d4133c5fd2c28d4eb5ce6c9515b01c3ecc474c5201579597141f5da7aed4a9`,
  archive SHA-256
  `7d32ea800268a3e406a73d49367064af92eee1a9cc42268df7f659e7ff9d79ef`.
- Retained Windows-default failure:
  `target/dt-interface-remote/20261026T155724434628Z`.

[Public manifests](dt-interface-spike/manifest.json) retain patch/input/command/
raw-output hashes and the exact feature sets. [Compatibility bytes](dt-interface-spike/compat.csv)
are lossless. Per-run runner hashes are retained; the combined qualifications use
the same current compiled spike inputs. A later Git whitespace attribute only
recognizes required blank-context markers in `.patch` files; it changes no build
input or native behavior.

Four native fixture byte arrays are quoted from the pinned upstream
[`src/list/encoding/tests.rs`](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/tests.rs#L347-L389),
with attribution. Original spike test logic is newly written; no upstream test
implementation or alternate merger/decoder is copied. The upstream-declared ISC
terms, original contribution terms and missing distribution notices still need
review before publishing or distributing a modified native package.

## Maintenance judgement and remaining decision

**The tested stop criteria were not crossed.** The additions are bounded and
reviewable, with no new merge rules, persistent fields, native format version,
encoder or independently implemented parser.

There is nevertheless real ownership:

1. Preserve the no-fast-forward/end-state contracts of the private native tracker
   and planner, especially after upstream refactors.
2. Re-audit strict-mode propagation, every ignored/added field and overlap fast
   path on decoder upgrades. Known-operation rejection must remain explicit.
3. Keep original byte fixtures, semantics and all regression cases on upgrades.
   Smallvec/prerelease dependencies, notices and upstream release status remain
   obligations; a tiny patch is not a guarantee of cheap rebases forever.
4. Qualify performance. Anchor queries rebuild history; strict inspection clones
   it. These are not per-keystroke implementations, and no claim follows that
   the full binding retains the earlier native-only timing/memory advantage.
5. Qualify production safety/compatibility. Full upstream tests, fuzzing,
   hostile-input resource/panic guarantees, native MSRV 1.96, 32-bit/Wasm and the
   complete upstream feature/platform matrix have not been validated here.

**Recommendation:** DT is technically viable as the preferred candidate with a
small maintained native patch set, rather than a separately developed CRDT fork.
Before selecting it and starting I03, the maintainer must accept the cursor policy,
external epoch-owned empty-seed contract, original span/outer-batch association,
patch/dependency maintenance ownership and remaining production qualification
scope. The main workspace still uses unmodified DT and its original 110-test
research suite; the new prototype is a separate harness. No engine is accepted,
no source cache is modified, no patch is sent upstream, and nothing is committed,
pushed, published or released.
