# SPDX-License-Identifier: Apache-2.0
"""I05 P5 golden fixtures: semantic manifest, conflicts, metadata operations.

Independent validator for the metadata-artifact side of the frozen profile.
Source of truth: ``docs/spec/canonical-profile-v1.md`` — the canonical CBOR
subset (§1.1), the metadata operation registry (§7.1), the semantic manifest
(§7.2), epoch descriptors (§8.2) and conflicts (§9). This module hand-encodes
those layouts and validates strictly, without a CBOR library and without any
Rust code, so agreement with the I05 projection codec is evidence about the
specification rather than about a shared dependency.

Not a production codec or admission path.

CLI: ``python3 scripts/i05_semantic_manifest.py`` verifies the committed
fixtures under ``tests/fixtures/metadata-v1/`` and exits nonzero on any
mismatch, on a resolution lacking its §9 same-batch effect, or on a manifest
whose conflicts disagree with the recomputed cut; ``--write`` regenerates
them from the builders below.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

PROFILE_VERSION = 1

MAX_METADATA_OPERATIONS = 65_536
MAX_CONTEXT_ASSERTIONS = 4_096
MAX_CONTEXT_EVENT_BYTES = 65_536
MAX_PATH_BYTES = 4_096
MAX_PATH_COMPONENT_BYTES = 255
MAX_OPERATION_INDEX = 65_535

TEXT_PROFILE = "dt-text-v1"

LIVE_TAGS = (0, 1)
MODE_TAGS = (0, 1, 2)
EPOCH_KIND_TAGS = (0, 1, 2)
FIELD_TAGS = (0, 1, 2)
CONTEXT_SCHEMA_TAGS = tuple(range(8))
OPERATION_TAGS = tuple(range(10))
RESOLUTION_TAGS = tuple(range(5))
CONFLICT_KIND_TAGS = (0, 1, 2)
GIT_ALGORITHM_WIDTHS = {1: 20, 2: 32}
RESERVED_SEGMENTS = (".git", ".puyu")

DOMAINS = {
    "epoch": b"puyudb/epoch/v1",
    "conflict": b"puyudb/conflict/v1",
    "semantic_manifest": b"puyudb/semantic-manifest/v1",
}

# Record kinds that carry a domain-separated identity, used by the negatives.
RECORD_DOMAINS = {
    "epoch_descriptor": "epoch",
    "conflict": "conflict",
    "semantic_manifest": "semantic_manifest",
}


class ProfileError(ValueError):
    """A record or value violates the frozen profile."""

    def __init__(self, code: str, message: str):
        super().__init__(f"{code}: {message}")
        self.code = code


# --------------------------------------------------------------------------
# Canonical CBOR subset (spec §1.1)
# --------------------------------------------------------------------------

def _head(major: int, count: int) -> bytes:
    if count < 0:
        raise ProfileError("invalid_encoding", "negative head value")
    if count < 24:
        return bytes([(major << 5) | count])
    if count < 1 << 8:
        return bytes([(major << 5) | 24, count])
    if count < 1 << 16:
        return bytes([(major << 5) | 25]) + count.to_bytes(2, "big")
    if count < 1 << 32:
        return bytes([(major << 5) | 26]) + count.to_bytes(4, "big")
    if count < 1 << 64:
        return bytes([(major << 5) | 27]) + count.to_bytes(8, "big")
    raise ProfileError("invalid_encoding", "integer exceeds 64 bits")


def uint(value: int) -> bytes:
    if value < 0 or value >= 1 << 64:
        raise ProfileError("invalid_encoding", "uint out of range")
    return _head(0, value)


def byt(raw: bytes) -> bytes:
    return _head(2, len(raw)) + raw


def text(value: str) -> bytes:
    encoded = value.encode("utf-8")
    return _head(3, len(encoded)) + encoded


def array(items) -> bytes:
    items = list(items)
    return _head(4, len(items)) + b"".join(items)


NULL = b"\xf6"


def _decode_head(data: bytes, offset: int) -> tuple:
    if offset >= len(data):
        raise ProfileError("invalid_encoding", "truncated head")
    initial = data[offset]
    major, info = initial >> 5, initial & 0x1F
    offset += 1
    if info < 24:
        return major, info, offset
    limits = {24: (1, 24), 25: (2, 1 << 8), 26: (4, 1 << 16),
              27: (8, 1 << 32)}
    if info in limits:
        width, minimum = limits[info]
        if offset + width > len(data):
            raise ProfileError("invalid_encoding", "truncated argument")
        value = int.from_bytes(data[offset:offset + width], "big")
        if value < minimum:
            raise ProfileError("invalid_encoding", "non-minimal integer encoding")
        return major, value, offset + width
    if info == 31:
        raise ProfileError("invalid_encoding", "indefinite length is not canonical")
    raise ProfileError("invalid_encoding", "reserved additional information")


def _decode_item(data: bytes, offset: int, depth: int) -> tuple:
    if depth > 16:
        raise ProfileError("invalid_encoding", "nesting too deep")
    major, value, offset = _decode_head(data, offset)
    if major == 0:
        return value, offset
    if major == 1:
        raise ProfileError("invalid_encoding", "negative integers are not in the profile")
    if major in (2, 3):
        if offset + value > len(data):
            raise ProfileError("invalid_encoding", "truncated string")
        raw = data[offset:offset + value]
        if major == 2:
            return raw, offset + value
        try:
            return raw.decode("utf-8"), offset + value
        except UnicodeDecodeError as error:
            raise ProfileError("invalid_encoding",
                               "text string is not valid UTF-8") from error
    if major == 4:
        items = []
        for _ in range(value):
            item, offset = _decode_item(data, offset, depth + 1)
            items.append(item)
        return items, offset
    if major == 5:
        raise ProfileError("invalid_encoding", "maps are not in the profile")
    if major == 6:
        raise ProfileError("invalid_encoding", "CBOR tag items are not in the profile")
    if major == 7:
        if value == 22:
            return None, offset
        raise ProfileError("invalid_encoding",
                           "floats and simple values are not in the profile")
    raise ProfileError("invalid_encoding", "unsupported major type")


def decode(data: bytes):
    """Decode exactly one canonical record; trailing bytes are an error."""
    item, offset = _decode_item(data, 0, 0)
    if offset != len(data):
        raise ProfileError("invalid_encoding", "trailing bytes after record")
    return item


# --------------------------------------------------------------------------
# Digests (spec §2.1) and strict field helpers
# --------------------------------------------------------------------------

def digest_with_separator(separator: bytes, payload: bytes) -> bytes:
    return hashlib.sha256(separator + b"\x00" + payload).digest()


def domain_digest(domain: str, *fields: bytes) -> bytes:
    return digest_with_separator(DOMAINS[domain], b"".join(fields))


def _list(value, length: int, what: str) -> list:
    if not isinstance(value, list):
        raise ProfileError("invalid_encoding", f"{what} is not an array")
    if len(value) != length:
        raise ProfileError("invalid_encoding", f"{what} has wrong element count")
    return value


def _uint(value, what: str, maximum: int = (1 << 64) - 1) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ProfileError("invalid_encoding", f"{what} is not a uint")
    if value > maximum:
        raise ProfileError("invalid_encoding", f"{what} exceeds the profile cap")
    return value


def _bytes(value, width: int, what: str) -> bytes:
    if not isinstance(value, bytes):
        raise ProfileError("invalid_encoding", f"{what} is not a byte string")
    if len(value) != width:
        raise ProfileError("invalid_encoding", f"{what} has wrong width")
    return value


def _text(value, what: str, maximum: int = MAX_PATH_BYTES) -> str:
    if not isinstance(value, str):
        raise ProfileError("invalid_encoding", f"{what} is not a text string")
    if len(value.encode("utf-8")) > maximum:
        raise ProfileError("invalid_encoding", f"{what} exceeds the profile cap")
    return value


def _tag(value, allowed, what: str) -> int:
    value = _uint(value, what)
    if value not in allowed:
        raise ProfileError("unsupported_capability", f"{what} is not an assigned tag")
    return value


def _sorted_unique(values: list, what: str) -> list:
    if any(left >= right for left, right in zip(values, values[1:])):
        raise ProfileError("invalid_encoding", f"{what} is not sorted and unique")
    return values


def _version(value) -> None:
    value = _uint(value, "profile version")
    if value != PROFILE_VERSION:
        raise ProfileError("unsupported_version", "only profile version 1 is frozen")


# --------------------------------------------------------------------------
# Paths (spec §11)
# --------------------------------------------------------------------------

def validate_path(value: str) -> None:
    if not isinstance(value, str):
        raise ProfileError("invalid_encoding", "path is not text")
    encoded = value.encode("utf-8")
    if not encoded:
        raise ProfileError("invalid_encoding", "path is empty")
    if len(encoded) > MAX_PATH_BYTES:
        raise ProfileError("invalid_encoding", "path exceeds the profile cap")
    if b"\x00" in encoded:
        raise ProfileError("invalid_encoding", "path contains NUL")
    if encoded.startswith(b"/") or encoded.endswith(b"/"):
        raise ProfileError("invalid_encoding",
                           "path is absolute or has a trailing slash")
    for segment in encoded.split(b"/"):
        if not segment:
            raise ProfileError("invalid_encoding", "path has an empty component")
        if len(segment) > MAX_PATH_COMPONENT_BYTES:
            raise ProfileError("invalid_encoding",
                               "path component exceeds the profile cap")
        if segment in (b".", b".."):
            raise ProfileError("invalid_encoding", "path contains a dot component")
        if segment.decode("utf-8") in RESERVED_SEGMENTS:
            raise ProfileError("invalid_encoding",
                               "path uses a reserved control segment")


# --------------------------------------------------------------------------
# Record encoders (spec §7.1, §7.2, §8.1, §8.2, §9)
#
# Structured inputs are JSON-shaped dicts with hex strings for byte fields.
# --------------------------------------------------------------------------

def unhex(value: str) -> bytes:
    return bytes.fromhex(value)


def encode_git_oid(oid: dict) -> bytes:
    return array([uint(PROFILE_VERSION), uint(oid["algorithm"]),
                  byt(unhex(oid["digest"]))])


def encode_epoch_descriptor(desc: dict) -> bytes:
    oid = desc["oid"]
    return array([
        uint(PROFILE_VERSION),
        byt(unhex(desc["file"])), byt(unhex(desc["nonce"])), uint(desc["kind"]),
        byt(unhex(desc["baseline"])),
        NULL if desc["seed_profile"] is None else text(desc["seed_profile"]),
        NULL if oid is None else encode_git_oid(oid),
    ])


def epoch_id(body: bytes) -> bytes:
    return domain_digest("epoch", body)


def encode_operation(op: dict) -> bytes:
    """Encode one §7.1 operation from its structured form; order is semantic."""
    tag = op["tag"]
    if tag == 0:
        return array([uint(0), byt(unhex(op["repository"])), byt(unhex(op["file"])),
                      text(op["path"]), uint(op["mode"]),
                      encode_epoch_descriptor(op["epoch"])])
    if tag == 1:
        return array([uint(1), byt(unhex(op["file"])), text(op["path"])])
    if tag == 2:
        return array([uint(2), byt(unhex(op["file"])), uint(op["live"])])
    if tag == 3:
        return array([uint(3), byt(unhex(op["file"])),
                      encode_epoch_descriptor(op["epoch"])])
    if tag == 4:
        return array([uint(4), byt(unhex(op["file"])), byt(unhex(op["epoch"])),
                      byt(unhex(op["artifact"]))])
    if tag == 5:
        return array([uint(5), uint(op["schema"]), byt(unhex(op["event"])),
                      byt(unhex(op["bytes"]))])
    if tag == 6:
        return array([uint(6), byt(unhex(op["repository"])),
                      encode_git_oid(op["oid"]), byt(unhex(op["version"])),
                      uint(op["relation"])])
    if tag == 7:
        return array([uint(7), byt(unhex(op["file"])), uint(op["mode"])])
    if tag == 8:
        payload = op["payload"]
        resolution = op["resolution"]
        if resolution in (0, 1):
            encoded_payload = NULL
        elif resolution == 2:
            encoded_payload = byt(unhex(payload))
        elif resolution == 3:
            encoded_payload = array([byt(unhex(payload[0])),
                                     byt(unhex(payload[1]))])
        else:
            encoded_payload = array([uint(payload[0]),
                                     array([byt(unhex(payload[1][0])),
                                            uint(payload[1][1])])])
        return array([uint(8), byt(unhex(op["conflict"])), uint(resolution),
                      encoded_payload])
    if tag == 9:
        detail = op["detail"]
        return array([uint(9),
                      array(byt(unhex(source)) for source in op["sources"]),
                      uint(op["relation"]),
                      NULL if detail is None else byt(unhex(detail))])
    raise ProfileError("unsupported_capability", f"operation tag {tag} is unassigned")


def metadata_artifact(operations, context_assertions) -> bytes:
    return array([uint(PROFILE_VERSION), array(operations),
                  array(byt(item) for item in context_assertions)])


def encode_conflict(conflict_rec: dict) -> bytes:
    """§9 ConflictV1. Participants are ordered by (batch, index); the encoder
    never sorts — an unsorted input is an encoding error, not a reordering."""
    return array([
        uint(PROFILE_VERSION),
        uint(conflict_rec["kind"]), byt(unhex(conflict_rec["file"])),
        array(byt(unhex(epoch)) for epoch in conflict_rec["epochs"]),
        array(array([byt(unhex(participant["batch"])),
                     uint(participant["index"])])
              for participant in conflict_rec["participants"]),
    ])


def conflict_id(body: bytes) -> bytes:
    return domain_digest("conflict", body)


def file_entry_raw(file_id, live, paths, epochs, contents, modes, conflicts) -> bytes:
    """Encode one FileEntryV1 exactly as given; no sorting happens here."""
    return array([
        uint(PROFILE_VERSION),
        byt(file_id),
        array(uint(tag) for tag in live),
        array(text(path) for path in paths),
        array(byt(epoch) for epoch in epochs),
        array(byt(content) for content in contents),
        array(uint(tag) for tag in modes),
        array(byt(item) for item in conflicts),
    ])


def file_entry(entry: dict) -> bytes:
    """§7.2 FileEntryV1: every inner array sorted ascending and deduplicated,
    paths by raw UTF-8 bytes, exactly like the Rust FileEntry::encode."""
    paths = {path.encode("utf-8"): path for path in entry["paths"]}
    return file_entry_raw(
        unhex(entry["file"]),
        sorted(set(entry["live"])),
        [paths[raw] for raw in sorted(paths)],
        sorted({unhex(epoch) for epoch in entry["epochs"]}),
        sorted({unhex(content) for content in entry["contents"]}),
        sorted(set(entry["modes"])),
        sorted({unhex(item) for item in entry["conflicts"]}),
    )


def semantic_manifest_raw(space: bytes, entry_bodies) -> bytes:
    """Encode one SemanticManifestV1 from pre-encoded entries, unsorted."""
    return array([uint(PROFILE_VERSION), byt(space), array(entry_bodies)])


def semantic_manifest(manifest_rec: dict) -> bytes:
    """§7.2 SemanticManifestV1: entries sorted ascending by raw FileId bytes."""
    bodies = [file_entry(entry) for entry in manifest_rec["entries"]]
    bodies = sorted(bodies, key=lambda body: decode(body)[1])
    return semantic_manifest_raw(unhex(manifest_rec["space"]), bodies)


def semantic_manifest_id(body: bytes) -> bytes:
    return domain_digest("semantic_manifest", body)


# --------------------------------------------------------------------------
# Record validators (spec §7.1, §7.2, §8.1, §8.2, §9, §11)
# --------------------------------------------------------------------------

def validate_git_oid(item) -> None:
    record = _list(item, 3, "git_oid")
    _version(record[0])
    algorithm = _uint(record[1], "git_oid algorithm")
    if algorithm not in GIT_ALGORITHM_WIDTHS:
        raise ProfileError("unsupported_capability", "unknown Git OID algorithm")
    _bytes(record[2], GIT_ALGORITHM_WIDTHS[algorithm], "git_oid digest")


def validate_epoch_descriptor(item) -> None:
    record = _list(item, 7, "epoch_descriptor")
    _version(record[0])
    _bytes(record[1], 16, "epoch file")
    _bytes(record[2], 16, "generation nonce")
    kind = _tag(record[3], EPOCH_KIND_TAGS, "epoch kind tag")
    _bytes(record[4], 32, "baseline artifact")
    profile = record[5]
    if kind == 0:
        _text(profile, "seed profile", 64)
        if profile != TEXT_PROFILE:
            raise ProfileError("unsupported_capability",
                               "only dt-text-v1 is a frozen text profile")
    elif profile is not None:
        raise ProfileError("invalid_encoding",
                           "non-text epochs must not declare a native seed profile")
    if record[6] is not None:
        validate_git_oid(record[6])


def validate_operation(item) -> None:
    if not isinstance(item, list) or not item:
        raise ProfileError("invalid_encoding", "operation is not a non-empty array")
    tag = _tag(item[0], OPERATION_TAGS, "operation tag")
    if tag == 0:
        record = _list(item, 6, "create operation")
        _bytes(record[1], 16, "repository")
        _bytes(record[2], 16, "file")
        validate_path(_text(record[3], "path"))
        _tag(record[4], MODE_TAGS, "mode tag")
        validate_epoch_descriptor(record[5])
    elif tag == 1:
        record = _list(item, 3, "path operation")
        _bytes(record[1], 16, "file")
        validate_path(_text(record[2], "path"))
    elif tag == 2:
        record = _list(item, 3, "liveness operation")
        _bytes(record[1], 16, "file")
        _tag(record[2], LIVE_TAGS, "live tag")
    elif tag == 3:
        record = _list(item, 3, "epoch operation")
        _bytes(record[1], 16, "file")
        validate_epoch_descriptor(record[2])
    elif tag == 4:
        record = _list(item, 4, "opaque operation")
        _bytes(record[1], 16, "file")
        _bytes(record[2], 32, "opaque epoch")
        _bytes(record[3], 32, "opaque artifact")
    elif tag == 5:
        record = _list(item, 4, "context operation")
        _tag(record[1], CONTEXT_SCHEMA_TAGS, "context schema tag")
        _bytes(record[2], 16, "event id")
        if not isinstance(record[3], bytes):
            raise ProfileError("invalid_encoding", "context event is not bytes")
        if len(record[3]) > MAX_CONTEXT_EVENT_BYTES:
            raise ProfileError("invalid_encoding", "context event exceeds the cap")
    elif tag == 6:
        record = _list(item, 5, "git association operation")
        _bytes(record[1], 16, "repository")
        validate_git_oid(record[2])
        _bytes(record[3], 32, "version")
        # relation_tag is deliberately opaque in v1: width only, never interpreted.
        _uint(record[4], "git relation tag")
    elif tag == 7:
        record = _list(item, 3, "mode operation")
        _bytes(record[1], 16, "file")
        _tag(record[2], MODE_TAGS, "mode tag")
    elif tag == 8:
        record = _list(item, 4, "resolution operation")
        _bytes(record[1], 32, "conflict id")
        resolution = _tag(record[2], RESOLUTION_TAGS, "resolution tag")
        payload = record[3]
        if resolution in (0, 1):
            if payload is not None:
                raise ProfileError("invalid_encoding",
                                   "this resolution tag takes a null payload")
        elif resolution == 2:
            _bytes(payload, 32, "retained epoch")
        elif resolution == 3:
            nested = _list(payload, 2, "incorporation payload")
            _bytes(nested[0], 32, "incorporated epoch")
            _bytes(nested[1], 32, "provenance artifact")
        else:
            nested = _list(payload, 2, "alternative payload")
            _tag(nested[0], FIELD_TAGS, "field tag")
            identity = _list(nested[1], 2, "write identity")
            _bytes(identity[0], 32, "participant batch")
            _uint(identity[1], "operation index", MAX_OPERATION_INDEX)
    else:
        record = _list(item, 4, "provenance operation")
        sources = record[1]
        if not isinstance(sources, list):
            raise ProfileError("invalid_encoding", "source batches is not an array")
        for source in sources:
            _bytes(source, 32, "source batch")
        _uint(record[2], "provenance relation tag")
        if record[3] is not None:
            _bytes(record[3], 32, "detail artifact")


def validate_metadata_artifact(item) -> None:
    record = _list(item, 3, "metadata_artifact")
    _version(record[0])
    operations = record[1]
    if not isinstance(operations, list):
        raise ProfileError("invalid_encoding", "operations is not an array")
    if len(operations) > MAX_METADATA_OPERATIONS:
        raise ProfileError("invalid_encoding", "operations exceed the profile cap")
    for operation in operations:
        validate_operation(operation)
    assertions = record[2]
    if not isinstance(assertions, list):
        raise ProfileError("invalid_encoding", "context assertions is not an array")
    if len(assertions) > MAX_CONTEXT_ASSERTIONS:
        raise ProfileError("invalid_encoding", "context assertions exceed the cap")


def validate_conflict(item) -> None:
    record = _list(item, 5, "conflict")
    _version(record[0])
    kind = _tag(record[1], CONFLICT_KIND_TAGS, "conflict kind tag")
    _bytes(record[2], 16, "conflict file")
    epochs = record[3]
    if not isinstance(epochs, list):
        raise ProfileError("invalid_encoding", "epochs is not an array")
    for epoch in epochs:
        _bytes(epoch, 32, "conflict epoch")
    _sorted_unique(epochs, "conflict epochs")
    if kind == 0 and epochs:
        raise ProfileError("invalid_encoding",
                           "register conflicts concern no epoch")
    participants = record[4]
    if not isinstance(participants, list):
        raise ProfileError("invalid_encoding", "participants is not an array")
    identities = []
    for participant in participants:
        nested = _list(participant, 2, "participant")
        batch = _bytes(nested[0], 32, "participant batch")
        index = _uint(nested[1], "operation index", MAX_OPERATION_INDEX)
        identities.append((batch, index))
    if any(left >= right for left, right in zip(identities, identities[1:])):
        raise ProfileError("invalid_encoding",
                           "participants are not sorted by batch then index")


def validate_file_entry(item) -> None:
    record = _list(item, 8, "file_entry")
    _version(record[0])
    _bytes(record[1], 16, "file")
    for values, what, allowed in ((record[2], "live tags", LIVE_TAGS),
                                  (record[6], "mode tags", MODE_TAGS)):
        if not isinstance(values, list):
            raise ProfileError("invalid_encoding", f"{what} is not an array")
        for value in values:
            _tag(value, allowed, what)
        _sorted_unique(values, what)
    paths = record[3]
    if not isinstance(paths, list):
        raise ProfileError("invalid_encoding", "paths is not an array")
    encoded_paths = []
    for path in paths:
        validate_path(path)
        encoded_paths.append(path.encode("utf-8"))
    _sorted_unique(encoded_paths, "paths")
    for values, what in ((record[4], "epochs"), (record[5], "contents"),
                         (record[7], "conflicts")):
        if not isinstance(values, list):
            raise ProfileError("invalid_encoding", f"{what} is not an array")
        for value in values:
            _bytes(value, 32, what)
        _sorted_unique(values, what)


def validate_semantic_manifest(item) -> None:
    record = _list(item, 3, "semantic_manifest")
    _version(record[0])
    _bytes(record[1], 16, "space")
    entries = record[2]
    if not isinstance(entries, list):
        raise ProfileError("invalid_encoding", "entries is not an array")
    files = []
    for entry in entries:
        validate_file_entry(entry)
        files.append(entry[1])
    _sorted_unique(files, "manifest entries")


VALIDATORS = {
    "epoch_descriptor": validate_epoch_descriptor,
    "git_oid": validate_git_oid,
    "metadata_artifact": validate_metadata_artifact,
    "conflict": validate_conflict,
    "file_entry": validate_file_entry,
    "semantic_manifest": validate_semantic_manifest,
}


def validate_record(record: str, data: bytes) -> None:
    """Decode and validate one frozen record, raising ProfileError on failure."""
    if record not in VALIDATORS:
        raise KeyError(f"unknown record {record}")
    VALIDATORS[record](decode(data))


# --------------------------------------------------------------------------
# Deterministic fixture data
# --------------------------------------------------------------------------

def _fixture_bytes(label: str, width: int) -> bytes:
    digest = hashlib.sha256(b"puyudb/metadata-v1/fixture/" + label.encode()).digest()
    return digest[:width]


SPACE = _fixture_bytes("space", 16)
REPOSITORY = _fixture_bytes("repository", 16)
FILE_A = _fixture_bytes("file-a", 16)
FILE_B = _fixture_bytes("file-b", 16)
NONCE_TEXT = _fixture_bytes("nonce-text", 16)
NONCE_OPAQUE = _fixture_bytes("nonce-opaque", 16)
NONCE_SYMLINK = _fixture_bytes("nonce-symlink", 16)
BASELINE = _fixture_bytes("baseline-artifact", 32)
OPAQUE_BASELINE = _fixture_bytes("opaque-baseline-artifact", 32)
SYMLINK_BASELINE = _fixture_bytes("symlink-baseline-artifact", 32)
OPAQUE_ARTIFACT = _fixture_bytes("opaque-content-artifact", 32)
OID_SHA1 = _fixture_bytes("git-oid-sha1", 20)
OID_SHA256 = _fixture_bytes("git-oid-sha256", 32)
BATCH_A = _fixture_bytes("batch-a", 32)
BATCH_B = _fixture_bytes("batch-b", 32)
VERSION = _fixture_bytes("version", 32)
EVENT = _fixture_bytes("context-event", 16)
CONTEXT_BYTES = _fixture_bytes("context-bytes", 9)
ASSERTION = _fixture_bytes("context-assertion", 12)

DESC_TEXT = {"file": FILE_A.hex(), "nonce": NONCE_TEXT.hex(), "kind": 0,
             "baseline": BASELINE.hex(), "seed_profile": TEXT_PROFILE,
             "oid": {"algorithm": 1, "digest": OID_SHA1.hex()}}
DESC_OPAQUE_A = {"file": FILE_A.hex(), "nonce": NONCE_OPAQUE.hex(), "kind": 1,
                 "baseline": OPAQUE_BASELINE.hex(), "seed_profile": None,
                 "oid": None}
DESC_OPAQUE_B = {"file": FILE_B.hex(), "nonce": NONCE_OPAQUE.hex(), "kind": 1,
                 "baseline": OPAQUE_BASELINE.hex(), "seed_profile": None,
                 "oid": None}
DESC_SYMLINK = {"file": FILE_A.hex(), "nonce": NONCE_SYMLINK.hex(), "kind": 2,
                "baseline": SYMLINK_BASELINE.hex(), "seed_profile": None,
                "oid": None}

EPOCH_TEXT = epoch_id(encode_epoch_descriptor(DESC_TEXT))
EPOCH_OPAQUE_A = epoch_id(encode_epoch_descriptor(DESC_OPAQUE_A))
EPOCH_OPAQUE_B = epoch_id(encode_epoch_descriptor(DESC_OPAQUE_B))
EPOCH_SYMLINK = epoch_id(encode_epoch_descriptor(DESC_SYMLINK))


def _participants(*writes) -> list:
    return [{"batch": batch.hex(), "index": index}
            for batch, index in sorted(writes)]


CONFLICT_PATHS = {"kind": 0, "file": FILE_A.hex(), "epochs": [],
                  "participants": _participants((BATCH_A, 0), (BATCH_B, 0))}
# §9 as derived by projection.rs conflicts_impl: a kind-1 delete_edit
# conflict's epochs hold the edited epoch only, never every maximal epoch
# selection of the file.
CONFLICT_DELETE_EDIT = {"kind": 1, "file": FILE_A.hex(),
                        "epochs": [EPOCH_OPAQUE_A.hex()],
                        "participants": _participants((BATCH_A, 0), (BATCH_B, 0))}
CONFLICT_MODE = {"kind": 0, "file": FILE_A.hex(), "epochs": [],
                 "participants": _participants((BATCH_A, 0), (BATCH_B, 0))}

CID_PATHS = conflict_id(encode_conflict(CONFLICT_PATHS))
CID_DELETE_EDIT = conflict_id(encode_conflict(CONFLICT_DELETE_EDIT))
CID_MODE = conflict_id(encode_conflict(CONFLICT_MODE))


def _entry(file_id, live, paths, epochs, contents, modes, conflicts) -> dict:
    return {"file": file_id.hex(), "live": live, "paths": paths,
            "epochs": [epoch.hex() for epoch in epochs],
            "contents": [content.hex() for content in contents],
            "modes": modes, "conflicts": [item.hex() for item in conflicts]}


# Pre-resolution cut of the delete-concurrent-edit scenario: the deletion and
# the epoch selection are concurrent, so both liveness alternatives and both
# epoch selections stay maximal and the kind-1 conflict is listed.
ENTRY_A_CONFLICTED = _entry(FILE_A, [0, 1], ["docs/renamed.md"],
                            [EPOCH_TEXT, EPOCH_OPAQUE_A], [], [0],
                            [CID_DELETE_EDIT])
# Post-resolution cut: the resolution batch's fresh Alive and tag-3 writes
# supersede the conflicted alternatives, and the recomputed cut carries no
# ConflictId (tag8_resolution_publishes_fresh_assignment_and_supersedes_alternatives
# in crates/puyudb-core/src/publication.rs).
ENTRY_A_RESOLVED = _entry(FILE_A, [1], ["docs/renamed.md"], [EPOCH_OPAQUE_A],
                          [], [0], [])
ENTRY_B_OPAQUE = _entry(FILE_B, [1], ["assets/logo.bin"], [EPOCH_OPAQUE_B],
                        [OPAQUE_ARTIFACT], [0], [])


def _scenario(name, summary, operations, assertions, epochs, conflicts,
              manifest) -> dict:
    ops = [encode_operation(op) for op in operations]
    artifact = metadata_artifact(ops, [unhex(item) for item in assertions])
    epoch_entries = []
    for desc in epochs:
        body = encode_epoch_descriptor(desc)
        epoch_entries.append({"descriptor": desc, "body_hex": body.hex(),
                              "epoch_id": epoch_id(body).hex()})
    conflict_entries = []
    for conflict_rec in conflicts:
        body = encode_conflict(conflict_rec)
        conflict_entries.append({**conflict_rec, "body_hex": body.hex(),
                                 "conflict_id": conflict_id(body).hex()})
    manifest_body = semantic_manifest(manifest)
    return {
        "name": name,
        "summary": summary,
        "operations": operations,
        "operations_hex": [op.hex() for op in ops],
        "context_assertions": assertions,
        "artifact_hex": artifact.hex(),
        "epochs": epoch_entries,
        "conflicts": conflict_entries,
        "manifest": {**manifest, "body_hex": manifest_body.hex(),
                     "semantic_manifest_id": semantic_manifest_id(manifest_body).hex()},
    }


def build_scenarios() -> dict:
    """The metadata-artifact scenario corpus: five cuts with their expected
    canonical hex of operations, ConflictIds and SemanticManifestId."""
    create_rename = _scenario(
        "create-rename",
        "One batch creates file A at docs/notes.md (text epoch with a SHA-1 Git "
        "OID) and causally renames it to docs/renamed.md; no conflict.",
        [{"tag": 0, "repository": REPOSITORY.hex(), "file": FILE_A.hex(),
          "path": "docs/notes.md", "mode": 0, "epoch": DESC_TEXT},
         {"tag": 1, "file": FILE_A.hex(), "path": "docs/renamed.md"}],
        [], [DESC_TEXT], [],
        {"space": SPACE.hex(),
         "entries": [_entry(FILE_A, [1], ["docs/renamed.md"], [EPOCH_TEXT],
                            [], [0], [])]})
    concurrent_paths = _scenario(
        "concurrent-path-alternatives",
        "Batch A assigned docs/a.md while batch B concurrently assigned "
        "docs/b.md: a kind-0 register conflict and two maximal path "
        "alternatives in the manifest.",
        [{"tag": 1, "file": FILE_A.hex(), "path": "docs/b.md"}],
        [], [DESC_TEXT], [CONFLICT_PATHS],
        {"space": SPACE.hex(),
         "entries": [_entry(FILE_A, [1], ["docs/a.md", "docs/b.md"],
                            [EPOCH_TEXT], [], [0], [CID_PATHS])]})
    delete_edit = _scenario(
        "delete-concurrent-edit",
        "Batch A deleted file A while batch B concurrently selected a new "
        "opaque epoch: a kind-1 delete_edit whose epochs hold the edited "
        "epoch only (not every maximal epoch selection of the file); file B "
        "carries an opaque value assignment in the same cut.",
        [{"tag": 3, "file": FILE_A.hex(), "epoch": DESC_OPAQUE_A}],
        [], [DESC_OPAQUE_A, DESC_OPAQUE_B], [CONFLICT_DELETE_EDIT],
        {"space": SPACE.hex(),
         "entries": [ENTRY_A_CONFLICTED, ENTRY_B_OPAQUE]})
    mode_conflict = _scenario(
        "mode-conflict",
        "Batch A set mode 1 (executable) while batch B concurrently set mode "
        "2 (symlink entry) and selected the symlink-target epoch: register "
        "conflict on the mode field, both modes maximal.",
        [{"tag": 7, "file": FILE_A.hex(), "mode": 2},
         {"tag": 3, "file": FILE_A.hex(), "epoch": DESC_SYMLINK}],
        [], [DESC_SYMLINK], [CONFLICT_MODE],
        {"space": SPACE.hex(),
         "entries": [_entry(FILE_A, [1], ["docs/renamed.md"],
                            [EPOCH_TEXT, EPOCH_SYMLINK], [], [1, 2], [CID_MODE])]})
    resolution = _scenario(
        "resolution",
        "Batch B resolves the delete_edit conflict by retaining the opaque "
        "epoch (resolution tag 2) with the fresh tag-3 selection of that epoch "
        "in the same batch — the required same-batch effect of §9 — and asserts "
        "provenance over batch A; it also re-asserts liveness, associates a "
        "SHA-256 Git object, emits a context event and assigns file B's opaque "
        "value. The fresh writes supersede the conflicted alternatives, so the "
        "recomputed cut — and the manifest — carries no ConflictId.",
        [{"tag": 2, "file": FILE_A.hex(), "live": 1},
         {"tag": 8, "conflict": CID_DELETE_EDIT.hex(), "resolution": 2,
          "payload": EPOCH_OPAQUE_A.hex()},
         {"tag": 3, "file": FILE_A.hex(), "epoch": DESC_OPAQUE_A},
         {"tag": 9, "sources": [BATCH_A.hex()], "relation": 1, "detail": None},
         {"tag": 6, "repository": REPOSITORY.hex(),
          "oid": {"algorithm": 2, "digest": OID_SHA256.hex()},
          "version": VERSION.hex(), "relation": 0},
         {"tag": 5, "schema": 1, "event": EVENT.hex(),
          "bytes": CONTEXT_BYTES.hex()},
         {"tag": 4, "file": FILE_B.hex(), "epoch": EPOCH_OPAQUE_B.hex(),
          "artifact": OPAQUE_ARTIFACT.hex()}],
        [ASSERTION.hex()], [DESC_OPAQUE_A], [],
        {"space": SPACE.hex(),
         "entries": [ENTRY_A_RESOLVED, ENTRY_B_OPAQUE]})
    return {
        "profile": PROFILE_VERSION,
        "generated_by": "scripts/i05_semantic_manifest.py",
        "spec": "docs/spec/canonical-profile-v1.md",
        "plan": "docs/plans/I05-native-text-projections.md",
        "scope": "I05 P5 golden fixtures: metadata operations, ConflictV1 and "
                 "SemanticManifestV1 at five scenario cuts, with expected "
                 "canonical hex and digests computed by this module alone. "
                 "The hex and digests are record-encoding goldens (§1.1/§7/§8/"
                 "§9/§11); the scenarios additionally pin derivation semantics "
                 "(the §9 same-batch effect of a resolution, the recomputed cut "
                 "after a resolution, the delete_edit epoch set) that the Rust "
                 "projection cross-checks as P9 (T01-T03 parity).",
        "scenarios": [create_rename, concurrent_paths, delete_edit,
                      mode_conflict, resolution],
    }


def build_negatives() -> dict:
    """Rejected or hash-diverging forms. Ordering violations must be rejected
    by validation AND hash differently from the canonical form; a wrong domain
    separator must change the digest."""
    delete_edit_body = encode_conflict(CONFLICT_DELETE_EDIT)
    manifest = {"space": SPACE.hex(),
                "entries": [ENTRY_A_CONFLICTED, ENTRY_B_OPAQUE]}
    manifest_body = semantic_manifest(manifest)
    entry_a = file_entry(ENTRY_A_CONFLICTED)
    entry_b = file_entry(ENTRY_B_OPAQUE)
    bigger, smaller = sorted([entry_a, entry_b],
                             key=lambda body: decode(body)[1], reverse=True)

    unsorted_participants = encode_conflict(
        {**CONFLICT_DELETE_EDIT,
         "participants": list(reversed(CONFLICT_DELETE_EDIT["participants"]))})
    epoch_edit = {"kind": 2, "file": FILE_A.hex(),
                  "epochs": sorted([EPOCH_OPAQUE_A.hex(), EPOCH_TEXT.hex()]),
                  "participants": _participants((BATCH_A, 1), (BATCH_B, 2))}
    unsorted_epochs = encode_conflict(
        {**epoch_edit, "epochs": list(reversed(epoch_edit["epochs"]))})
    unsorted_entries = semantic_manifest_raw(SPACE, [bigger, smaller])
    unsorted_paths_entry = file_entry_raw(
        FILE_A, [0, 1], ["docs/renamed.md", "docs/a.md"],
        sorted([EPOCH_TEXT, EPOCH_OPAQUE_A]), [], [0], [CID_DELETE_EDIT])
    unsorted_paths_manifest = semantic_manifest_raw(SPACE, [unsorted_paths_entry])

    def case(name, record, body, code, why, **extra):
        result = {"name": name, "record": record, "body_hex": body.hex(),
                  "code": code, "why": why}
        result.update(extra)
        return result

    def ordering_case(name, record, body, canonical_body, why):
        return case(name, record, body, "invalid_encoding", why,
                    other_id_hex=domain_digest(RECORD_DOMAINS[record], body).hex(),
                    canonical_body_hex=canonical_body.hex(),
                    canonical_id_hex=domain_digest(RECORD_DOMAINS[record],
                                                   canonical_body).hex())

    def domain_case(name, record, body, separator, why):
        return case(name, record, body, None, why,
                    separator=separator,
                    other_id_hex=digest_with_separator(separator.encode("ascii"),
                                                       body).hex(),
                    canonical_id_hex=domain_digest(RECORD_DOMAINS[record],
                                                   body).hex())

    return {
        "profile": PROFILE_VERSION,
        "generated_by": "scripts/i05_semantic_manifest.py",
        "scope": "Negative cases for the I05 P5 fixtures: unsorted inputs are "
                 "rejected and hash differently; wrong domain separators "
                 "change the digest.",
        "cases": [
            ordering_case("conflict/unsorted-participants", "conflict",
                          unsorted_participants, delete_edit_body,
                          "participants sort by (batch, index); reversed order is "
                          "invalid and must change the ConflictId"),
            ordering_case("conflict/unsorted-epochs", "conflict",
                          unsorted_epochs, encode_conflict(epoch_edit),
                          "conflict epochs are sorted unique; reversed order is "
                          "invalid and must change the ConflictId"),
            ordering_case("manifest/unsorted-entries", "semantic_manifest",
                          unsorted_entries, manifest_body,
                          "manifest entries sort by raw FileId; reversed order is "
                          "invalid and must change the SemanticManifestId"),
            ordering_case("manifest/entry-unsorted-paths", "semantic_manifest",
                          unsorted_paths_manifest, manifest_body,
                          "entry paths sort by raw UTF-8 bytes; unsorted paths "
                          "are invalid and must change the SemanticManifestId"),
            domain_case("domain/conflict-wrong-separator", "conflict",
                        delete_edit_body, "puyudb/conflict/v2",
                        "a wrong separator must change the ConflictId"),
            domain_case("domain/manifest-wrong-separator", "semantic_manifest",
                        manifest_body, "puyudb/semantic-manifest/v2",
                        "a wrong separator must change the SemanticManifestId"),
            domain_case("domain/cross-domain", "conflict", delete_edit_body,
                        "puyudb/semantic-manifest/v1",
                        "hashing a conflict under the manifest separator must "
                        "differ (domain separation)"),
            case("registry/unknown-operation-tag", "metadata_artifact",
                 metadata_artifact([array([uint(10), byt(FILE_A)])], []),
                 "unsupported_capability", "operation tag 10 is unassigned"),
            case("registry/unknown-resolution-tag", "metadata_artifact",
                 metadata_artifact([array([uint(8), byt(CID_DELETE_EDIT),
                                           uint(5), NULL])], []),
                 "unsupported_capability", "resolution tag 5 is unassigned"),
            case("registry/unknown-conflict-kind", "conflict",
                 encode_conflict({"kind": 3, "file": FILE_A.hex(), "epochs": [],
                                  "participants": _participants((BATCH_A, 0))}),
                 "unsupported_capability", "conflict kind 3 is unassigned"),
        ],
    }


# --------------------------------------------------------------------------
# Fixture verification (re-encode from the frozen layouts, compare every hex)
# --------------------------------------------------------------------------

def _has_fresh_effect(op: dict, operations: list) -> bool:
    """§9: each resolution tag requires a fresh same-batch effect, the same
    rule the Rust admission enforces (has_live_op / has_epoch_op in
    validate_resolutions). Tag 4 (choose an alternative) is not exercised by
    the corpus."""
    resolution = op["resolution"]
    if resolution == 0:
        return any(o["tag"] == 2 and o["live"] == 0 for o in operations)
    if resolution == 1:
        return any(o["tag"] == 2 and o["live"] == 1 for o in operations)
    if resolution in (2, 3):
        target = op["payload"] if resolution == 2 else op["payload"][0]
        has_epoch = any(
            o["tag"] == 3
            and epoch_id(encode_epoch_descriptor(o["epoch"])).hex() == target
            for o in operations)
        if resolution == 2:
            return has_epoch
        detail = op["payload"][1]
        return has_epoch and any(o["tag"] == 9 and o["detail"] == detail
                                 for o in operations)
    return True


def verify_scenarios(doc: dict) -> list:
    failures = []
    for scenario in doc["scenarios"]:
        name = scenario["name"]
        ops = [encode_operation(op) for op in scenario["operations"]]
        if [op.hex() for op in ops] != scenario["operations_hex"]:
            failures.append(f"{name}: operations_hex mismatch")
        artifact = metadata_artifact(ops, [unhex(a) for a in
                                           scenario["context_assertions"]])
        if artifact.hex() != scenario["artifact_hex"]:
            failures.append(f"{name}: artifact_hex mismatch")
        try:
            validate_record("metadata_artifact", artifact)
        except ProfileError as error:
            failures.append(f"{name}: metadata artifact rejected: {error}")
        for epoch in scenario["epochs"]:
            body = encode_epoch_descriptor(epoch["descriptor"])
            if body.hex() != epoch["body_hex"]:
                failures.append(f"{name}: epoch body_hex mismatch")
            if epoch_id(body).hex() != epoch["epoch_id"]:
                failures.append(f"{name}: epoch_id mismatch")
            try:
                validate_record("epoch_descriptor", body)
            except ProfileError as error:
                failures.append(f"{name}: epoch descriptor rejected: {error}")
        for conflict_rec in scenario["conflicts"]:
            body = encode_conflict(conflict_rec)
            if body.hex() != conflict_rec["body_hex"]:
                failures.append(f"{name}: conflict body_hex mismatch")
            if conflict_id(body).hex() != conflict_rec["conflict_id"]:
                failures.append(f"{name}: conflict_id mismatch")
            try:
                validate_record("conflict", body)
            except ProfileError as error:
                failures.append(f"{name}: conflict rejected: {error}")
        manifest = scenario["manifest"]
        body = semantic_manifest(manifest)
        if body.hex() != manifest["body_hex"]:
            failures.append(f"{name}: manifest body_hex mismatch")
        if semantic_manifest_id(body).hex() != manifest["semantic_manifest_id"]:
            failures.append(f"{name}: semantic_manifest_id mismatch")
        try:
            validate_record("semantic_manifest", body)
        except ProfileError as error:
            failures.append(f"{name}: semantic manifest rejected: {error}")
        # §9: a resolution operation must carry its required same-batch effect.
        for op in scenario["operations"]:
            if op["tag"] == 8 and not _has_fresh_effect(op, scenario["operations"]):
                failures.append(f"{name}: resolution tag {op['resolution']} lacks "
                                f"its required same-batch effect")
        # The manifest's per-entry conflicts must equal the conflicts
        # recomputed at the scenario's cut.
        derived = {}
        for conflict_rec in scenario["conflicts"]:
            derived.setdefault(conflict_rec["file"], set()).add(
                conflict_rec["conflict_id"])
        for entry in manifest["entries"]:
            if set(entry["conflicts"]) != derived.get(entry["file"], set()):
                failures.append(f"{name}: manifest conflicts for file "
                                f"{entry['file'][:8]} do not match the recomputed cut")
    return failures


def verify_negatives(doc: dict) -> list:
    failures = []
    for case in doc["cases"]:
        name = case["name"]
        record = case["record"]
        body = unhex(case["body_hex"])
        if case["code"]:
            try:
                validate_record(record, body)
                failures.append(f"{name}: accepted but must raise {case['code']}")
            except ProfileError as error:
                if error.code != case["code"]:
                    failures.append(f"{name}: raised {error.code}, "
                                    f"expected {case['code']}")
        if "other_id_hex" in case:
            separator = case.get("separator")
            wrong = (digest_with_separator(separator.encode("ascii"), body)
                     if separator else domain_digest(RECORD_DOMAINS[record], body))
            if wrong.hex() != case["other_id_hex"]:
                failures.append(f"{name}: other_id_hex mismatch")
            canonical_body = (unhex(case["canonical_body_hex"])
                              if "canonical_body_hex" in case else body)
            canonical = domain_digest(RECORD_DOMAINS[record], canonical_body)
            if "canonical_id_hex" in case and \
                    canonical.hex() != case["canonical_id_hex"]:
                failures.append(f"{name}: canonical_id_hex mismatch")
            if wrong == canonical:
                failures.append(f"{name}: wrong form hashes identically to the "
                                f"canonical one")
            if "canonical_body_hex" in case:
                try:
                    validate_record(record, canonical_body)
                except ProfileError as error:
                    failures.append(f"{name}: canonical form rejected: {error}")
    return failures


def fixture_dir() -> Path:
    return Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "metadata-v1"


def load_fixtures() -> tuple:
    target = fixture_dir()
    scenarios = json.loads((target / "scenarios.json").read_text(encoding="utf-8"))
    negatives = json.loads((target / "negative.json").read_text(encoding="utf-8"))
    return scenarios, negatives


def verify_fixtures() -> list:
    scenarios, negatives = load_fixtures()
    return verify_scenarios(scenarios) + verify_negatives(negatives)


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify (or with --write regenerate) the I05 P5 golden "
                    "fixtures in tests/fixtures/metadata-v1/.")
    parser.add_argument("--write", action="store_true",
                        help="regenerate the committed fixtures instead of "
                             "verifying them")
    args = parser.parse_args(argv)
    target = fixture_dir()
    if args.write:
        target.mkdir(parents=True, exist_ok=True)
        for name, payload in (("scenarios.json", build_scenarios()),
                              ("negative.json", build_negatives())):
            body = json.dumps(payload, indent=2, sort_keys=True) + "\n"
            (target / name).write_text(body, encoding="utf-8")
        print(f"wrote {target / 'scenarios.json'} and {target / 'negative.json'}")
        return 0
    scenarios, negatives = load_fixtures()
    failures = verify_scenarios(scenarios) + verify_negatives(negatives)
    if failures:
        for failure in failures:
            print(f"MISMATCH: {failure}")
        print(f"{len(failures)} mismatch(es)")
        return 1
    print(f"ok: {len(scenarios['scenarios'])} scenarios and "
          f"{len(negatives['cases'])} negative cases verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
