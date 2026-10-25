# I02 fixture and comparator provenance

All PuyuDB test implementations are newly written Apache-2.0 code. The
source-informed cases below adapt behavioral specifications, not upstream
implementation code or a copied fixture corpus. Paper figures/prose/code are not
redistributed.

## Automerge regression specifications

The published **Automerge 0.12.0** archive is pinned by its registry checksum:

```text
2fb849130973a7808a167887f56b1e8bdc36b9b541542602dcb337f34aecf47b
```

Its recorded VCS revision is `37545d0d72b50f52084a2eca2cadf2eb1a5a512a`,
subpath `rust/automerge`. That recorded revision was not retrievable through
GitHub during research; it is not represented as a working GitHub source link.
Use the checksum-pinned [published archive](https://static.crates.io/crates/automerge/automerge-0.12.0.crate),
[release metadata](https://crates.io/api/v1/crates/automerge/0.12.0), and
[exact-version test source](https://docs.rs/crate/automerge/0.12.0/source/tests/test.rs).

The manifest declares MIT. Source attribution: the Automerge contributors.
A separately retrievable [repository MIT license](https://github.com/automerge/automerge/blob/5bae7757ef7b92c8cd6850def388f22767305ccb/LICENSE)
states copyright 2019–2021 the Automerge contributors; this corroborating license
file is not claimed to be the release's recorded revision.

| Original test | New text adaptation and independently specified expected result |
| --- | --- |
| `does_not_interleave_sequence_insertions_at_same_position`, upstream `tests/test.rs` lines 1459–1516 | Two ordered native actors insert forward runs `xy` and `UV` into one empty seed. Equal first insertion counters and higher actor for `UV` imply the pinned result `UVxy`. |
| `insertion_after_a_deleted_list_element`, upstream `tests/test.rs` lines 1306–1351 | From `hjk`, one writer deletes observed `j,k`; another inserts `Q` after `j`. Merged result `hQ`, while original cuts remain `hjk`, `h`, and `hjQk`. |

The upstream specifications use lists. Our tests use one text object, different
labels and explicit fixture-derived actors, native byte/sequence metadata and
historical reload checks. The actor/counter ordering precondition is asserted;
actor labels alone are not a universal native ordering oracle.

## Paper-informed backward insertion diagnostic

Matthew Weidner and Martin Kleppmann, **The Art of the Fugue: Minimizing
Interleaving in Collaborative Text Editing**, [arXiv:2305.00583v3](https://arxiv.org/abs/2305.00583v3),
especially Figure 2 and Appendix A-A8, motivates a fresh backward-insertion trace.

This arXiv version has the arXiv perpetual distribution license, not an open
adaptation license. We cite the research question; no paper presentation or
implementation is copied.

Our original trace uses two offline writers with actual native actor order
`A < B` and equal initial operation counters:

- A inserts `p`, then inserts `m` before it: local result `mp`.
- B inserts `q`, then inserts `n` before it: local result `nq`.
- The source/algorithm-derived Automerge expectation is `nmqp`.

Execution on the pinned candidate confirms `nmqp` across all six topological
interleavings of the two two-change chains, including duplicate delivery and
historical reload. It is neither `mpnq` nor `nqmp`. This is **convergence with
backward interleaving**, not a convergence failure and not a promise of maximal
non-interleaving. A proof for FugueMax is not a proof for Automerge/Yrs/Loro.

## Pinned diamond-types upstream experiment

The broken published 1.0.0 remains a recorded build/security finding, not an
algorithm result. With maintainer authorization to continue the safe comparison,
the dev-only dependency now selects official upstream:

```text
Git:      https://github.com/josephg/diamond-types
Revision: 89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922
Manifest: diamond-types 2.0.0, Rust 1.96.0, ISC
Defaults: disabled (including optional LZ4 and storage)
Archive SHA256:
47eb1d9c78877641239b5a1e1c8167fdf506fac5657951e2e4cbeafcc411258b
```

This is an **unpublished upstream Git revision**, not a stable crates.io 2.0
release. GitHub reports the commit as unsigned. The full revision and package
version are both constrained in Cargo.toml; the lockfile pins the same source
for its `rle` path dependency. No upstream source is patched or vendored.

The [exact manifest](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/Cargo.toml)
and [feature-gated encoder](https://github.com/josephg/diamond-types/blob/89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922/src/list/encoding/encode_oplog.rs)
fix the no-LZ4 build path. The selected graph does not enable DT's optional LZ4.
Actual build, common observed-delete trace, duplicate decoding, full reload, and
portable historical-frontier assertions pass locally. Native positions are scalar
indices; native local indexes are never transferred between independent oplogs.
Historical cuts use remote agent/sequence frontiers and fallible local resolution.
The follow-up also runs a scalar-indexed Unicode/combining/CRLF trace with
concurrent emoji deletion and Japanese insertion, retaining each original cut.
No stable native DT anchor interface is established by these tests; do not
promote text/frontier coverage into an anchor-capability claim.

The archive lacks a root LICENSE/NOTICE text; the manifest and README declare ISC.
This notice gap, smartstring's MPL/source obligations and maintenance advisory,
and a prerelease smallvec 2 transitive require review before any distribution.
The experiment is not approval to ship that dependency.

## Negative native-wire fixtures

`tests/native_wire.rs` constructs small malformed or extended chunks independently
from the pinned source layout, rather than copying native decoder code:

- A well-formed unknown integer column, with recomputed valid native checksum,
  remains natively decodable but is lost by expanded inspection. The experiment
  rejects the non-roundtripping original bytes.
- Genuine native author metadata is constructed through upstream author APIs.
  A separately malformed footer demonstrates why `author() == None` is
  insufficient without also requiring empty extra bytes.
- Zero, over-width and crossing-range counters, oversized input, an already
  deleted target, and a dependency known outside the intent cut are rejected.

This is a negative corpus and feasibility check, not a replacement canonical
codec, proof over every native extension, fuzzing certificate, or bounded
untrusted-input decoder.
