# I02 diamond-types long-term selection assessment

**Status: bounded DT-fit experiments complete, including preserved counterexamples.
Stable-anchor support and general strict admission do NOT pass the tested
adapter profiles. No engine selected, I02 open, I03 not started.**
Executed on 2026-10-26.

**Later follow-up:** the [native-interface maintainability spike](I02-dt-interface-spike.md)
addresses both gaps with small extensions in a separate generated harness.
The main dependency and the original adapter tests below are unchanged; their
counterexamples are not retroactively relabeled as passing results.

## Decision being made

The maintainer wants one long-term text engine before committing to a persistent
format, not a convenient initial engine with a planned later migration.
The [previous initial-AM recommendation is on hold](../adr/01-native-text-evidence.md).
DT must be assessed against PuyuDB's requirements, not against how many AM-specific
tests already exist or whether the current DT source pin has a crates.io release.

Correctness, retained historical text, exact intent cuts, offline writer ownership
and stable anchors are hard requirements. Performance and format/maintenance cost
then matter among candidates that can deliver them. Reusing a text-only engine
is a reasonable fit: PuyuDB core, not the native CRDT, owns file metadata and batches.
No promise can establish the best engine for every future workload; the objective
is an evidence-backed choice without relying on an assumed future engine switch.

## What has actually run

The [common comparison](I02-candidate-comparison.md) already executes DT at
`89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`, manifest 2.0.0, defaults/LZ4 disabled.
It passes shared ASCII/Unicode, observed-delete/concurrent insertion, duplicate,
full reload, portable historical-frontier and deleted-interval traces.
Common W01/W02 ran on macOS and WSL2; this follow-up does not rerun those
performance measurements.

The follow-up adds **41 default integration tests**: 16 DT profile, 12 native-wire,
10 DT identity/anchor projection, two Loro native-cursor controls and one Loro
same-cut serialization control. All execute on macOS ARM64, native Windows
x86-64/MSVC and Debian WSL2 x86-64. Green characterization tests preserve observed
failures; they do not mean all fit gates pass.

Code: [profile tests](../../crates/puyudb-crdt/tests/diamond_fit.rs#L1),
[profile helper](../../crates/puyudb-crdt/tests/support/diamond_fit.rs#L1),
[wire tests](../../crates/puyudb-crdt/tests/diamond_wire.rs#L1),
[anchor experiment](../../crates/puyudb-crdt/tests/diamond_anchors.rs#L1),
[Loro cursor controls](../../crates/puyudb-crdt/tests/anchor_controls.rs#L1) and
[same-cut control](../../crates/puyudb-crdt/tests/loro_fit.rs#L746).

In the retained WSL2 1 MiB/100,000-logical-batch run, DT's history construction is
107,179,655 ns, maximum saved-cut read 10,143,617 ns, and whole-process max RSS
64,704,512 bytes. These favorable observations justify deeper validation.
Loro loads faster in that run; DT is not best on every metric. W01 native calls
can defer rope work, so full-text materialization and the
[measurement boundaries](I02-candidate-comparison.md#measurement-boundaries)
must remain visible. These repetitive-ASCII fixtures exclude authenticated
admission, original-artifact storage, durable IO and a full multi-writer cost model.

The earlier broken/vulnerable published 1.0.0 build path is not an algorithm
result for this safe Git pin. [Exact provenance](I02-sources.md#pinned-diamond-types-upstream-experiment)
and [dependency risks](../DEPENDENCIES.md) remain recorded.

## Source-supported binding directions and gaps

The source facts below refer to that exact Git revision. Two independent read-only
research tasks first inspected binding APIs and format/maintenance separately.
The subsequent executed evidence below is distinguished from those source findings.

### Identity does not have Loro's numeric-peer constraint

DT's portable identity is an agent **name plus sequence**. Local `AgentId` is an
oplog-local numeric ID, not a transferable writer identity. Names are interned by
exact string equality, must be **less than 50 UTF-8 bytes**, and cannot be `ROOT`;
violations panic in the public registration path.[^names] Portable frontier
conversion has fallible APIs for unknown agents and unavailable sequences.[^remote]

A feasible direction is to domain-separately hash the entire writer key, branch,
writer epoch and content epoch, then encode all 256 digest bits within the name
limit. An unpadded base64url digest needs 43 ASCII characters, leaving room for a
short namespace prefix. This is a full-digest hash binding, not a lossless encoding
of the original tuple, authentication or a frozen naming syntax. Keep the full
tuple externally and separate seed/writer namespaces.

This avoids silently reducing a 256-bit binding to a `u64` peer. It is a material
reason to investigate DT, not proof that the whole ownership design already works.
Persistent sequence widths still need checked bounds: the public remote sequence
uses Rust `usize`, which must not become an implicit wire/disk width.[^remote]

### Epoch seed and exact intent cut

An empty branch starts with no text operations. Nonempty inserts allocate scalar
operations; merely registering a seed agent does not establish an epoch-bearing
empty native seed. Agent names are encoded when referenced by exported history.
File `user_data` can be written but is discarded during decode.[^seed]
Therefore an explicit epoch-owned descriptor/original artifact is a plausible
empty-seed direction, requiring a semantic amendment rather than a hidden
insert/delete sentinel. Cross-process native seed repetition already passes,
and the follow-up now exercises full-context fixture names and external epoch
checks. This is not authentication or acceptance of the required seed amendment.

Public checkout and `add_operations_at(agent, parents, operations)` support
authoring at an explicit cut. Agent sequences still allocate from stored agent
history, so historical authoring must not reuse newer writer counter state.[^cut]
The source's `version_contains_lv` wrapper returns true for an empty frontier,
unlike the public graph containment query. The wire suite now reproduces this
discrepancy with a real existing operation; the fixture inspector uses the graph
query, not the root-special-casing wrapper.[^root]

### Operation inspection and stable anchors are decisive

The list model exposes only insert/delete kinds, not a foreign map/tree/container
namespace. `TextOperation` exposes original scalar ranges/direction and optional
content; `iter_full` joins operations, causal history and remote identity spans.[^ops]
Text-only specialization is not itself a missing file-metadata feature.

However, native insert origins and deletion targets are computed in private merge
machinery. This inspection did not establish a public stable text-anchor
create/resolve interface or a direct public original-target-ID inspector.[^targets]
Portable version frontiers are **not stable text/comment anchors**. Existing DT
historical text tests must not be relabeled as anchor conformance.

An executed visible-scalar-ID projection now recovers insertion identities and
original delete targets using public native patches and each delete's parent cut,
including repeated Unicode and overlapping deletes. No merge algorithm is copied.
However, the tested stable-cursor projection fails import-order independence for
deleted anchors. The minimal-birth-cut alternative fixes the earlier whole-creation-
cut restriction but does not fix the counterexample below. Seek a maintainable
native identity-aware anchor seam rather than write a second CRDT merger.
This is an adapter failure, not proof that DT text fails to converge or that
every possible public-API anchor design must fail.

### Native decode/re-export is not an immutable artifact ledger

The decoder can skip unknown chunks and discard file user data; it does not
require final whole-input exhaustion. Duplicate operation ranges may be filtered
by identity rather than compared as original payload evidence.[^decode]
Auditing only newly applied operations can miss data hidden in overlapping input.

`encode_from` selects history outside a supplied frontier through the oplog tip;
it is not an arbitrary bounded batch selector. Encoding and iteration can merge
or regroup runs.[^export] Author/export from an exact staged log, and retain
original bytes, operation-ID spans, producer/dependency associations and immutable
PuyuDB publication boundaries separately. The same outer-ledger obligation
applies to AM and Loro; lack of native batch framing does not automatically reject DT.

The executed narrow profile checks the captured SHA, full writer context, exact
cut, counters, ranges/content and raw `user_data`, reconstructing via native APIs.
It rejects ignored trailing/unknown fields, missing insertion content and
conflicting known prefixes hidden by native delta application. Native decoding
occurs only in a private stage; this is not a production publication interface.

Crucially, it also falsely rejects valid updates when the same logical base cut
was assembled in opposite import orders. Whole-byte reconstruction is therefore
**not a general DT admission solution**, despite useful negative-fixture coverage.
Keep the byte guard; do not “fix” this by silently accepting uninspected input.

## Format, release and maintenance obligations

- **Release status, checked 2026-10-26:** the official crates.io API reports
  1.0.0, published 2022-08-25, as latest/default/stable. GitHub's release API
  reports no releases. The tested revision is an official upstream source pin
  whose manifest says 2.0.0, not a released stable 2.0.[^release]
  Its README calls the published Cargo package outdated; that alone does not
  establish that upstream development is inactive.[^readme]
- **Native dialect:** the list encoding module calls itself the old/original
  text encoding; it uses `DMNDTYPS`, protocol version 0. Other protocol versions
  are rejected. The historical `BINARY.md` has concrete layout drift from the
  source enum and is not a complete current decoder specification.[^format]
  Old fixed-byte compatibility fixtures are positive, but narrow, evidence.[^compat]
- **A source pin can be selected responsibly.** A release is one route, not the
  only route. If DT is selected, explicitly own the accepted native dialect,
  feature profile, original fixtures, recoverable source/dependency archive and
  security/compatibility maintenance. A SHA alone does not guarantee future
  upstream compatibility or a buildable recovery tool forever.
- **Distribution work is not an algorithm failure.** Manifest/README declare ISC,
  but the pin lacks a root license/notice text. Obtain authoritative notices and
  resolve redistribution obligations before shipping. `smartstring`'s archival/
  informational maintenance advisory and MPL source-delivery obligations require
  explicit maintenance/compliance handling, not a claim of license incompatibility
  or a demonstrated exploit.[^license]
- **No-LZ4 is not a hardened decoder.** The pinned source includes unchecked decoded
  arithmetic/TODOs, assertions and unsafe reader helpers with caller invariants.
  CRC checking is optional and late. The public merge wrapper does roll back
  returned errors; this is not a proof of panic safety or resource bounds.[^safety]
  The wire suite now reproduces a caught checkout panic for an artifact generated
  by the public encoder with insertion content omitted. The fixture inspector
  rejects that missing content before checkout. No exploit, undefined behavior,
  fuzzing certificate or general panic/resource bound is established.

These obligations must be applied fairly to every candidate. I02 must establish
credible binding feasibility and acknowledge maintenance responsibility; I03 owns
exact profile syntax and compatibility vectors. Full decoder hardening/fuzzing,
durability and supported-release qualification remain implementing/release gates,
not extra production work demanded only of DT before considering it.

## Decisive fit gates

The bounded experiments are executed. Their outcomes are deliberately not
collapsed into a single “tests passed, engine fits” label:

| Gate | Executed outcome | Disposition |
| --- | --- | --- |
| Full identity and epoch seed | 45-byte full-digest names vary with every full context field; empty/native Unicode/ASCII seeds repeat across three fresh processes on each platform; empty native seed has no operation | Bounded fixture evidence passes; external epoch descriptor and authenticated association still require explicit design/acceptance |
| Exact cut and counter ownership | Trusted closure extraction excludes unrelated newer history; root, unknown, duplicate/redundant cuts and stale-writer reuse are handled; fresh writer epoch authoring passes | Bounded native-cut evidence passes; not a production outer-stream allocator or admission ledger |
| Original targets and portable anchors | Public native-patch projection recovers insertion IDs and observed delete targets; minimal-birth cursor still resolves one deleted gap as 3 versus 1 by import order | Target projection demonstrated; general stable-anchor gate NOT satisfied by this adapter |
| Strict original-input inspection | Ignored fields/known overlap are rejected, but valid same-cut opposite-order updates falsely fail exact byte reconstruction | PARTIAL; strict, complete and representation-independent admission not established |
| Original artifacts and publication membership | All 1,000 original updates retained/replayed; bounded re-export works only after explicitly bounding the tip and restoring external context; native adjacent batches coalesce | Byte retention demonstrated separately; native snapshot is not an outer batch/producer ledger |

### Stable-anchor counterexample and Loro controls

The minimal cursor stores a primary remote insertion ID, before/after side and
gap affinity. Resolution uses that insertion's own native birth frontier, so
unrelated comment-creation history is no longer a precondition.

The preserved failure uses seed `ab`, a Right-affinity gap before `a`, and three
concurrent edits: `😀` at 0, `e` plus combining mark at 2, and deletion of `ab`.
Both replicas converge to the same `😀é` text **and the same remote scalar IDs**.
Nevertheless:

| Import order | Cursor scalar position, before / after full reload |
| --- | --- |
| left insertion, right insertion, deletion | 3 / 3 |
| deletion, right insertion, left insertion | 1 / 1 |

The literal intended gap is 1, after `😀` and before the suffix. A native patch
plan can delete `ab`, collapse the numeric cursor, then insert the suffix at that
collapsed coordinate; the simple Right-affinity transform incorrectly absorbs it.
The test asserts that exact patch trace. Interior/collapsed-gap positive probes
do not erase this counterexample.

Two Loro controls use analogous edge/interior workloads, both native `Side`
values, all six import orders, full reload and historical forks. They consistently
return native scalar positions 1 and 2 respectively. Native `Side` is not
automatically DT's experimental gap-affinity policy; this is a stability control,
not a claim of identical cross-engine cursor semantics or general conformance.

**Required DT remedy:** a maintained native remote-ID cursor resolver, or a
public identity-aware deleted-item/insertion-origin ordering seam. No replacement
merger, actor-order override or elaborate tombstone workaround is implemented.
The experiment does not prove that such an interface is impossible.

### Valid same-cut updates and serialization order

Two concurrent inserts produce `aXbYc` from `abc`. Each replica has the same
remote frontier and logical history, but imports the inserts in opposite order.
A third writer replaces `b` with `Z`.

Both replicas author/self-inspect successfully, and native cross-import gives
`aXZYc`. Their original update encodings differ, so cross-replica fixture
inspection rejects the legitimate other encoding. Independent review normalized
the public per-atom IDs, parent sets and operations and confirmed equal histories.
The two 461-byte artifacts differ only in the ordering of two 45-byte agent names
(zero-based offsets 62 and 108) and the resulting CRC payload.

The native encoder assigns wire agent IDs on first encounter while visiting
locally ordered parents; local arrival order leaks into representation. A matching
Loro control accepts one original update against both corresponding bases.
That is a positive directional fixture, not proof of universal Loro canonicality.

**Required DT remedy:** strict lossless/whole-input inspection independent of
incidental native encoding order, or a justified deterministic authoring/encoding
profile with compatibility tests. Neither is delivered by this prototype.
Canonical replay or weaker comparison is not assumed to solve it for free.

### Other executed native-wire findings

- File user data is lost unless supplied externally again on re-export.
- Trailing bytes, including a second valid artifact, can be ignored.
- Valid-CRC unknown chunks are skipped. An unknown chunk immediately before CRC
  can also prevent CRC verification; a wrong-CRC control then loads.
- Fully known conflicting IDs are ignored; known `A` plus same-writer incoming
  `BX` can become `AX`, discarding the conflicting prefix while importing `X`.
- `encode_from` includes unrelated newer tip operations; adjacent author batches
  can collapse into the same native snapshot as one insertion.
- A missing predecessor returns `BaseVersionUnknown` and is not buffered.
  Explicit retry is required. Missing-base and late-checksum returned errors
  restore the tested observable state, not a general panic-safety guarantee.
- Publicly generated contentless insertion input decodes but checkout panics.
  One isolated `catch_unwind` fixture preserves it; the profile rejects missing
  insertion content before checkout.

## Original-byte component probe

The [DT example](../../crates/puyudb-crdt/examples/diamond_fit_artifacts.rs#L1)
uses the same 10 KiB ASCII baseline, 1,000 one-character replacements and fixed
full-context metadata dimensions as the [Loro byte probe](I02-loro-fit.md).
Only the first update is inspected; the other 999 are native author/export/hash/
retention work. **No admission timing or total-storage measurement is made.**

| Component/observation | DT | Loro |
| --- | ---: | ---: |
| Original update payloads, total bytes | 370,770 | 375,708 |
| Digest values alone, bytes | 32,000 | 32,000 |
| Seed native snapshot, bytes | 10,338 | 475 |
| Full native snapshot after all edits, bytes | 14,389 | 7,382 |
| Known original replays that introduce no history | 1,000 | 1,000 |
| Byte-equal bounded re-exports in the declared profile | 1,000 | 1,000 |

DT's final snapshot contains 12,240 native operation atoms and 2,001 iterator
entries, **not** 1,001 immutable changes/batches. DT bounded re-export explicitly
reconstructs the upper cut and restores separately known original context.
Loro uses its range export; the native mechanisms are not the same.
Different native encoding/compression, framing, signatures, indexes, SQLite/WAL
and durability costs prevent a total-storage conclusion from these components.

All three platforms produced identical DT seed, snapshot and 1,000-update CSV
bytes. The fresh Loro control corpus/metrics also match the previously retained
Loro evidence. DT `updates.csv` is 870,397 bytes, SHA-256
`677777f516e610fa5c09b0c8ccdce5c24e8d03919fee055914872e43dd8ede03`.
Its ordered binary-digest index SHA-256 is
`0277c24f2a179bf00a2c4b08465b77bb59d3ba0a4af2d7334e0681e3ef5e7553`.
Final text SHA-256 matches the Loro probe:
`1fb2c5bc5f1ca6c369109903a3cc16da45179a1b771436f4b60342dc2cc48cef`.

The [retained manifest](diamond-fit/manifest.json), [raw metrics](diamond-fit/macos-metrics.csv),
[lossless update corpus](diamond-fit/updates.csv), [seed cases](diamond-fit/seeds.csv),
and native seed/snapshot hexadecimal files retain the evidence.
`expected_seed_agent` is an expected binding name; the empty native artifact does
not secretly contain that agent or an epoch-bearing seed operation.

## Verification, provenance and remaining decision

- Run ID: `20261026T142650716518Z`.
- Source fingerprint:
  `bf435fa8b97f1f3a5fd4364593adbe7d954ab4ba6b70f6fb279958f012dd5447`.
- Source archive SHA-256:
  `735c03f315c8ce87c975e68f17773eb7dc0c94a8f56e60680c15465d390de9ae`.
- Cargo.lock SHA-256 remains
  `a31fc17f20dda26b52772c23d3500842fdc372e8c691102987baf73db56851b4`.
- Raw logs:
  `target/diamond-fit-evidence/20261026T142650716518Z/{macos,windows,wsl2}`
  (ignored). Public manifests retain their hashes and explicit rustdoc flags.

All three platforms pass formatting, workspace Clippy with `-D warnings`,
**110 default integration tests (104 native research + 6 CLI)**, frozen feature
variants, frozen all-target build, warning-denied rustdoc and both byte probes.
Each remote environment completes 16 recorded steps; macOS completes 21,
including allocator calibration, both explicitly invoked default-ignored release
fixtures, cargo-deny and actionlint. Python: 21 pass on macOS/WSL2; Windows passes
20 and skips one Unix-only test. Three fresh seed processes per platform repeat
all three seed cases. Remote checks share the authorized Windows/WSL machine and
ran concurrently; these are correctness/byte observations, not timing samples.
No infrastructure, dependencies, native pin or product interfaces changed.

Independent code reviews executed the profile/wire/byte and anchor controls.
They confirmed both adapter counterexamples and their qualified interpretation;
no general native merger failure, canonicality or security proof is claimed.
The same-cut witness was first run as an expected-admission test and failed,
then retained explicitly as failure evidence without weakening the byte guard.
Independent evidence review also decoded both 1,000-update corpora, checked public
and raw inventories, all 57 source files/archive correspondence, seed repetition
and platform logs/counts. It found no discrepancy, with high confidence in the
retained figures and qualified conclusions; it did not rerun remote/native probes.

Reproduce with the pinned toolchain and a new output directory:

```sh
cargo test -p puyudb-crdt --test diamond_fit --test diamond_wire --test diamond_anchors --test anchor_controls --test loro_fit --frozen
cargo run --release -p puyudb-crdt --example diamond_fit_artifacts --frozen -- --seeds
cargo run --release -p puyudb-crdt --example diamond_fit_artifacts --frozen -- target/diamond-fit-new
```

**Experiment complete is not fit-gate acceptance.** DT has concrete advantages
and demonstrated identity/target/history feasibility, but the tested anchor and
general strict-admission paths need the native seams described above before a
permanent binding can be accepted. Loro has positive native controls here, but its
offline `u64` allocation/collision policy and empty-seed amendment remain separate
unresolved design obligations. Authentication, outer publication and hardened
decoders are undelivered for both. No new common performance run or production
support is claimed; native Linux, macOS Intel and GitHub Actions remain unexecuted.

The next decision is which obligations to own for the long-term engine, **not**
“AM now, DT later.” No engine is selected and I03 has not started.

[^names]: [Agent-name limit, reservation and exact interning](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/causalgraph/agent_assignment/mod.rs#L123-L155).
[^remote]: [Remote identity types](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/causalgraph/agent_assignment/remote_ids.rs#L16-L38), [fallible resolution](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/causalgraph/agent_assignment/remote_ids.rs#L109-L178), [owned export](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/causalgraph/agent_assignment/remote_ids.rs#L195-L208).
[^seed]: [Empty branch](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/branch.rs#L12-L19), [insert allocation](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/oplog.rs#L273-L284), [referenced-agent serialization](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/encode_oplog.rs#L156-L194), [user-data write](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/encode_oplog.rs#L659-L673) and [discard](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_oplog.rs#L642-L647).
[^cut]: [Explicit-parent authoring](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/oplog.rs#L252-L270), [agent sequence allocation](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/causalgraph/agent_assignment/mod.rs#L201-L213).
[^root]: [List wrapper](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/oplog.rs#L406-L410), [graph query](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/causalgraph/graph/tools.rs#L109-L116).
[^ops]: [Operation kinds and fields](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/operation.rs#L24-L71), [full inspection iterator](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/op_iter.rs#L179-L193).
[^targets]: [Computed insert origins](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/listmerge/merge.rs#L391-L450), [delete targets](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/listmerge/merge.rs#L513-L540), [private tracker](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/listmerge/mod.rs#L18-L44).
[^decode]: [Unknown-chunk skipping](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_tools.rs#L208-L232), [decoder end/CRC](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_oplog.rs#L946-L966), [overlap filtering](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_oplog.rs#L794-L818).
[^export]: [Export selection](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/encode_oplog.rs#L550-L559), [run merging](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/encode_oplog.rs#L105-L112).
[^release]: Live official [crates.io registry](https://crates.io/api/v1/crates/diamond-types) and [GitHub releases](https://api.github.com/repos/josephg/diamond-types/releases), accessed 2026-10-26; [pinned manifest](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/Cargo.toml#L1-L14).
[^readme]: [Upstream package-status note](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/README.md#L15-L21).
[^format]: [Encoding module/version and actual chunk enum](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/mod.rs#L1-L64), [historical binary prose](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/BINARY.md#L93-L114), [version/compression checks](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_oplog.rs#L606-L624).
[^compat]: [Older fixed-byte compatibility fixtures](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/tests.rs#L347-L389).
[^license]: [Pinned ISC statement](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/README.md#L51-L53), [smartstring first-party repository metadata](https://api.github.com/repos/bodil/smartstring), [informational advisory](https://github.com/RustSec/advisory-db/blob/main/crates/smartstring/RUSTSEC-2026-0249.md), accessed 2026-10-26; [covered-source obligations](https://github.com/bodil/smartstring/blob/e407ca23c747257a812d2d2e70bf336412718c3a/LICENCE.md#L152-L172). This is not legal clearance.
[^safety]: [Decoded sequence arithmetic](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_oplog.rs#L29-L67), [decoded positional arithmetic](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_oplog.rs#L288-L335), [unsafe reader helper](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_tools.rs#L38-L40), [merge rollback wrapper](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/decode_oplog.rs#L475-L494).
