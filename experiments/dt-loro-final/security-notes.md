# Bounded malformed-input screening

This is a deterministic, generated smoke/property corpus, **not** a cargo-fuzz
coverage-guided campaign, hostile-input admission implementation, panic-safety
certificate, or allocator/undefined-behavior guarantee.

## Child protocol

The generated package must place `screen.rs`, `compare.rs` and the existing
`loro_fit.rs` support module together. `compare.rs` exposes its existing
`pub(crate) mod loro_fit` and the two `dt_screen_*` hooks. There is no second DT
binding implementation: screening invokes the final comparison's strict
inspector.

Run `screen ENGINE list` to enumerate the bounded corpus without decoding any
mutant. Engines are `dt-binding`, `dt-native`, `loro-binding`, `loro-native`.
The list has this CSV header:

```text
case_index,family,expectation,risk,checksum,encoded_bytes
```

Run `screen ENGINE CASE_INDEX` in a **fresh parent-supervised child process** for
one trial. Its single stdout CSV row has no header:

```text
engine,layer,case_index,family,expectation,risk,checksum,encoded_bytes,count,errors,accepts,panics,base_mutated,snapshot_panics,violations,elapsed_us
```

`count` is one. Counts and other booleans are numeric; `base_mutated` is `0`, `1`
or `unknown` when post-state observation fails. `errors` counts returned errors,
never panics. Exit 0 means a completed observation or passing control; exit 1
means an unwind, control violation, snapshot failure, or forbidden base mutation;
exit 2 means CLI/setup failure. Native successful imports normally change their
receiver. Native mutation after error/panic is a diagnostic finding, not a claim
that the staged binding leaked a change. Binding bases must remain unchanged
even on success. Snapshot failure sets `base_mutated=unknown` and remains a
diagnostic failure; it is never counted as an observed caller mutation.

Stderr flushes `SEED`, `VALID` and `TRIAL` records with exact hex before decoding.
`ATTEMPT_DONE` distinguishes completed attempts from subsequent snapshot
diagnostics. A crash/abort/timeout can have no result row; the parent must retain
stderr, exit/signal and wall-time outcome rather than counting it as an ordinary
returned error. `elapsed_us` covers the attempt and post-state observation;
parent wall time additionally includes fixture/corpus construction and logging.

## Corpus and oracle

Every small-update truncation offset is included, along with targeted bit/byte
mutations, trailing/concatenated input, DT unknown/unsupported chunks, extreme
varints/length fields and fixed-seed random byte strings. Each engine has at most
600 cases, with each encoded candidate at most 64 KiB. High-risk trials remain
separate children. No gigabyte-sized host fixture is generated.

Valid Unicode controls must accept. Required rejections are based on explicit
profile rules, not the assumption that any changed byte is invalid. Remaining
accepted mutations are observations. The Loro binding reconstructs the candidate
`CapturedUpdate`, recomputing its outer SHA-256; stale artifact hashes are not
used as a malformed-input oracle.

DT repaired mutations use the existing strict-test CRC-32C arithmetic
(`0x82f63b78` reflected polynomial), with a final `[100,4]` chunk. Loro current
FastUpdates/FastSnapshot checksums follow the pinned primary implementation:
`loro-internal` 1.17.1 `src/encoding.rs` checks XXH32 over `[20..]`, seeded with
`u32::from_le_bytes(*b"LORO")`, stored little-endian at `[16..20]`. The safe
independent fixture helper follows `xxhash-rust` 0.8.19 `src/xxh32.rs` and
`src/xxh32_common.rs`; tests verify known vectors and authentic native exports.
It is not a native parser and adds no dependency. Loro repairs concern this
outer checksum, not every possible internal checksum in every historical format.

## Resource and state limits

DT's selected native features are off and strict decoding admits only its narrow
uncompressed dialect. An unsupported compression marker is a rejection control,
**not** a DT compressed-payload expansion test. Loro exercises authentic default
format: a native 32 KiB text update, and a native default compressed snapshot
containing a 128 KiB insertion. The snapshot is forked from the exact trusted
base and adds a fresh peer's operations; it is not conflicting seed history.
Snapshot rejection by the update-only binding does not establish a decompressor
resource bound. The raw native import trial separately reaches that path.

The parent owns wall deadlines, OS limits, crash capture and final results.
Record unsupported resource limits honestly: on the validation macOS host,
setting `RLIMIT_DATA` to 512 MiB raises `ValueError`; CPU/core limits work.
Wall/CPU limits without an effective memory limit are not memory containment.

Before/after native snapshots observe semantic state, including DT's unused-agent
count and Loro's pending transaction count. The immutable Loro binding base is
also reconstructed/exported before and after inspection; no aliased `LoroDoc`
clone is mistaken for an independent base. These checks do not observe arbitrary
interior caches or guarantee that damaged native state remains snapshotable.
`catch_unwind` is only a diagnostic aid; native aborts, OOM and process failures
still require the parent watchdog and external containment.
