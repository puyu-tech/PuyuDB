# Strict native input inspection spike

Bounded feasibility evidence, not engine selection, a production admission
interface, a new binary version, or decoder hardening. Source:
`89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`, default features/LZ4 disabled.

## Interface and contract

The [patch](strict.patch#L103) adds
`ListOpLog::decode_new_strict(&self, bytes) -> Result<(Self, StrictDecodeInfo), ParseError>`.
It uses the existing native parser on a private clone. Returned errors cannot
mutate the base. The caller can reject the staged result before publishing it.
Default load/replay remains permissive and retains its existing rollback wrapper.

Success means **every submitted operation is new**, and `submitted` covers all
of them in the staged log. Any known span, including an exact replay, a
conflicting prefix/suffix, or repeated IDs inside one artifact, returns typed
`KnownOperation`. Exact outer-artifact duplicate recognition must happen
separately; strict inspection does not compare or admit conflicting known IDs.
Existing public `iter_full_range` then exposes all original submitted operations,
owners and parents, without re-authoring or comparing native output bytes.

The [report](strict.patch#L299) owns copies of original DocId, opaque user data
(absent versus empty preserved), agent declarations in wire order (including
unused/duplicate names), optional start text, and mapped start/end frontiers.
Metadata is copied only in strict mode; default decoding leaves those report
fields `None`/empty rather than adding new string/byte allocations. Ownership
also keeps the report independent of input lifetime or temporary native
decompression buffers; the report-outlives-input test enforces this contract.
The staged oplog is not an archive of those fields or the original artifact.
Caller policy must check metadata, start text/cut, ownership, counters, positions,
and parent relationships. Parsing and CRC are not authentication.

The strict dialect requires uncompressed protocol 0, a checked final four-byte
CRC, complete input and nested-payload consumption, and insertion content.
Unknown chunks, end-branch/transform/compression extensions, duplicate content
streams and leftover positional operations are rejected rather than skipped.
Understood representation differences remain valid: this is **not canonical
encoding**. Optional declarations/metadata may differ and must be exposed to
policy, not silently normalized away.

## Evidence

The self-contained [tests](tests/strict.rs#L1) cover opposite import orders:
both original, byte-different third-writer updates pass against both same-cut
bases. Native parsing supplies metadata and the all-new range; external policy
checks original operations and parents. There is no whole-byte reconstruction.
Alongside the small generic fixture, the exact Puyu witness uses independently
computed, hardcoded 45-byte full-context names: epoch `[7; 32]`, branch/writer
epoch `[1; 16]`, concurrent key 2 inserting `X` at 1 and key 3 inserting `Y` at 2
into seed `abc`, then key 1 replacing `b` with `Z`. Both original updates are
461 bytes, differ only in the agent names at offsets 62/108 and final CRC, and
pass against both bases. Assertions preserve the exact 264-byte user data, full
original agent declarations, submitted range `5..7`, operation ownership,
sequence ranges and original parent sets. The test uses public native APIs and
standard-library formatting, with no new dependency or repository-relative import.
Puyu-specific context remains in the test, not the generic native patch.

Negative cases include valid-CRC unknown chunks, trailing/second artifacts,
unknown-before-CRC bypass, missing/invalid/oversized CRC payloads, nested and
ignored leftovers, overwritten content, buffered operation remainders, missing
insertion content, foreign ownership, wrong parents, known overlaps and repeated
IDs. Separate controls preserve native permissive behavior. Late-CRC and
missing-base errors preserve the base's operations, history, agents and encoding.

Executed locally on macOS ARM64 / Rust 1.99.1:

- Archived the exact read-only cache pin into non-Git
  `target/dt-strict-spike/native`; applied the stored patch with
  `git apply --directory=target/dt-strict-spike/native`.
- Tiny separate workspace/path harness, `default-features = false`, seeded
  from the main lock, then offline resolution. All 41 registry packages retain
  the main lock's exact version/source/checksum. Only harness and path DT/rle
  entries are added; 185 irrelevant main-graph entries are pruned.
- Initial `strict_rejects_trailing_input` using default native decode failed
  as expected; changing to the implemented strict seam passed. Ignored
  `target/dt-strict-spike/red.log` and `green.log` retain that red/green run.
- `cargo test --manifest-path target/dt-strict-spike/Cargo.toml --offline --locked
  --test strict --test native_wire`: **16 strict + 12 unchanged wire tests pass**.
  The same targets pass with `--release`.
- Harness `cargo fmt --check` and `cargo clippy --offline --locked --all-targets
  --all-features -- -D warnings` pass. The path dependency still emits its 63
  existing smallvec deprecation warnings; this is not a warning-free upstream build.
- Main Cargo manifests/lock unchanged. Main lock SHA-256 remains
  `a31fc17f20dda26b52772c23d3500842fdc372e8c691102987baf73db56851b4`.

### LZ4-feature compile regression and bounded correction

Review identified that the initial borrowed report could expose start content
borrowed from the decoder's local decompression buffer, breaking compilation
with the LZ4 feature even though strict mode rejects compression at runtime.
A **type-check-only stand-in** for the three referenced `lz4_flex` functions,
compiled with `cargo rustc -p diamond-types --lib -- --cfg 'feature="lz4"'
--extern lz4_flex=...`, confirmed E0515 before the correction and successful
compilation afterward. Enabling both default feature cfgs (`lz4` and `storage`)
also compiles with that stand-in. No stand-in function was executed. Owned
metadata fixes the lifetime coupling without refactoring native decompression or
parsing.

This synthetic compiler check is **not real LZ4-feature qualification**.
The actual offline feature check fails because `lz4_flex 0.14.0` is unavailable
in the cache. No dependency was downloaded, substituted in the durable Cargo
graph, or changed in the main manifest/lock. Ignored
`target/dt-strict-spike/lz4-{typecheck-before,typecheck-after,real-check}.log`
retain those separate results.

## Maintenance and limits

Native footprint: **4 files, 117 additions / 18 deletions**. No encoder, merge
algorithm, protocol version or main dependency changes. The parser's existing
chunk reader, content iterators, overlap mapping and checksum implementation are
reused; there is no second decoder. Upgrades must re-audit mode propagation into
nested readers, every ignored field/chunk, and overlap fast paths.

Private-clone staging and owned metadata have additional time/memory cost; neither
was benchmarked.
Existing unchecked arithmetic, indexing, allocations, unsafe-reader invariants
and assertions remain: complete format consumption is **not** panic/resource
security. No fuzzing, independent review, full upstream suite, production
admission, cross-platform rerun, or hardened decoder is claimed. No blocker was
encountered for this bounded interface; those remain separate qualification work.
