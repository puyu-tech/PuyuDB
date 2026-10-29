# I04 plan — atomic SQLite storage publication

Branch: `i04-atomic-sqlite-publication` (off the merged I03, `2aec9d6`).
Normative source: `/Users/pixel/Downloads/puyu/PUYUDB-BLUEPRINT.md` — the I04 row
in §18.3, plus §4.4, §6.1–§6.6, §17.1 (T04), §17.3, §22.1, §22.2. Acceptance is
T04: *faulted SQLite/VFS writes/fsyncs, process kill, power-loss model,
disk-full; no partial multi-file publication; acked batch fully recoverable,
unacked may be present or absent.*

ADR-02 pin (accepted 2026-10-29, `docs/adr/02-i04-pin.md`): `rusqlite` 0.41.1
bundled SQLite 3.54.1 (≥ 3.51.3 WAL-reset fix), `journal_mode=WAL`,
`synchronous=FULL`, `fullfsync=ON` on Apple, one owning process via an OS lock
beside the database.

## Already on the branch

- `crates/puyudb-core/src/publication.rs`: `Store` over `space.sqlite` with
  `chunks`, `batches`, `batch_chunks`, `events`, `outbox`; `stage_chunk` is its
  own durable autocommit; `publish` verifies signature/key equality, recomputes
  the batch id, checks stream step and parent lamports, quarantines on
  equivocation, requires staged chunks, and commits batch + chunk refs + event +
  outbox in one `BEGIN IMMEDIATE … COMMIT` under WAL/FULL; the ack returns only
  after that commit. Owner lock via `owner.lock`, retry before `Busy`.
- `crates/sqlite-fault`: VFS shim — failed `xSync` discards unsynced bytes,
  torn `xWrite` leaves a short prefix, then refuses further writes.
- Tests: `tests/publication_power.rs` (lost sync / torn write during publish
  and stage, disarmed publish, acked batch surviving a later armed fault;
  serialized around the global VFS state), `tests/publication_crash.rs`
  (SIGKILL after staged chunk → no publication).
- B1 done (`64f0136` plus the audit-fix commit): manifest + `format_meta`.
- Spike evidence on `adr02-storage-spike` (`docs/adr/02-storage-spike.md`):
  six engines, boundary/random kill matrices — macOS only, SIGKILL only.
- Audit of B1 + ledger (`87e1c2b`) found and the follow-up commit fixed three
  blockers: `publish` now rejects a batch naming another space (the store
  keeps its `space`); a writer stream is keyed by the full
  `(public_key, branch, writer_epoch, sequence)` tuple per spec §13, so two
  keys sharing epoch bytes no longer equivocate; and the manifest is written
  only after the owner lock is held, closing the two-process race that could
  leave manifest and database naming different spaces. Also fixed: exact
  `lamport = 1 + max(parent lamport)` enforcement, bounds-checked snapshot
  indexing in `sqlite-fault::reset` paths, strict `created_unix`/no-CR
  manifest parsing, and corruption (not silent rebind) when an existing
  database has an emptied `format_meta`. Two residuals found while verifying
  those fixes are also closed: reset under an open file must fail the sync
  without restoring an empty snapshot (R01), and reset must never shrink the
  snapshot vector, which would alias file ids across two fault stores (R02);
  both have regression tests in `publication_power.rs`.

## Scope (I04 tracer bullets, in order)

| # | Bullet | Blueprint | Files |
|---|---|---|---|
| B1 | `format_meta` (schema major/minor, native codec, space id) + per-space `manifest.toml` (format, space, storage/lock profiles, creation time); open rejects unknown major formats and manifest/DB space mismatch; `quick_check` at open | §6.2, §6.3, §6.5 | `publication.rs`, tests |
| B2 | Artifact layer: `artifacts` / `artifact_chunks` / `batch_artifacts`, §22.1 artifact manifest (kind, raw length, raw digest, ordered chunk ids), 256 KiB chunk cap, `content_availability` (resident / intentional-missing / lost) so Applied ≠ Hydrated | §6.3, §22.1, §22.2 | `publication.rs`, `profile.rs`, tests |
| B3 | `batch_parents` DAG table + `(stream,seq)`/parent→child indexes, plus the §4.3 parent-closure and minimal-frontier checks at admission (currently only existence + lamport are checked) | §6.3, §4.3, §13 | `publication.rs`, tests |
| B4 | `requests` idempotency table: same caller key returns the same receipt; conflicting key reuse rejected; cancel-before-publish vs unknown-after-commit per §6.4 | §6.4 | `publication.rs`, tests |
| B5 | `staged_batches` / `peer_receipts` schema for replication staging (apply path itself is I21) | §6.3 | `publication.rs` |
| B6 | Fault hooks: disk-full (ENOSPC) in `sqlite-fault` in addition to lost sync/torn write; power-loss tests for each; publish/stage under disk-full leave no partial publication and no ack. Also add the fixed SQLite WAL-reset regression scenario (§17.3) as an explicit T04 evidence item — it is currently only covered by the ≥ 3.51.3 version gate, nowhere in the test corpus | §17.3, T04 | `sqlite-fault`, `publication_power.rs` |
| B7 | Cross-platform evidence: rerun the ADR-02 spike kill matrix on Windows and Linux; VFS model already covers software power loss | §17.3 | `adr02-storage-spike` spike harness |
| B8 | Docs: I04 section in `docs/IMPLEMENTATION_STATUS.md`, CHANGELOG entry, ADR-02 status update | §18.3 | docs |

## Explicitly out of scope (later issues)

Projections (`field_writes`, `files_current`, `paths_current` — I05),
`frontiers`/`branches` resolution (I08/I12), `native_changes` scope checks
(I05 — native artifacts stay refused), checkout/Git state (I07+),
`upload_reservations`/quota mechanics (I15), replication protocol (I20+),
backup/export/migration tooling (I18/I19), daemon/IPC (I17), CLI (I10).
Machine/VM power-cut drills remain an open gate recorded in ADR-02; the VFS
model is the software power-loss evidence. Known recorded decisions/deviations:
equivocation quarantine unpublishes only the claimant row — descendants and
their events stay visible until I08 frontier logic decides (matches ADR-02's
literal wording); the owner lock lives inside the space directory
(`<dir>/owner.lock`) rather than as the `spaces/<space-id>.owner.lock` sibling
of blueprint §6.2, which only matters for payload eviction in I39/I40.

## Definition of done for I04

- B1–B6 merged with green CI (fmt, clippy `-D warnings`, workspace tests in all
  three feature configurations, python unittest, cargo-deny).
- T04 evidence: VFS lost-sync/torn-write/disk-full tests + SIGKILL test show no
  partial publication and no ack for an uncommitted batch; every acked batch
  survives reopen complete.
- One owning writer enforced by the OS lock on all CI platforms.
- `docs/IMPLEMENTATION_STATUS.md` records what is done and what stays open
  (B7 cross-platform spike, machine power-cut).

Commit policy: checkpoint commits on the branch, English messages, DCO
sign-off, Mistral Vibe co-author trailer. Push only to
`i04-atomic-sqlite-publication`; no merge/PR without maintainer approval.
