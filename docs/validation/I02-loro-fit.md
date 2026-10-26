# I02 Loro candidate-fit experiment

**Status: bounded experiment complete; Loro is not selected, I02 remains open,
and I03 has not started.** Evidence collected on 2026-10-26.

## Scope and provenance

This follows the [common four-candidate baseline](I02-candidate-comparison.md).
It tests whether pinned **Loro 1.17.1** can support a narrow plain-text inspection
profile, full fixture-writer/epoch checks and retained original updates. Loro is
still a dev-only research dependency. Core, local integration, CLI behavior,
dependencies and Cargo.lock are unchanged.

- Rust/MSRV: 1.99.1.
- Source fingerprint:
  `b73e3d32a40c480e42fc2dfc2ea882a874b1f6eab713e317da8bd68ca7c1d973`.
- Source archive SHA-256:
  `458228297235007b1e81a21e1229c16a8543889a49a7b12bb3b067008e5a348a`.
- Cargo.lock SHA-256:
  `a31fc17f20dda26b52772c23d3500842fdc372e8c691102987baf73db56851b4`.
- Remote run ID: `20261026T123635692980Z`. Current uncommitted sources were
  transferred to new authorized directories, not an existing checkout.
- Primary-source revision: `ef83c083f19aea0d590a46db2d541b3a5daf1339`,
  recorded in the published Loro/loro-internal crate archives. Relevant cached
  source files were checked against archives whose digests match Cargo.lock.

The [retained manifest](loro-fit/manifest.json) contains the source inventory,
artifact hashes, remote step outcomes and raw-log hashes. Full remote logs remain
under ignored `target/loro-fit-evidence/20261026T123635692980Z/{windows,wsl2}`.
Fresh macOS verification logs and their source inventory are retained alongside
them in `macos/`.
The public corpus retains all 1,000 original updates, not just summary values.
Report/status edits and the evidence-only `.hex` LF checkout attribute added
after transfer do not change the inventoried Rust inputs.
The final macOS check inventory has fingerprint
`a863e127bc6b2bb89f3858211671799d7322b6e9e2aa13143efead9c65ea24d2`;
its sole source-inventory difference from the transferred set is that
`.gitattributes` rule. Every measured Rust file still matches its original hash.

This is not a production adapter, authenticated ownership registry, signed batch
format, storage implementation, ACID transaction, admission benchmark or security
proof. Fixture strings, peer assignments and limits are not normative syntax.

## Experimental inspection seam

The [fixture support module](../../crates/puyudb-crdt/tests/support/loro_fit.rs#L1)
is shared only by research tests and the byte-bookkeeping example:

1. A caller-supplied epoch registry binds the entire fixture public key, branch
   and writer epoch to one native `u64` peer. It rejects collisions, reassignment
   and the reserved seed peer instead of silently truncating full identities.
   It does **not** solve independent allocation, agreement or authentication.
2. Capture an already trusted snapshot and an explicit native intent frontier.
   Capture rejects pending local operations before snapshot export can commit
   them. Loading/forking occurs on a private document, never the caller's live
   object. This is a sequential fixture precondition, not a concurrency lock.
3. Author UTF-8 byte edits through checked scalar conversion at that exact cut.
   One explicit zero-time native commit contains the staged primitive edits.
4. Inspect the whole captured SHA-256, update mode, one-change envelope, expected
   peer/counter span, timestamps, exact dependencies and full fixture-context
   message, including outer sequence and epoch.
5. Inspect the entire trusted prefix and candidate operations. Allow only
   insert/delete on the exact root `Text("text")`; reject marks and all foreign
   or non-text containers, including those with unchanged final visible text.
6. Independently replay native positional APIs on the exact prefix, checking
   operation-counter continuity. Regenerate deletion targets rather than trust
   decoded `start_id`. Negative native deletion spans replay right-to-left.
7. Require the independently exported **whole update bytes** and resulting text
   to match. This is a narrow pinned-encoder fixture condition, not proof that
   Loro has one canonical encoding for every valid history.

Loro exposes [blob metadata and JSON inspection][inspection] and
[typed change/operation records][json-types]. However, JSON is not a lossless
wire validator: the operation deserializer explicitly [ignores unknown fields][json-unknown].
Decoded min/max version-vector bounds are also not a certificate of every
interior operation. The experiment therefore does not treat JSON export,
successful import or equal visible text as sufficient validation.

Native rich-text positions are entity positions; public Rust text edits use
Unicode scalars. The [native handler][positions] supports an identity mapping
only without style anchors. The experiment audits prior history as well as the
new update before using that plain-text assumption. It does not implement a
second CRDT merger.

## Executed behavioral evidence

All **18 tests** in the [Loro-fit suite](../../crates/puyudb-crdt/tests/loro_fit.rs#L1)
pass on macOS ARM64, native Windows x86-64/MSVC and Debian WSL2 x86-64.

| Area | Observed result |
| --- | --- |
| Full fixture-writer ownership | Collision, reassignment and reserved seed-peer use rejected; different branch/writer epoch is not the same writer |
| Exact-cut Unicode edit | Correct scalar-safe result; captured trusted base unchanged |
| Hidden operations | Map insert/delete, same-name root map, list, tree, foreign text and nested text rejected despite unchanged visible `text` |
| Rich text | Mark operations rejected by the plain-text profile |
| Context | Wrong key, branch, writer epoch, native peer, sequence, epoch or timestamp rejected |
| Whole bytes | Stale SHA rejected; altering an unused native checksum byte passes native checksum inspection but fails independent whole-byte reconstruction; actual checksum corruption rejected |
| Envelope/span | Snapshot, empty update, already-known span and oversized encoded input rejected |
| Staged edits | Multiple primitive byte edits remain one context-bound native change |
| Backward deletion | Deleting `b` then `😀` from `a😀bc` yields `ac` and reconstructs native `pos=2,len=-2` |
| Capture side effects | Pending local edit rejected before export; later explicit commit retains its operation and requested message |
| Empty seed | Zero native changes; identical native snapshots across two epochs; update from epoch 7 rejected against matching epoch-8 base/binding by native metadata checks |
| Delete targets | A known but wrong insertion target imported through typed JSON is rejected by independent reconstruction |
| Unrelated current history | Broader current history cannot silently replace the declared intent cut |
| Missing predecessor | Suffix can remain pending; ordinary snapshot omits it; retained original or native pending sidecar restores it after predecessor delivery |
| Fully known equivocation | Conflicting payload with an already-known native ID is accepted as a no-op, not compared as original-payload evidence |
| Overlap plus new counters | Conflicting known prefix raises `UsedOpID`; matching overlap applies only its new suffix |
| Boundaries | Local no-merge setting does not prevent two same-message commits becoming one exported/imported change; distinct messages preserve two in this fixture |
| Original envelope | Accepted altered envelope is not recovered from full snapshot; range re-export regenerates different bytes |

Some tests intentionally preserve native counterexamples. A green test does
not mean the corresponding native behavior satisfies PuyuDB's outer contract.

Independent correctness and scope reviewers found two implementation defects
(backward deletion direction and implicit commit during capture) and one test
gap (epoch-in-message sensitivity). Both defects were reproduced with failing
regressions before fixes. Follow-up static review confirmed all three addressed
and found no further actionable issue in the reviewed changes. Reviewers did not
independently rerun the tests or prove the upstream algorithm.

## Identity and empty-seed implications

Loro's peer is not the full PuyuDB writer identity. The fixture registry and
context message demonstrate rejection behavior only under a trusted, externally
supplied mapping. SHA-256 and native peer IDs are not signatures or permission.
A production binding would need an explicit allocation/collision policy and
authenticated association without discarding full writer, branch or epoch data.

Loro's [implicit root containers][root] need no creation operation. An empty seed
therefore has no native change on which to store the epoch message; the tested
empty snapshots are byte-identical across epochs. A separate epoch-owned outer
descriptor/binding is required. Inventing a hidden dummy operation would evade,
not establish, the proposed text-only seed contract.

These are required ADR dispositions if Loro is proposed, not silent replacements
for the proposed 33-byte Automerge actor/seed profile. Native counter/peer widths
and seed semantics cannot be forced through the AM-specific counter rule.

## Change boundaries, replay and original bytes

Explicit commits are useful authoring barriers, not immutable PuyuDB batches.
Loro's [commit API][commit] and [transaction implementation][txn] also explain
implicit commit on export/import/checkout barriers. Its local merge interval and
timestamp configuration have [documented defaults in source][defaults].

More importantly, [nonlocal insertion][remote-merge] uses different merge
configuration, and [ordinary update export][export-merge] builds a scratch change
store that can coalesce changes. The [merge predicate][merge-predicate] includes
peer, contiguous counters/dependencies, timestamp interval and equal messages.
The executed two-commit counterexample turns 2 local changes into 1 exported
and reloaded change with no messages, even with local merge interval `-1`.
Distinct fixture sequence messages preserve 2/2/2, but that is a constrained
profile observation, not an immutable-boundary guarantee or outer batch ledger.

Fully known updates can be discarded without payload comparison. For overlapping
known history that introduces new counters, [known-history validation][known]
can reject conflicting operation content/dependencies; matching input is sliced
to its new suffix. Neither path verifies original transport bytes, all original
metadata or application publication boundaries.

A [full native snapshot][snapshot] represents native state/oplog, not a bag of
submitted update envelopes. Pending remote input needs separate handling:
[pending export][pending] provides a sidecar, and the test also restores via
retained original bytes. State-only or shallow exports are not a substitute for
the full-history profile. [Range export][range] re-encodes a native span; it does
not retrieve the original submitted artifact.

**Consequence:** a PuyuDB Loro binding must retain and validate original artifacts
and their outer publication associations separately. Native idempotence alone
must not authorize producer records, equivocation acceptance or durable receipts.

## Byte-component experiment, not total storage cost

The [artifact example](../../crates/puyudb-crdt/examples/loro_fit_artifacts.rs#L18)
uses a 10 KiB ASCII baseline and 1,000 one-character replacements. Every batch
has a distinct full fixture-context message and two native operation atoms.
Only the first update goes through the experimental inspector; the other 999
exercise native author/export/hash/retention. No admission latency is measured.

The three [raw metric CSVs](loro-fit/macos-metrics.csv) are byte-identical.
All retained seed/snapshot/update artifacts were also compared byte-for-byte
across macOS, Windows and WSL2:

| Component/observation | Value |
| --- | ---: |
| Baseline text | 10,240 bytes |
| Seed native snapshot | 475 bytes |
| Original update payloads, total | 375,708 bytes |
| Smallest/largest update | 372 / 377 bytes |
| SHA-256 digest values alone, 1,000 × 32 | 32,000 bytes |
| Full native snapshot after all updates | 7,382 bytes |
| Native changes before/after snapshot reload | 1,001 / 1,001 |
| Replayed known original updates that were no-ops | 1,000 / 1,000 |
| Same-span re-exports byte-equal to original in this fixture | 1,000 / 1,000 |

The original payload and digest components cannot be inferred from the 7,382-byte
snapshot. This is **not** total persistent storage: framing, keys, batch records,
indexes, signatures, SQLite/WAL, allocator overhead and durability are absent.
The CSV's hex doubles payload representation and is an evidence format, not
the proposed database layout. No timing, RSS or allocation conclusion follows.
The altered-envelope regression above disproves a general guarantee of
recovering original bytes despite this fixture's 1,000 matching range exports.

`updates.csv` is 836,272 bytes, SHA-256
`60afeec89a82232b05058a82d637f4d77c4667dc81bd6395aa5b2e5c832e2528`.
The ordered concatenation of its 1,000 binary digest values hashes to
`6f12abc5d5c040f5aad5f17f53b37bc9cbf503d4fed3aa187fb47d254f882e72`.
The final text SHA-256 is
`1fb2c5bc5f1ca6c369109903a3cc16da45179a1b771436f4b60342dc2cc48cef`.

## Decoder limits and remaining gates

The native fast envelope [checks xxHash32][checksum], not cryptographic
authentication; only the last four bytes of its 16-byte checksum area are checked.
The whole-artifact SHA check is separate. [Metadata decoding][meta] already
decodes/allocates from the payload before returning counts, so the experimental
8 MiB encoded cap and post-decode operation limits do **not** establish bounded
inflation, allocation, CPU, recursion or panic behavior for hostile inputs.

Whole-byte replay equality is stronger than visible-text comparison for the
tested narrow profile. It is not a general binary-format audit, canonicality
proof, fuzzing campaign or complete acceptance proof for every valid plain-text
history/encoding. The experiment reloads/forks and scans trusted history; its
performance is not the native W01/W02 performance.

The subsequent [engine/binding proposal](../adr/01-native-text-evidence.md) is
under reconsideration for a long-term choice before format freeze. The
[diamond-types follow-up](I02-diamond-types-assessment.md#decisive-fit-gates)
is now executed with two unmet adapter gates and three additional Loro controls.
This report's original run contains 18 Loro-fit tests; the follow-up adds a 19th
same-cut control plus two separate native-cursor controls. Loro remains a serious
contender and AM remains the conformance reference.
Obtain the maintainer's direction and cost acceptance before I03.
Do not select Loro from this experiment or silently accept either native profile.
Production hardening and full-system cost/durability remain undelivered.

## Verification and reproduction

Fresh checks on all three platforms passed:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo test --workspace --no-default-features --frozen
cargo test --workspace --all-features --frozen
python3 -m unittest discover -s scripts -p 'test_*.py'
cargo build --release -p puyudb-crdt --example loro_fit_artifacts --frozen
```

Use `py -3.14` rather than `python3` for the executed Windows Python check.
There are **69 default integration tests: 63 native research + 6 CLI**.
Python: 21 pass on macOS/WSL2; Windows passes 20 and skips one Unix-only test.
Each remote run's ten recorded steps, including compiler probes and example
execution, exited zero. The initial transport command exceeded Windows command
length; transferring the runner separately resolved it. The first Windows
runner launch could not resolve Rust from a child-only PATH; explicit executable
resolution fixed the launcher. Neither required a project-source change.
Windows and WSL2 checks ran concurrently; these are correctness/byte checks,
not performance measurements.

Additional fresh macOS-only checks passed: frozen all-target build, warning-free
rustdoc, cargo-deny, actionlint, allocator calibration and both explicitly invoked
default-ignored release fixtures. They were not rerun remotely in this slice.
No common performance benchmark was rerun; its measured code is unchanged.
WSL2 is approved provisionally for I02, not independent native Linux evidence.
GitHub Actions, native Linux and macOS Intel remain unexecuted.

Independent read-only evidence review verified the 1,000 payloads, digest/span
index, retained-file hashes, saved platform logs, test counts and
source-inventory correspondence without finding a discrepancy. It did not rerun
project checks. macOS feature-variant results remain parent-transcript evidence;
the saved rustdoc manifest does not record its `RUSTDOCFLAGS` environment value,
so the reviewer confirmed success/no warnings, not that flag independently.

Reproduce the byte probe with a **new** directory:

```sh
cargo test -p puyudb-crdt --test loro_fit --frozen
cargo run --release -p puyudb-crdt --example loro_fit_artifacts --frozen -- target/loro-fit-new > target/loro-fit-new.csv
```

Verify the retained evidence independently with Python's standard library:

```sh
python3 - <<'PY'
import csv, hashlib, json
from pathlib import Path

root = Path("docs/validation/loro-fit")
manifest = json.loads((root / "manifest.json").read_text())
for name, expected in manifest["files"].items():
    raw = (root / name).read_bytes()
    assert len(raw) == expected["bytes"]
    assert hashlib.sha256(raw).hexdigest() == expected["sha256"]
for name in ("seed", "snapshot"):
    raw = bytes.fromhex((root / (name + ".hex")).read_text())
    expected = manifest["decoded_native_artifacts"][name + ".native"]
    assert len(raw) == expected["bytes"]
    assert hashlib.sha256(raw).hexdigest() == expected["sha256"]
rows = list(csv.DictReader((root / "updates.csv").open()))
assert len(rows) == 1000
index = hashlib.sha256()
total = 0
for i, row in enumerate(rows):
    assert [int(row[k]) for k in
            ("ordinal", "native_peer", "native_start", "native_end")] == [
                i + 1, 2, i * 2, i * 2 + 2]
    raw = bytes.fromhex(row["artifact_hex"])
    digest = hashlib.sha256(raw).digest()
    assert len(raw) == int(row["bytes"]) and digest.hex() == row["sha256"]
    index.update(digest)
    total += len(raw)
assert total == 375708
assert index.hexdigest() == "6f12abc5d5c040f5aad5f17f53b37bc9cbf503d4fed3aa187fb47d254f882e72"
print("verified", len(rows), "original updates,", total, "payload bytes")
PY
```

[inspection]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro/src/lib.rs#L1609-L1683
[json-types]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/encoding/json_schema.rs#L1090-L1268
[json-unknown]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/encoding/json_schema.rs#L1351-L1377
[positions]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/handler.rs#L2349-L2405
[root]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-common/src/lib.rs#L594-L605
[commit]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro/src/lib.rs#L632-L708
[txn]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/txn.rs#L501-L579
[defaults]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/configure.rs#L40-L52
[remote-merge]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/oplog/change_store.rs#L1344-L1406
[export-merge]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/oplog/change_store.rs#L1088-L1125
[merge-predicate]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/change.rs#L267-L282
[known]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/oplog/known_history.rs#L91-L255
[snapshot]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/encoding/fast_snapshot.rs#L25-L60
[pending]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/loro.rs#L3532-L3544
[range]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/oplog/change_store.rs#L337-L393
[checksum]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/encoding.rs#L311-L329
[meta]: https://github.com/loro-dev/loro/blob/ef83c083f19aea0d590a46db2d541b3a5daf1339/crates/loro-internal/src/encoding/fast_snapshot.rs#L555-L628
