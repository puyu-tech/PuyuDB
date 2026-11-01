# PuyuDB canonical profile v1

**Status: frozen at I03 T1 (2026-10-27) as a specification; nothing in this
document is implemented or released yet.** No storage, protocol, SQLite schema,
package, or CLI surface follows from it. Implementation lands in I03 T4 (Rust
codec in `puyudb-core`) and T2 (golden vectors plus independent Python
validator); the DT production binding is promoted in T3.

Normative sources: blueprint §5.2 (identities and canonical objects), §5.4
(lazy text seeding), §5.6 (flat paths), §22.1 (normative record appendix) and
§22.2 (cross-cutting clarifications), with the engine substitution recorded in
[ADR-01 amendment 1](../adr/01-native-text-evidence.md#amendment-1-2026-10-27-engine-bound-canonical-profile-for-diamond-types).
Where this document is silent, the blueprint governs. Where the blueprint still
says Automerge (`am-text-v1`, `puyudb/am-seed-actor/v1`, `puyudb/am-actor/v1`),
this document supersedes it for v1; those names are reserved and unused.

Any change to a frozen layout, derivation, tag, or limit requires a new profile
version plus regenerated fixtures. A patch release cannot repurpose a tag,
separator, or presentation prefix.

## 1. Conventions

- `uint` is a canonical CBOR unsigned integer. `bytesN` is a CBOR byte string of
  exactly `N` bytes. `text` is a CBOR text string holding valid UTF-8. `null` is
  the CBOR null. `array` is a CBOR definite-length array.
- Every record is a fixed array whose first element is the profile version
  `uint` = `1`. Records are addressed positionally; extra or missing elements are
  an encoding error.
- All counts, lengths, offsets, sequences and indices are unsigned 64-bit unless
  a narrower bound is listed in §10. Rust `usize` is never a persistent width.
- Byte order in this document: `||` is concatenation; `0x00` is one zero byte;
  "sorted" means ascending unsigned lexicographic on the byte string unless a
  record says otherwise.

### 1.1 Canonical CBOR subset

Records use RFC 8949 core deterministic encoding, restricted further:

1. Definite lengths only; no indefinite strings, arrays, or maps.
2. Shortest integer encoding; no 8/16/32-bit form for a value that fits narrower.
3. No floating point, no big numbers, no epoch/simple values beyond `null`.
4. No CBOR tag items (`#6.x`). The `uint` tag that starts an operation array in
   §7 is an array element, not a CBOR tag item.
5. No maps in v1 records. Duplicate-map-key rejection is therefore vacuous and
   not a substitute for positional length checks.
6. Text strings must be valid UTF-8; no normalization, case folding, or Unicode
   escape rewriting is applied. Byte strings are raw.
7. Unknown trailing bytes after a record are an encoding error; a decoder must
   require full input exhaustion.

The encoder is hand-rolled for these fixed schemas (see the I03 plan); a generic
serializer's "deterministic mode" is not accepted as evidence. Decoders bound
lengths before allocating and reject non-canonical (over-long, non-minimal)
forms rather than normalizing them.

## 2. Typed identifiers

| Type | Width | Use |
| --- | --- | --- |
| `SpaceId` | 16 bytes | Sharing, authorization, replication unit |
| `BranchId` | 16 bytes | Branch view inside one space |
| `FileId` | 16 bytes | Stable logical file identity |
| `RepositoryId` | 16 bytes | Repository binding inside one space; **not** a `SpaceId` and never derived from a Git remote URL |
| `WriterEpoch` | 16 bytes | One writer stream incarnation in one branch |
| `FileGenerationNonce` | 16 bytes | Epoch descriptor discriminator |
| `EpochId` | 32 bytes | Content lineage digest |
| `BatchId` | 32 bytes | Immutable batch digest |
| `ArtifactId` | 32 bytes | Artifact manifest digest |
| `ChunkId` | 32 bytes | Raw chunk digest |
| `VersionId` | 32 bytes | Frontier digest |
| `PublicKey` | 32 bytes | Ed25519 verifying key |
| `GitOid` | record | Typed Git object identity (§8.1) |

Random-width IDs (space, branch, file, repository, writer epoch, generation
nonce) are generated from the operating system CSPRNG. They are never derived, and never
copied between independently writable devices.

### 2.1 Domain separators

Each digest is `SHA-256(separator || 0x00 || fields...)` where `separator` is the
exact ASCII byte string below followed by one NUL byte.

| Purpose | Separator | Fields after NUL |
| --- | --- | --- |
| Batch body | `puyudb/batch/v1` | canonical batch body bytes (§4) |
| Batch signature preimage | `puyudb/signature/v1` | `space16`, `batch32` |
| Frontier / version | `puyudb/frontier/v1` | canonical frontier bytes (§5) |
| Raw chunk | `puyudb/chunk/v1` | raw chunk bytes |
| Artifact manifest | `puyudb/artifact/v1` | canonical manifest bytes (§6) |
| Raw content | `puyudb/content/v1` | `kind_text`, `0x00`, raw payload |
| Epoch descriptor | `puyudb/epoch/v1` | canonical epoch descriptor (§8.2) |
| Conflict identity | `puyudb/conflict/v1` | canonical conflict record (§9) |
| Branch descriptor | `puyudb/branch-descriptor/v1` | canonical descriptor body (§8.3) |
| Branch descriptor signature | `puyudb/branch-signature/v1` | canonical descriptor body |
| Semantic manifest | `puyudb/semantic-manifest/v1` | canonical manifest bytes (§7.2) |
| DT seed actor name | `puyudb/dt-seed-actor/v1` | `epoch32` (§12.2) |
| DT writer actor name | `puyudb/dt-actor/v1` | `public_key32`, `branch16`, `writer_epoch16`, `epoch32` (§12.3) |

Reserved and unused in v1: `puyudb/am-seed-actor/v1`, `puyudb/am-actor/v1`,
`puyudb/loro-*`. A v1 decoder must not accept a `seed_profile` or actor name
derived from them.

A separator is never reused for a second meaning, and a field list is never
reordered. Domain separation is the only defense that keeps two record kinds with
equal field widths from colliding in one hash space.

## 3. Textual presentation

IDs are displayed as `prefix` + `:` + lowercase hexadecimal of the exact width
(32 or 64 hex characters). Presentation is not identity: parsers reject
uppercase, missing/extra characters, other alphabets, and unknown prefixes, and
no record stores a presented string.

| Prefix | Type | Example width |
| --- | --- | --- |
| `space1` | `SpaceId` | 32 |
| `branch1` | `BranchId` | 32 |
| `file1` | `FileId` | 32 |
| `writer1` | `WriterEpoch` | 32 |
| `repo1` | `RepositoryId` | 32 |
| `nonce1` | `FileGenerationNonce` | 32 |
| `epoch1` | `EpochId` | 64 |
| `batch1` | `BatchId` | 64 |
| `artifact1` | `ArtifactId` | 64 |
| `chunk1` | `ChunkId` | 64 |
| `version1` | `VersionId` | 64 |
| `conflict1` | `ConflictId` | 64 |
| `gitoid1` | `GitOid` | `sha1:` + 40 / `sha256:` + 64 |

Chunk IDs are an internal addressing detail; presenting them is allowed but they
are not a public content capability.

## 4. Batch

### 4.1 Body

```text
BatchBodyV1 = [
  1,
  space: bytes16,
  branch: bytes16,
  public_key: bytes32,
  writer_epoch: bytes16,
  sequence: uint,
  previous: bytes32 / null,
  parents: array(bytes32),        ; sorted unique, minimal frontier, <= 128
  lamport: uint,
  native_artifact_ids: array(bytes32),   ; <= 4096, order is the batch's edit order
  metadata_artifact: bytes32
]
```

`BatchId = SHA256("puyudb/batch/v1" || 0x00 || canonical_body_bytes)`.
Signatures are not part of the hashed body.

The body's canonical bytes are at most `MAX_BATCH_BODY_BYTES` (§10). Larger
logical bodies are not a v1 concept; text edits live in artifacts, not in the
body.

### 4.2 Signature

```text
BatchSignatureV1 = [1, public_key: bytes32, signature: bytes64]
```

The Ed25519 signature is over `"puyudb/signature/v1" || 0x00 || space16 || batch32`
(§2.1) and is verified with strict verification (reject low-order keys and
non-canonical `S`). A v1 batch carries exactly one signature, whose `public_key`
must equal the body's `public_key`. The binding is enforced by one function,
`verify_batch`, which compares the two keys **before** verifying the signature;
verifying the signature and the body independently and then trusting the body's
key would let one signer attest a batch that names a different writer. Multiple signers and cosignatures are a
later profile, not an implicit extension of this one.

The preimage deliberately binds only the space and the body digest: branch,
sequence, parents, lamport and artifacts are bound transitively through the body
hash.

### 4.3 Validation order

A receiver validates in this order and stops at the first failure:

1. Framing and size caps (§10) before allocation.
2. Canonical encoding (§1.1) and exact record shape.
3. `BatchId` recomputation; reject on mismatch.
4. Signature verification.
5. Writer-stream and dependency checks (§13) against the declared parent cut.
6. Native artifact and metadata scope inspection (§12.5, §7) against that cut.

`validate_record` is not admission. Before a batch may be applied, a caller
must additionally: bound the input buffer before decoding, recompute `BatchId`,
call `verify_batch` (key equality and strict verification together), check the
parent closure and minimal frontier, check `lamport` against the actual parent
lamports, check the `(stream, sequence)` ledger for equivocation, check native
scope against the declared cut, and track hydration separately from the
manifest. A missing dependency stays pending; `Stored` implies neither `Applied`
nor `Hydrated`.

No semantic effect is produced by a batch that fails any step. A missing
dependency is a pending/Want condition (`missing_dependency`), not a permanent
validation error, and never causes a provisional application.

## 5. Frontier and version

```text
FrontierV1 = [1, space: bytes16, heads: array(bytes32)]   ; sorted unique, <= 65536
VersionId  = SHA256("puyudb/frontier/v1" || 0x00 || canonical_frontier_bytes)
```

Normalization is pure and order-independent, and the three steps below are a
**producer-side construction**, not a decoder repair:

1. Deduplicate heads.
2. Drop every head that is an ancestor of another head (ancestor dominance
   computed over the batch DAG, not over arrival order or lamport values).
3. Sort ascending by unsigned lexicographic bytes.

A decoder never runs these steps on received bytes: it validates that the list is
already deduplicated and sorted, and rejects a redundant head wherever it can
prove dominance from the history it holds. Only a normalized frontier may be
hashed, stored, transmitted, or compared. An
unnormalized or redundant head list is `invalid_encoding`, never silently fixed,
because silently normalizing would let two replicas disagree on a version's
identity.

`VersionId` resolution requires the space and the canonical frontier body; a
missing head is `VersionUnavailable` (`content_unavailable` on the wire), never an
approximate latest view. Large frontiers are transported as chunks of the same
canonical body (§6), so a `VersionId` does not depend on chunking.

## 6. Artifacts and chunking

### 6.1 Manifest

```text
ArtifactManifestV1 = [1, kind: text, raw_length: uint, raw_digest: bytes32,
                      chunk_ids: array(bytes32)]
ArtifactId = SHA256("puyudb/artifact/v1" || 0x00 || canonical_manifest_bytes)
ChunkId    = SHA256("puyudb/chunk/v1"   || 0x00 || raw_chunk_bytes)
RawDigest  = SHA256("puyudb/content/v1" || 0x00 || kind_text || 0x00 || raw_payload_bytes)
```

- `CHUNK_RAW_BYTES` is 262,144 (256 KiB). Every chunk except the final one is
  exactly that long; the final chunk is `1..=262,144` bytes. A zero-length
  artifact has an empty chunk list.
- `MAX_ARTIFACT_CHUNKS` is 4,096, so the profile cap of 1 GiB is
  `4,096 × 256 KiB`. The 1 GiB figure is a declared profile constant, not a quota
  system; a deployment may enforce a smaller configured limit downward.
- `kind` is assigned ASCII, at most 64 bytes (`MAX_KIND_TEXT_BYTES`).
- A mismatch between `raw_length` and the chunk count is reported as
  `hash_mismatch`, but that code here means only "the count disagrees"; the raw
  digest is **not** compared at this stage. Comparing it, and verifying that
  resident chunks hash to the listed IDs, happens at hydration.
- Manifest integrity is checked before any payload work: `ArtifactId` must match
  the ID committed by signed history, and it binds kind, length, raw digest and
  every chunk ID. Hydration then verifies each chunk, streams the full
  kind-specific raw digest, and only then reports the bytes verified.
- An unhydrated artifact therefore has an authenticated manifest and **no** claim
  that its bytes were locally verified.
- Compression wrappers never change chunk, content, or manifest identity; a
  compressed transport encoding is outside these digests.

### 6.2 Kind registry

| Kind text | Status | Payload |
| --- | --- | --- |
| `native-change-v1` | assigned | §6.3 |
| `metadata-v1` | assigned | §7 |
| `text-baseline-v1` | assigned | raw UTF-8 baseline bytes |
| `opaque-baseline-v1` | assigned | raw opaque bytes |
| `symlink-target-v1` | assigned | raw target bytes |
| `branch-descriptor-v1` | assigned | §8.3 body (catalog archive use is I39) |
| `named-snapshot-v1` | reserved | I39 |
| `archive-coverage-v1` | reserved | I39 |

Unassigned kind text is `unsupported_capability`. Reserved kinds cannot be
accepted by a decoder that does not implement their record.

### 6.3 Native text artifact

```text
NativeArtifactV1 = [1, epoch: bytes32, profile: text, native_bytes: bytes]
```

`profile` is the frozen engine profile string; v1 uses `dt-text-v1` (§12).
`native_bytes` are the producer's exact exported bytes, including the empty case.

The payload contains **no declared native dependency list**. For DT, the wire
format carries its own parents, so a second declared copy would be a second
source of truth that can disagree; the dependency cut is the enclosing batch's
parent cut (§4.1 `parents`), or the epoch descriptor for a seed artifact (§8.2).
Scope inspection maps the artifact's own parent references back into that cut and
rejects anything outside it.

One native artifact per `(epoch, batch)`; all of that batch's edits to one epoch
are grouped in it, in the batch's edit order.

The blueprint's AM row (`[1, epoch32, "am-text-v1", native_hash_bytes,
native_dependency_hashes_array, native_change_bytes]`) differs from this record in
**two** fields, both deliberate:

- `native_dependency_hashes_array` is absent because DT's wire already carries its
  own parents and the outer cut is the authoritative declaration; a second copy
  could disagree with it.
- `native_hash_bytes` is absent because DT has no single change hash: portable
  identity is `(agent name, operation index)`, and identity of the artifact as a
  whole comes from the manifest over the exact exported bytes (§6.4). A per-change
  hash would be a PuyuDB invention, not a native identifier.

Both deviations are AM-specific shape, not dropped information.

### 6.4 Native artifact identity versus re-encoding

Native bytes are producer-owned. Two replicas may hold logically equivalent
histories whose DT encodings differ (agent-name serialization order leaks local
import order; see [the DT assessment](../validation/I02-diamond-types-assessment.md#valid-same-cut-updates-and-serialization-order)).
Consequences that are frozen here:

- Identity is over the exact bytes a producer exported. A replica must never
  re-encode a history and claim the producer's `ArtifactId`.
- Equivalent-but-different encodings are distinct artifacts with distinct IDs.
  Both may be valid; neither is a canonical form of the other.
- Admission validates the *logical* content (agent names, sequences, parents,
  operation ranges and content) against the declared cut. Byte reconstruction is
  a strict negative-input guard only, never the general admission rule, because
  it falsely rejects valid same-cut updates assembled in the opposite import
  order.
- Re-importing a known artifact adds no authorship and creates no new producer
  association.

## 7. Metadata artifact

```text
MetadataArtifactV1 = [1, operations: array, context_assertions: array]
```

Operation order inside `operations` is semantic (it is the batch's order of
application) and is never sorted. At most `MAX_METADATA_OPERATIONS` operations and
`MAX_CONTEXT_ASSERTIONS` assertions per artifact.

A field write's identity is `(batch32, operation_index)` where `operation_index`
is the zero-based position in `operations`, encoded as a `uint`. This pair is the
unit of supersession, conflict participation and provenance.

### 7.1 Operation registry

| Tag | Array | Semantic rule |
| ---: | --- | --- |
| 0 | `[0, repository16, file16, path_text, mode_tag, epoch_descriptor]` | Create a fresh `FileId`; creator operation is in the parent closure of later use |
| 1 | `[1, file16, path_text]` | Causal path assignment |
| 2 | `[2, file16, live_tag]` | Liveness: 0 deleted, 1 alive; no implicit resurrection |
| 3 | `[3, file16, epoch_descriptor]` | Select/reset content lineage, retaining old epochs |
| 4 | `[4, file16, epoch32, opaque_artifact32]` | Opaque value assignment for a declared opaque epoch |
| 5 | `[5, context_schema_tag, event_id16, event_bytes]` | Bounded generic context event |
| 6 | `[6, repository16, gitoid, version32, relation_tag]` | Git association assertion, not a Git mutation |
| 7 | `[7, file16, mode_tag]` | Mode assignment |
| 8 | `[8, conflict32, resolution_tag, resolution_payload]` | Explicit conflict resolution (§9) |
| 9 | `[9, source_batches: array(bytes32), relation_tag, detail_artifact: bytes32 / null]` | Provenance assertion; never rewrites native IDs |

Tags 10 and above are unassigned; a v1 decoder rejects them as
`unsupported_capability`. `relation_tag` in tags 6 and 9 is the one field whose
values are **deliberately opaque in v1**: the registry is owned by I05 (Git and
filesystem association) and I07 (import provenance), so a decoder validates the
uint width only and must not interpret or reject a value. Every other registry in
this document has assigned values and rejects the rest. Mode tags: 0 regular non-executable, 1 regular
executable, 2 symlink entry — no other value. Epoch kind tags: 0 text UTF-8,
1 opaque bytes, 2 inert symlink target. Context schema tags: 0 subject
assertion, 1 conversation/thread descriptor, 2 message revision, 3 comment/reply,
4 session lifecycle, 5 tool/execution event, 6 review/verdict, 7 derived-context
manifest. An incompatible required field set needs a new tag, never a widened
existing one.

A path assignment is never encoded as an overwrite, and a path assignment must
name a `FileId`; two files with one path are a collision set, not a resolution.

### 7.2 Semantic manifest

A derived projection record used to compare two replicas on one version. It is
not stored inside a batch; a deployment may persist it, and the hash is the
comparison unit in T02/T13 fixtures.

```text
FileEntryV1 = [1, file: bytes16,
               live: array(uint),          ; maximal liveness alternatives, 0/1
               paths: array(text),         ; maximal path alternatives
               epochs: array(bytes32),     ; maximal epoch alternatives
               contents: array(bytes32),   ; maximal opaque content alternatives
               modes: array(uint),         ; maximal mode alternatives
               conflicts: array(bytes32)]  ; ConflictIds at this cut
SemanticManifestV1 = [1, space: bytes16, entries: array(FileEntryV1)]
SemanticManifestId = SHA256("puyudb/semantic-manifest/v1" || 0x00 || canonical_bytes)
```

- `entries` is sorted ascending by raw `FileId` bytes; every inner array is
  sorted ascending and deduplicated (paths by raw UTF-8 bytes, not locale).
- "Maximal" means the surviving alternatives at the viewed cut after causal
  supersession — the same set a reader would see. Deleted files keep their
  entry, and their content stays listed.
- Arrival order, cache contents, event cursors, chunk boundaries, native
  snapshot bytes and native re-encoding must never change this hash. Two
  replicas of one closed cut that disagree on it have diverged, and that is a
  release-blocking bug, not a display difference.
- Register maxima and conflict derivation are computed by I05; this record only
  fixes their canonical output shape and ordering.
- `modes` is an addition to §22.1's manifest description (which lists live/path/
  epoch alternatives, content IDs and conflict IDs). It is required: without it a
  mode-only divergence between two replicas would hash identically, and mode is a
  destructive/type decision (§5.6), not presentation.
- Every `paths` entry must satisfy §11. A manifest carrying a path that no
  path operation could have produced is an encoding error, not a
  materialization problem.

## 8. Epoch, Git object and branch descriptor

### 8.1 Git object

```text
GitOidV1 = [1, algorithm_tag, raw_digest]   ; 1 = SHA-1/20 bytes, 2 = SHA-256/32 bytes
```

Other algorithms are unsupported until assigned. The typed record keeps
algorithm and digest separate so a 20-byte and a 32-byte object cannot collide or
be compared.

### 8.2 Epoch descriptor

```text
EpochDescriptorV1 = [
  1,
  file: bytes16,
  generation_nonce: bytes16,
  kind_tag,                  ; 0 text, 1 opaque, 2 symlink target
  baseline_artifact: bytes32,
  seed_profile: text / null,
  git_oid: GitOidV1 / null
]
EpochId = SHA256("puyudb/epoch/v1" || 0x00 || canonical_descriptor_bytes)
```

- `kind_tag` 0 requires `seed_profile = "dt-text-v1"`; kinds 1 and 2 require
  `null`, because a non-text epoch has no native text seed.
- `baseline_artifact` is the content artifact (§6.2) holding the baseline bytes
  for the declared kind; it is independently retained and integrity checked, so
  a Git object that `git gc` removes cannot break history.
- A baseline text artifact of length 0 is legitimate and produces an empty seed
  (§12.4).
- Changing encoding, mode-independent whole-file replacement, or restoring a
  writable backup selects a new epoch through the same MV register; existing
  epochs stay and concurrent old-epoch edits are reported, never dropped.

### 8.3 Branch descriptor

A signed space-catalog record, outside the mutable file metadata namespace. It
records that a branch existed and where it started; it never changes history
membership.

```text
BranchDescriptorV1 = [1, space: bytes16, branch: bytes16,
                      initial_version: bytes32, creator_key: bytes32]
BranchDescriptorId = SHA256("puyudb/branch-descriptor/v1" || 0x00 || body_bytes)
BranchDescriptorEnvelopeV1 = [1, body: bytes, signature: bytes64]
```

The Ed25519 signature is over `"puyudb/branch-signature/v1" || 0x00 ||
canonical_body_bytes` (§2.1), which already binds space and creator key.

- Registration is idempotent by `(BranchId, BranchDescriptorId)`.
- Two different descriptors for one `BranchId` are **rejected**, not resolved by
  arrival order. A fork cannot rewrite its captured initial version.
- No label, display name, or rename field is signed, so renaming a branch cannot
  mint a new identity or a conflicting descriptor.
- Advancing a branch joins accepted batches declared for it with its retained
  frontier; the descriptor is not rewritten.
- Authorization is control-plane policy (I24). A history export includes
  descriptors and never includes permissions.

## 9. Conflicts

```text
ConflictV1 = [1, kind_tag, file: bytes16, epochs: array(bytes32),
              participants: array([batch32, operation_index: uint])]
ConflictId = SHA256("puyudb/conflict/v1" || 0x00 || canonical_conflict_bytes)
```

- `kind_tag`: 0 register alternative, 1 `delete_edit`, 2 `epoch_edit`.
- `epochs` holds the distinct epoch IDs the conflict concerns, sorted unique;
  empty for kind 0.
- `participants` holds the maximal concurrent writes that created the conflict,
  sorted ascending by `(batch32, operation_index)`. Sorted order makes the ID
  independent of delivery order.
- Conflicts are recomputed from history, never persisted as arrival-dependent
  mutable flags.

Resolution tags (`§7.1` tag 8 payload):

| Tag | Meaning | `resolution_payload` | Required same-batch effect |
| ---: | --- | --- | --- |
| 0 | Retain deletion | `null` | New `Deleted` liveness assignment |
| 1 | Restore edited content | `null` | New `Alive` assignment observing the edit |
| 2 | Retain selected epoch | `epoch32` | New selection of that epoch |
| 3 | Incorporate old-epoch work | `[new_epoch32, provenance_artifact32]` | New explicit epoch write plus the provenance operation |
| 4 | Choose an alternative | `[field_tag, [batch32, operation_index]]` with field tags 0 path, 1 content value, 2 mode | That field is written again with the chosen alternative |

A resolution names a conflict visible at its parent cut and must include every
named participant in its parents. Acceptance-only records are insufficient.
Opposing concurrent resolutions therefore remain alternatives. A later concurrent
edit creates a new conflict; an old approval never covers it. Resolved
participant sets stay auditable; incorporation never relabels old native
operations.

## 10. Fixed limits

Every value below is a v1 declared constant: chosen, not derived from a
measurement or a deployment quota. Each is checked before allocation.

| Constant | Value | Applies to |
| --- | ---: | --- |
| `PROFILE_VERSION` | 1 | every record's first element |
| `MAX_BATCH_BODY_BYTES` | 1,048,576 | canonical body bytes; implied by the parent and artifact caps, kept as a decode guard |
| `MAX_BATCH_PARENTS` | 128 | `parents` |
| `MAX_BATCH_NATIVE_ARTIFACTS` | 4,096 | `native_artifact_ids` |
| `MAX_FRONTIER_HEADS` | 65,536 | `heads` |
| `MAX_ARTIFACT_CHUNKS` | 4,096 | `chunk_ids` |
| `CHUNK_RAW_BYTES` | 262,144 | every non-final chunk |
| `MAX_ARTIFACT_RAW_BYTES` | 1,073,741,824 | profile cap, configurable downward |
| `MAX_KIND_TEXT_BYTES` | 64 | artifact kind, profile strings |
| `MAX_METADATA_OPERATIONS` | 65,536 | `operations` |
| `MAX_CONTEXT_ASSERTIONS` | 4,096 | `context_assertions` |
| `MAX_CONTEXT_EVENT_BYTES` | 65,536 | operation tag 5 `event_bytes` |
| `MAX_PATH_BYTES` | 4,096 | one logical path |
| `MAX_PATH_COMPONENT_BYTES` | 255 | one path component |
| `MAX_TEXT_EPOCH_BYTES` | 8,388,608 | `text-baseline-v1` manifests; the resulting text of a kind-0 epoch |
| `MAX_OPERATION_INDEX` | 65,535 | write identity index |

## 11. Paths

```text
path      = component ( "/" component )*
component = 1*( any UTF-8 byte except 0x00 and "/" )
```

A path is valid only if all of the following hold:

1. Valid UTF-8, at most `MAX_PATH_BYTES` bytes; every component at most
   `MAX_PATH_COMPONENT_BYTES` bytes.
2. Slash-separated, non-empty, no leading or trailing slash, no empty component
   (`a//b`), no `"."` or `".."` component, no `0x00` byte.
3. No component equal to `.git` or `.puyu` (reserved control segments). The check
   is byte-exact and case-sensitive.
4. No Unicode normalization and no case folding. `A` and `a`, `é` and `e` +
   U+0301, and NFC/NFD forms are distinct logical paths. Case-insensitive,
   normalization-insensitive, Windows-reserved-name, trailing-dot/space and
   length collisions are materialization-profile conflicts, not encoding errors,
   and never silently rename or drop a file.
5. A path that cannot be represented losslessly is reported and quarantined; no
   lossy substitution and no fabricated hidden filename.

A directory is implicit; a directory rename expands to the descendant `FileId`
set visible at its base version and updates those full paths atomically. A
concurrently created descendant keeps the old prefix; a concurrently renamed
member keeps alternatives. A move exceeding `MAX_METADATA_OPERATIONS` returns a
limit error and a preview; split jobs are explicitly non-atomic.

## 12. Native text profile `dt-text-v1`

### 12.1 Binding

`dt-text-v1` is bound to diamond-types Git revision
`89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922` (manifest 2.0.0, unpublished) with the
reviewed patch set in `experiments/diamond-types-interfaces/` promoted as the
production binding in I03 T3. Dialect: `DMNDTYPS` protocol version 0, no LZ4, no
other feature profile. Any other revision, dialect version, or feature set is a
different profile and needs a new profile string.

Portable identity in DT is an **agent name plus sequence**. The local numeric
`AgentId` is replica-local interning and is never persisted, transmitted, hashed,
or compared as identity; only names and sequences cross a boundary.

### 12.2 Seed actor name

```text
seed_actor_name(epoch32) =
  "s:" || base64url_unpadded_43(SHA256("puyudb/dt-seed-actor/v1" || 0x00 || epoch32))
```

### 12.3 Writer actor name

```text
writer_actor_name(public_key32, branch16, writer_epoch16, epoch32) =
  "w:" || base64url_unpadded_43(
    SHA256("puyudb/dt-actor/v1" || 0x00 ||
           public_key32 || branch16 || writer_epoch16 || epoch32))
```

`base64url_unpadded_43` is RFC 4648 §5 alphabet (`A-Z a-z 0-9 - _`), no padding,
exactly 43 characters for 32 digest bytes.

Both names are therefore exactly 45 ASCII bytes, inside DT's under-50-byte limit,
distinct from `ROOT`, and separated by the leading `s:`/`w:` tag: a seed and a
writer can never occupy one namespace, matching the blueprint's `0x00`/`0x01`
namespace tag rule in textual form. Names are ASCII-only, so DT's byte-length and
UTF-8 constraints cannot disagree.

The name is a binding digest, not a lossless encoding and not authentication. The
full tuple remains in the outer records: `public_key`, `branch` and
`writer_epoch` in the batch body (§4.1), `epoch` in the native artifact payload
and epoch descriptor (§6.3, §8.2). Deriving a name from the full context is
mandatory: omitting any field would let two write identities intern as one agent.

Two distinct tuples mapping to one name would require a SHA-256 collision; if one
is ever observed, the profile is broken — quarantine the evidence and stop, never
merge the writers and never renumber retained native IDs.

### 12.4 Epoch-owned seed and the empty case

The seed is an **epoch-owned baseline dependency**, never owned by the first
editing batch that arrives.

- The seed artifact is `NativeArtifactV1` with `profile = "dt-text-v1"`, no
  enclosing batch, and its `epoch32` in the payload. Its bytes are the native
  export of the seeded document produced by the seed actor's first native
  operation. DT operation indices start at 0 per agent and a remote frontier is
  `(name, next_index)`, so a seeded document's native frontier is
  `(seed name, 1)`; a native agent index is never the outer batch `sequence` of
  §4.1.
- An empty baseline yields a **zero-length** native payload: no sentinel insert,
  no delete, no dummy operation, and no hidden eager seeding. The artifact is
  still well defined and epoch-specific, because its payload binds `epoch32`
  rather than only the native bytes; identical empty payloads across different
  epochs therefore never share an `ArtifactId`.
- A non-empty seed is validated by applying its native bytes and comparing the
  resulting text to the epoch descriptor's baseline content artifact; a mismatch
  is `hash_mismatch`, not a repaired baseline.
- Raw seed bytes are persisted once, transferred when needed, and reused for
  existing lineages. A newer library version never regenerates a retained
  lineage; format upgrades preserve the original bytes and identity.
- Cross-process reproducibility of these bytes is an I02-measured property and a
  T2 golden-fixture obligation: the same epoch must produce the same seed bytes
  and the same `ArtifactId` on every platform.

### 12.5 Native scope and publication binding

- Non-seed native changes must use the enclosing writer's derived actor name, the
  declared epoch's root text object, supported insert/delete operations only, and
  native actor-sequence continuity.
- Native operations introduced by one artifact are that artifact's span. Their
  producer is the enclosing outer batch; the association is immutable and
  derived, not a stored duplicate. Re-introducing a known operation adds no
  authorship and is not a new producer.
- The dependency cut is the enclosing batch's parent cut. A native change whose
  own parents fall outside that cut is `invalid_native_scope`, never silently
  widened by adding current heads.
- There is no change message or timestamp field in the DT wire format. The outer
  binding is therefore `(agent name, sequence range, epoch)` plus the signed
  batch; the blueprint's frozen ASCII AM change-message rule does not apply and is
  not emulated with an invented field.
- The accepted cursor policy of the [interface spike](../validation/I02-dt-interface-spike.md)
  is identity-bound: stable comment anchors store a native insertion identity and
  affinity, and one tested post-deletion gap still resolves differently by import
  order. That limitation is an accepted I02 finding, not fixed by this profile;
  no replacement merger and no actor-order override is permitted.

### 12.6 Explicit non-guarantees

This profile does not claim a canonical native encoding, a hardened hostile-input
decoder, bounded worst-case decode work, upstream acceptance of the patch set,
anchor stability for deleted gaps, or semantic (code-level) merge correctness.
Native convergence is reused for text only; PuyuDB owns batches, identity,
metadata, publication and durability.

## 13. Writer streams, sequence, lamport, ancestry

- A stream is `(public_key, branch, writer_epoch)` and is **branch-specific**.
  Never copy a stream between independently writable devices or forked handles;
  restoring a writable backup allocates a fresh `writer_epoch`.
- `sequence` starts at 1 and increases by exactly 1 per stream, with checked
  arithmetic: a stream whose next sequence would exceed `u64::MAX` is
  `invalid_encoding`, not a wrap back to 0. `previous` is the
  stream's preceding batch and must be present in the parent closure (not
  necessarily as a direct parent). A batch with `previous = null` is the first of
  its stream and has `sequence = 1`; any later batch has `sequence >= 2`.
- **A stream's first batch may declare parents.** It is not a genesis batch in the
  DAG sense: it declares the intent-base cut (§13.1) as its `parents`, which is
  non-empty for every fork, every branch start from a captured frontier (§5.3) and
  every writer-epoch restart after a `causal_fork`. Parent-free batches exist only
  for the very first batch of a space.
- `lamport = 1 + max(parent.lamport)` with **checked** arithmetic: a step that
  would overflow `u64` is `invalid_encoding`, never a wrap and never a panic, in
  debug and release alike. Therefore a batch
  with no parents has `lamport = 1`, and any batch with parents has
  `lamport >= 2`. Both directions are locally checkable without history and are
  enforced by every decoder. Wall-clock time is optional untrusted display
  metadata and never determines causality.
- `parents` are sorted unique, minimal (no parent is an ancestor of another),
  at most 128. A larger frontier is reduced by explicit no-content join batches;
  joins preserve ancestry and do not create content.
- Duplicate `(stream, sequence)` with a different `BatchId` is equivocation:
  quarantine the conflicting evidence and suspend the stream. Never select a
  last-arrival winner and never rewrite an already admitted immutable batch.
  Operator recovery for inconsistent admission sets is outside the equal-valid-
  history convergence guarantee.

### 13.1 Intent base and publication ancestry (`causal_fork`)

A mutation's intent base fixes its observed positions, field observations and the
only native dependencies it may use. Publication parents must represent exactly
that closed cut, except that a no-content join may represent the same cut with a
different parent count.

If the selected stream's previous batch is not in that cut, allocate a fresh
`writer_epoch`, start at `sequence = 1`, and return `causal_fork = true` in the
receipt. Do not silently add current branch heads, reuse a native actor, or
reinterpret metadata against a newer cut. Strict `expected_version` still checks
the current branch state at publication, and concurrent history joins the visible
frontier after publication, not the mutation's observations. A retry after an
unknown outcome preserves the originally allocated stream and result.

## 14. Receipts and error codes

Receipt states are exactly:

| State | Meaning |
| --- | --- |
| `Stored` | Bytes staged and verified against their manifest; no history claim |
| `Applied` | Complete validated history plus every required native/baseline artifact, and authenticated manifests for optional opaque bytes |
| `Hydrated` | `Applied`, and every content artifact reachable through the ancestor closure is resident and raw-hash verified |

`Stored` never implies `Applied`; `Applied` never implies `Hydrated`; no receipt
implies filesystem materialization. Persist expected-but-absent chunks
intentionally, so recovery can distinguish unhydrated content from loss of
previously resident authoritative content. Loss of required text/native data is
corruption; loss of resident opaque bytes is degraded integrity, not a
reclassification as intentional omission.

Writer-stream violations map to the same stable codes: `writer_equivocation` for
one `(stream, sequence)` claimed by two different batches, and `invalid_encoding`
for a changed stream context, a skipped sequence, a wrong `previous`, or a
non-advancing lamport. A receiver must not invent a second spelling.

Failure codes used by this profile are the assigned stable strings
`invalid_encoding`, `unsupported_version`, `unsupported_capability`,
`hash_mismatch`, `invalid_signature`, `writer_equivocation`, `missing_dependency`,
`invalid_native_scope`, `invalid_operation`, `precondition_failed`,
`content_unavailable`, `corruption`. Mapping into transport and API error shapes
is I20/I22 work; public code strings are never parsed as their text.

## 15. Conformance obligations

Before I03 can close, all of the following exist in the repository:

1. Golden vectors under `tests/fixtures/canonical-v1/`: canonical hex bodies plus
   expected `BatchId`, `VersionId`, `EpochId`, `ArtifactId`, `ChunkId`,
   `RawDigest`, `ConflictId`, `SemanticManifestId`, plus one Ed25519
   signing/verification vector per signature preimage (§4.2, §8.3).
2. An independently written validator
   (`scripts/i03_canonical_vectors.py` with `scripts/test_i03_canonical_vectors.py`)
   that reaches the same bytes and digests from the frozen layouts alone, wired
   into CI. Agreement is evidence about the specification, not about DT.
3. A negative corpus: truncated and trailing bytes, non-minimal integers,
   indefinite lengths, wrong widths, unknown tags and versions, duplicate or
   unsorted parents, non-minimal or unsorted frontier heads, oversized
   `raw_length` before allocation, tampered manifests, unknown kind text,
   unknown epoch kind, path-rule violations, and a wrong-key signature. Record-level
   negatives are fixtures; `(stream, sequence)` equivocation needs two bodies and
   history, so it is covered by the protocol/stream layer and its unit tests, not
   by the single-record corpus.
4. N/N-1 behaviour: a decoder for profile v1 rejects unknown versions and unknown
   values of every **assigned** registry, and does not ignore unknown required
   fields. The single deliberately opaque field is `relation_tag` (§7.1).

Fixture provenance rules: vectors are generated from the frozen layouts, then
independently re-derived by the validator; a vector that either side cannot
re-derive is a specification defect. Seed fixtures additionally record the exact
DT revision, feature profile and producer command so cross-process equality is
checkable.

## 16. Deferred by design

Not frozen here, not implied by this document:

- SQLite schema, publication atomicity and durable receipts (I04, ADR-02).
- Metadata and text projections, MV register evaluation, offsets in UTF-8 bytes
  and UTF-16 conversion, anchor resolution implementation (I05, §5.5).
- Engine publication, owner lock, idempotency records, single-writer interface
  (I06).
- Native span indexes, delete attribution and provenance queries (I16).
- Wire kinds, frame limits, credits, hydration and receipt messages, CDDL (I20).
- Archive coverage/custody records, timestamps and assurance tags (I39).
- Authentication, key registry, roles and admission policy (I24); signature
  validity here is identity and integrity, not permission.
- Any second engine profile. Selecting one requires an ADR amendment and new
  actor/seed derivations; the reserved `am-*`/`loro-*` separators are not it.
- A CDDL grammar for these same layouts. The hand-written fixed-array schemas plus
  the Rust codec and the independent validator are the I03 artifact; CDDL is
  required by blueprint §22.1 before release and is owned by I20. If replication
  later needs generic CDDL-driven decoding, re-evaluate a pinned maintained CBOR
  library instead of extending the hand-rolled subset.
