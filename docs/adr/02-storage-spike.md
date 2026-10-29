# ADR-02 storage spike

**Status: evidence only. No engine is selected. ADR-02 is not amended.**

A follow-up on this Mac added Turso (`turso` 0.9.0-pre.2, the Rust rewrite,
not libsql) and two extra workloads. It does not change the recommendation.
Windows and Linux were still not run.

This is not I04. The harness is an isolated workspace under `spikes/adr02-storage/`
with its own `Cargo.lock`. It is not a member of the PuyuDB workspace, it is not
in `deny.toml`, and it does not change any `puyudb-*` crate. Nothing here is a
production dependency.

Reviewed sources: ADR-02 in the blueprint (§21 and §6.1–§6.5), the I04 row in
blueprint §18.3, T04 in §17.1, [the ADR index](README.md), and
[I03 closure](../I03-closure.md). I03 closed a codec, not storage. A validated
batch is not yet admissible; this spike does not add admission.

Platform for every executed check: macOS 27.2 (Darwin 27.2.0, `xnu-13432.41.4`),
Apple M1 Max, 10 cores, 34359738368 bytes RAM, APFS on an Apple Fabric SSD
(`/dev/disk3s1s1`, Solid State: Yes). Rust/Cargo 1.99.1. This is not the
blueprint's Linux NVMe reference class. Windows and Linux were not executed.
`Child::kill` is SIGKILL here; TerminateProcess was not run.

## Requirements checklist

Derived for the I04 storage contract. The spike tests the marked rows only.

- [x] Stage a chunk durably before publication. A staged chunk is not a visible publication.
- [x] Publish the root, its chunk reference, one event, and one outbox row in one commit.
- [x] Reject a duplicate `(stream, sequence)` and a publish whose chunk was not staged. Leave no extra root.
- [x] Return the durable ack only after that commit. The external ack log is fsynced only after the engine returns.
- [x] Kill the process after the chunk commit and before the root commit. Reopen must show no visible publication.
- [x] Kill at a random point after the first durable ack. Every acked id is complete. An unacked publication may be present or absent. No partial publication (root without chunk, event, or outbox; event or outbox without root).
- [x] One owning writer: a second process cannot take the owner lock while the first holds it. Crash releases the lock.
- [ ] Power-loss and faulted VFS writes. Process kill is not that test. Blueprint §6.1 says so; this spike agrees.
- [ ] Backup, checkpoint, quota, admission of a validated batch, daemon, replication, CLI.

Visible publication means a root row. Staged-only chunks are allowed.

## Candidates

None of the six requested engines were dropped. RocksDB was not added: the
blueprint already limits it to a measured LSM need, and fjall is the pure-Rust
LSM in this list. A custom log was not added: the blueprint rejects it initially.

| Candidate | Crate pin | License | MSRV vs 1.99.1 | Bundled code | Advisories in this lockfile |
| --- | --- | --- | --- | --- | --- |
| SQLite via rusqlite | `rusqlite 0.41.1`, `bundled` | MIT crate; SQLite amalgam disclaims copyright | 1.89.0 | C, SQLite 3.54.1 via `libsqlite3-sys 0.39.1` | none on this crate |
| SQLite via thin FFI | `libsqlite3-sys 0.39.1`, `bundled` | MIT crate; same amalgam | 1.88.0 | same C amalgam | none |
| SQLite via libsql | `libsql 0.10.0`, `default-features = false`, `core` | MIT | not declared; compiled here | C fork, `sqlite_version()` = 3.47.0, source id `2024-10-21 16:30:22 …alt1` | none on this crate |
| redb | `4.4.0` | MIT OR Apache-2.0 | 1.90 | pure Rust | none |
| fjall | `3.1.13` | MIT OR Apache-2.0 | 1.90.0 | pure Rust | none |
| LMDB via heed | `heed 0.23.1` | MIT crate; vendored LMDB is OpenLDAP Public License 2.8; `lmdb-master-sys 0.2.6` declares Apache-2.0 | not declared; compiled here | C, header `MDB_VERSION` 0.9.70 dated 2015-12-19, copyright in `mdb.c` through 2021 | `RUSTSEC-2025-0141` unmaintained `bincode 1.3.3` via `heed-types`, not a vulnerability |

`cargo-deny 0.20.2 check advisories` on this lockfile, with no local deny config,
reported only `RUSTSEC-2025-0141`. It did not use the workspace `deny.toml`.

Platforms: only this Mac was executed. redb 4.4.0's packaged CI workflow lists
`ubuntu-latest`, `macos-latest`, `windows-latest`, and `ubuntu-24.04-arm`; that
workflow was not run. SQLite's own portability is not a claim that this harness
passed elsewhere. heed's LMDB uses `F_FULLFSYNC` on `__APPLE__` and `fsync`
elsewhere (`mdb.c`). fjall's README says one database must not be opened from
two processes.

### How the workload is expressed

SQL engines share one schema: `chunks`, `roots` with `UNIQUE(stream, seq)` and a
foreign key to `chunks`, `events(cursor, root_id UNIQUE)`, `outbox(root_id,
peer)`. `PRAGMA journal_mode=WAL`, `synchronous=FULL`, `foreign_keys=ON` on every
open. Stage is its own autocommit. Publish is `BEGIN IMMEDIATE` plus the three
inserts plus `COMMIT`.

redb and fjall have no SQL UNIQUE. Keys are unique only because insert replaces.
The harness checks inside the write transaction (redb) or against the last
commit before the write transaction (fjall) and returns an error instead of
replacing. That is application code. A later I04 bug can violate it. heed uses
`PutFlags::NO_OVERWRITE`, so a duplicate key fails in LMDB.

Outbox is one row per root, peer fixed at 1. Event cursor is the root id. Both
are written in the same commit as the root.

### What "durable ack" maps to

| Engine | Call that must return before ack | Flush |
| --- | --- | --- |
| rusqlite / FFI / libsql | `COMMIT` with `synchronous=FULL` | SQLite `xSync` of the WAL. In the bundled amalgam, `synchronous=FULL` is not `F_FULLFSYNC`. `SQLITE_SYNC_FULL` is the Mac fullfsync, and it is off unless `PRAGMA fullfsync=ON`. Source comment in `sqlite3.c` around the pager flags. |
| redb | `WriteTransaction::commit` after `set_durability(Durability::Immediate)` | redb's default immediate commit `fsync`s; the crate calls `File::sync_all` |
| fjall | `write_tx().durability(Some(PersistMode::SyncAll)).commit()` | `File::sync_all`. Default persist is `Buffer`, which the README says is not power-loss durable. Ack uses `SyncAll`, not the default. |
| heed | `RwTxn::commit` with default flags (no `MDB_NOSYNC`) | LMDB flushes on commit. On Apple the vendored `mdb.c` defines that flush as `fcntl(F_FULLFSYNC)`. Map size in this harness is 1 GiB. |

libsql's public execute API is async. The local `core` path completed on the
first poll, so the harness does not start a Tokio runtime. libsql and
`libsqlite3-sys` both export `sqlite3_*` and cannot be linked into one binary.
That is why the spike is its own workspace of packages, not one crate.

## Crash tests

Command:

```sh
CARGO_TARGET_DIR=/tmp/adr02-storage-target \
  cargo test --workspace --manifest-path spikes/adr02-storage/Cargo.toml -- --test-threads=1
```

Each engine binary runs the same matrix:

- preflight: duplicate `(stream, seq)` rejected; missing chunk rejected; one root remains
- second process: `owner lock busy` while the first holds `owner.lock` (`File::try_lock`, fd kept until process exit)
- 24 boundary kills, seeds `0xA4020000 + n`, root 7: SIGKILL after `CHUNK_DURABLE`, then reopen, require `roots=0`
- 24 random kills, seeds `0xA4021000 + n`: SIGKILL `5 + (seed % 47)` ms after the first `ACK`, then reopen against `acks.log`

Result on this machine, after the random kill was defined as "after the first
ack": all six matrices passed.

| Package | `cargo test` time |
| --- | --- |
| adr02-rusqlite | 1.54s |
| adr02-ffi | 1.55s |
| adr02-libsql | 1.70s |
| adr02-redb | 6.11s |
| adr02-heed | 2.18s |
| adr02-fjall | 23.67s |

Those times include the seeded sleeps. They are not a ranking.

Native second-writer probe, same machine, `--no-owner-lock`, 500 ms cap:

| Engine | Probe line |
| --- | --- |
| rusqlite | `second_begin=rejected:database is locked` |
| FFI | `second_begin=rejected:sqlite 5: database is locked` |
| libsql | `second_begin=rejected:SQLite failure: database is locked` |
| redb | `second_open=rejected:Database already open. Cannot acquire lock.` |
| heed | `second_open=rejected:environment already open in this program` |
| fjall | `timed_out_likely_blocked` |

SQLite refuses a second `BEGIN IMMEDIATE` while one is held. It still allows a
second connection. The owner lock is what ADR-02's one owning process is. redb
and heed refused a second open in-process. fjall blocked.

### Observed fjall creation window

An earlier random procedure killed the process without waiting for the first
ack. One fjall directory could not be reopened:

```text
fjall/lock          0 bytes
fjall/0.jnl         67108864 bytes
fjall/keyspaces/    empty
fjall/version       absent
```

Reopen returned `FjallError: Io(Os { code: 17, kind: AlreadyExists, message: "File exists" })`.
`Database::create_or_recover` treats a missing `version` marker as create, and
create uses `File::create_new` on the lock. A kill after the lock and journal
exist and before `version` is written leaves a directory `open()` cannot
recover. Sixteen later kills at fixed 5–60 ms sleeps did not hit that window
again. It is a real hole, not a measured rate. The publication matrix above
does not cover it, because those kills happen after the first ack.

## Measurements

Release profile, no custom `RUSTFLAGS`. Workload: 20 uncounted warmup publishes,
then 300 counted publishes. Each publish is a 64-byte staged chunk plus one
durable root/event/outbox commit. Chunk keys are deterministic, not SHA-256.
Latency rank is nearest-rank `ceil(p * n / 100) - 1`. Throughput is 300 divided
by the stage-plus-publish wall time. `file_bytes` is the sum of
`metadata().len()`. `du -sk` is a separate allocation view. `reopen_verify_ns`
is a clean close, not a kill. Crash-recovery `time -p` is a kill after the
first ack; its resolution is 0.01 s, so `real 0.00` means under 10 ms, not zero.

Command: `spikes/adr02-storage/measure.sh`. Fullfsync rerun:
`ADR02_FULLFSYNC=1` and the same `--cmd measure --count 300 --seed 1`.

Raw release output, default WAL/FULL (fullfsync left off):

```text
==== adr02-rusqlite measure ====
engine=rusqlite 0.41.1 bundled sqlite 3.54.1
samples=300
rank=nearest-rank ceil(p*n/100)-1
publish_p50_ns=78208
publish_p99_ns=196166
publish_sum_ns=27190209
workload_ns=89412792
throughput_per_s=3355.2246
file_bytes=135168
reopen_verify_ns=816583
du_kib 100
crash-recovery real 0.00

==== adr02-ffi measure ====
engine=libsqlite3-sys 0.39.1 bundled sqlite 3.54.1
samples=300
publish_p50_ns=74750
publish_p99_ns=202459
publish_sum_ns=27170414
workload_ns=75694625
throughput_per_s=3963.2933
file_bytes=135168
reopen_verify_ns=894875
du_kib 100
crash-recovery real 0.00

==== adr02-libsql measure ====
engine=libsql 0.10.0 sqlite 3.47.0
samples=300
publish_p50_ns=80542
publish_p99_ns=236209
publish_sum_ns=28463432
workload_ns=47966834
throughput_per_s=6254.3215
file_bytes=135168
reopen_verify_ns=1163166
du_kib 100
crash-recovery real 0.00

==== adr02-redb measure ====
engine=redb 4.4.0 durability=Immediate
samples=300
publish_p50_ns=5094625
publish_p99_ns=9842250
publish_sum_ns=1602379126
workload_ns=3137221167
throughput_per_s=95.6260
file_bytes=290816
reopen_verify_ns=6593417
du_kib 284
crash-recovery real 0.05

==== adr02-fjall measure ====
engine=fjall 3.1.13 single-writer tx persist=SyncAll
samples=300
publish_p50_ns=4347792
publish_p99_ns=9111750
publish_sum_ns=1375756820
workload_ns=2700107667
throughput_per_s=111.1067
file_bytes=125249
reopen_verify_ns=10707666
du_kib 176
crash-recovery real 0.02

==== adr02-heed measure ====
engine=heed 0.23.1 lmdb header 0.9.70 default commit sync
samples=300
publish_p50_ns=5064209
publish_p99_ns=8847584
publish_sum_ns=1580743630
workload_ns=3106977875
throughput_per_s=96.5569
file_bytes=434176
reopen_verify_ns=1706583
du_kib 424
crash-recovery real 0.00
```

Same three SQLite engines with `PRAGMA fullfsync=ON`:

```text
==== adr02-rusqlite fullfsync ====
engine=rusqlite 0.41.1 bundled sqlite 3.54.1 fullfsync=on
publish_p50_ns=4988000
publish_p99_ns=9894750
workload_ns=3037025667
throughput_per_s=98.7809
file_bytes=135168
reopen_verify_ns=3350959
du_kib 128

==== adr02-ffi fullfsync ====
publish_p50_ns=5033500
publish_p99_ns=10165375
workload_ns=3145920417
throughput_per_s=95.3616
file_bytes=135168
reopen_verify_ns=2363541
du_kib 128

==== adr02-libsql fullfsync ====
engine=libsql 0.10.0 sqlite 3.47.0 fullfsync=on
publish_p50_ns=4964208
publish_p99_ns=10562125
workload_ns=3009413917
throughput_per_s=99.6872
file_bytes=135168
reopen_verify_ns=1202167
du_kib 128
```

Default WAL/FULL on this Mac is about 80 µs per publish. That is not the same
flush as redb, fjall `SyncAll`, heed, or SQLite with `fullfsync=ON`, which all
land near 5 ms. Comparing the 80 µs number with the others would repeat the
mistake the blueprint forbids: treating a weaker flush as the same durability.

One run, warm page cache, no isolation from other load. Not a ranking and not
a budget check against the blueprint's 30 ms p95. p99 here is under 11 ms for
every full-flush candidate, on this SSD, for a 64-byte chunk.

## Risks

- macOS `fsync` is not `F_FULLFSYNC`. ADR-02's "WAL/FULL" sentence does not name the Apple flush. Shipping the default pragma would ack publications that this machine did not full-sync.
- libsql 0.10.0's embedded engine identifies itself as SQLite 3.47.0 from 2024-10-21. The WAL-reset fix is in 3.51.3 (2026-03-13) and later, per [sqlite.org/wal.html](https://www.sqlite.org/wal.html) §11. This spike did not audit the amalgam for a silent backport. Until that audit, it does not meet the patched pin. It also cannot share a process with `libsqlite3-sys`.
- rusqlite and the thin FFI both use SQLite 3.54.1, which is after 3.51.3. The patch surface is the amalgam in `libsqlite3-sys`, not the wrapper. They were not distinguishable on latency once `fullfsync` was on.
- fjall can fail to reopen if killed while creating the first journal, before `version` exists. Publication crashes after the first ack recovered in the matrix.
- redb and fjall uniqueness is not a schema constraint. heed's is `NO_OVERWRITE` on the key, not a multi-column UNIQUE plus foreign key.
- heed vendors LMDB under the OpenLDAP Public License while the wrapper crate declares Apache-2.0. `bincode` is an unmaintained advisory, not a vulnerability. The map size is fixed; this harness sets 1 GiB. Stat length on this volume was 434176 bytes, so the reservation did not show up as file length.
- No power-loss, no torn-sector VFS, no Windows, no second machine, no hosted CI. Do not read these numbers as a platform matrix.

## Recommendation

Do not select a replacement, and do not start I04 on a different store from this
spike. The evidence supports keeping ADR-02's SQLite direction.

An amendment, if the maintainer wants one, would change the flush and pin
wording, not the engine:

- Durable ack is WAL plus `synchronous=FULL` plus the platform flush that
  reaches stable storage. On Apple that is `PRAGMA fullfsync=ON` (or an
  equivalent `F_FULLFSYNC`), measured here. Default FULL is not that flush.
- Pin a bundled amalgam at or after 3.51.3. The observed pin is
  `libsqlite3-sys 0.39.1` / SQLite 3.54.1. Do not use the system library. Do
  not treat libsql 0.10.0 as that pin.
- rusqlite or a thin FFI are both compatible with that pin. The FFI did not
  buy durability or latency. It does cost unsafe code. Prefer rusqlite unless
  I04 needs a call rusqlite cannot express.
- Keep the one owning writer as a lock outside SQLite. SQLite will not enforce
  one process.
- Do not add redb, fjall, or heed as a second backend. They passed the
  publication crash matrix at the same full-flush latency, and they do not
  replace SQL constraints, SQLite's backup tools, or the WAL-reset pin. fjall's
  creation-window failure is an extra reason not to switch.

I04 still has to implement admission, the real schema, backups, and the
power-loss drill. This spike does not close that gate.

## Follow-up: Turso and larger chunks

Turso here is the `turso` crate, the in-process Rust rewrite. It is not the
libsql engine already in the first table. Pin `=0.9.0-pre.2` (published
2026-10-28, MIT, no declared MSRV). Latest non-pre on crates.io that day was
0.8.3. The README claims Linux, macOS, and Windows; only this Mac was run.

`sqlite_version()` returned `3.50.4`. [COMPAT.md](https://github.com/tursodatabase/turso/blob/v0.9.0-pre.2/COMPAT.md)
says that string is the SQLite version used for differential testing, not an
amalgam pin. The WAL-reset bug is a C SQLite bug fixed in 3.51.3. A rewrite
reporting 3.50.4 is neither proof it has that bug nor proof it has the fix.
It does not satisfy ADR-02's patched-amalgam pin.

Settings after `PRAGMA journal_mode=WAL` and `synchronous=FULL`:

```text
rusqlite: journal_mode=wal synchronous=2 fullfsync=0 foreign_keys=1 sqlite=3.54.1
turso:    journal_mode=Text("wal") synchronous=Integer(2) fullfsync=Integer(0) foreign_keys=Integer(1) sqlite_version=Text("3.50.4")
```

Integer 2 is SQLite's FULL. `fullfsync` stayed 0. Turso's pager comment says
`FullFsync` uses `F_FULLFSYNC` on Apple and is selected by `PRAGMA fullfsync`.
Default FULL is the same trap as bundled SQLite.

`cargo-deny check advisories` on the updated lockfile still has
`RUSTSEC-2025-0141` (heed) and adds three vulnerabilities pulled by
`turso_core` through `tantivy` / `zstd`: `RUSTSEC-2026-0391`,
`RUSTSEC-2026-0398`, `RUSTSEC-2026-0403`. They are not in the publication path
this harness called. They are in the crate graph of a local open.

The crash matrix, now including a kill after 16 durable 4 KiB stages and
before any root, passed for all seven engines (`roots=0`, `staged_only=16`).
Times: ffi 1.80s, fjall 21.13s, heed 2.26s, libsql 1.70s, redb 5.59s,
rusqlite 1.55s, turso 3.12s. Turso's second open was
`second_open=rejected:database is locked`. SQLite still allows the second
connection and rejects only `BEGIN IMMEDIATE`.

Small Turso publish, release, 300 samples, 64-byte chunks, fullfsync off:

```text
engine=turso 0.9.0-pre.2 fullfsync=default
chunk_bytes=64
publish_p50_ns=92791
publish_p99_ns=158084
throughput_per_s=4897.0597
file_bytes=1462104
file turso.db 90112
file turso.db-wal 1371992
du_kib 1428
```

That p50 matches SQLite with fullfsync off, not the ~5 ms flush. With
`ADR02_FULLFSYNC=1`, 80 samples:

```text
rusqlite publish_p50_ns=4529791 publish_p99_ns=11353375 throughput_per_s=104.7072
turso    publish_p50_ns=4964125 publish_p99_ns=8710792 throughput_per_s=104.2384
file turso.db 4096
file turso.db-wal 2966432
```

Once the Apple flush is on, Turso and rusqlite are the same latency. Turso
left the small publications in a multi-megabyte WAL; SQLite checkpointed the
WAL to 0 on close. Both totals are durable only if that flush ran.

256 KiB chunks, 40 counted publishes plus 20 warmup, repeated 32-byte payload,
fullfsync off. Publish p50 stays ~70 µs for every SQLite-family engine,
including Turso, because the flush is still not `F_FULLFSYNC`. redb/fjall/heed
stay near 4–5 ms.

```text
rusqlite p50=67166  p99=121666  file_bytes=15826944 wal=0        du_kib=15424
ffi      p50=70542  p99=141458  file_bytes=15826944 wal=0        du_kib=15424
libsql   p50=73375  p99=218083  file_bytes=15826944 wal=0        du_kib=15424
turso    p50=107208 p99=504667  file_bytes=15956936 wal=1215432  du_kib=15584
redb     p50=5065709 p99=12522583 file_bytes=31985664            du_kib=31236
fjall    p50=4318000 p99=5435250  file_bytes=89813               du_kib=140
heed     p50=4417208 p99=8707417  file_bytes=17014784            du_kib=16616
```

The fjall size is not a storage win. That payload repeats 32 bytes, and fjall
compressed it. The same count with an xorshift byte stream (`ADR02_INCOMPRESSIBLE=1`,
10 counted publishes, 20 warmup, 262144 bytes) stored about the raw size in
both engines:

```text
rusqlite file_bytes=7946240 du_kib=7728 publish_p50_ns=79292
fjall    file_bytes=7912669 du_kib=7780 publish_p50_ns=4489208
file fjall/0.jnl 7904220
```

Command: `spikes/adr02-storage/extra.sh`. The xorshift pair is the same binary
with `ADR02_INCOMPRESSIBLE=1`.

Turso does not replace the recommendation. It passes the publication crash
matrix, it speaks the same SQL constraints, and with `fullfsync=ON` it matches
rusqlite's real flush. It is a pre-release, it does not carry the 3.51.3
amalgam fix, its default FULL ack is the weak Mac flush, and its crate graph
has vulnerability advisories that bundled SQLite 3.54.1 did not pull. I04
should not start on it.
