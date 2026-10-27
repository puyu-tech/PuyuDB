# SPDX-License-Identifier: Apache-2.0
"""Canonical profile v1 vectors and independent validator.

Source of truth: ``docs/spec/canonical-profile-v1.md``. This module hand-encodes
the frozen layouts and validates records strictly, without a CBOR library and
without any Rust code, so agreement with the I03 T4/T5 codec is evidence about
the specification rather than about a shared dependency.

Not a production codec, admission path, or general crypto implementation.
Ed25519 is the RFC 8032 reference procedure, used only to freeze deterministic
signing vectors; strict verification hardening belongs to ed25519-dalek in T4.
"""

from __future__ import annotations

import base64
import hashlib
import json
from pathlib import Path

PROFILE_VERSION = 1

MAX_BATCH_BODY_BYTES = 1_048_576
MAX_BATCH_PARENTS = 128
MAX_BATCH_NATIVE_ARTIFACTS = 4_096
MAX_FRONTIER_HEADS = 65_536
MAX_ARTIFACT_CHUNKS = 4_096
CHUNK_RAW_BYTES = 262_144
MAX_ARTIFACT_RAW_BYTES = 1_073_741_824
MAX_KIND_TEXT_BYTES = 64
MAX_METADATA_OPERATIONS = 65_536
MAX_CONTEXT_ASSERTIONS = 4_096
MAX_CONTEXT_EVENT_BYTES = 65_536
MAX_PATH_BYTES = 4_096
MAX_PATH_COMPONENT_BYTES = 255
MAX_TEXT_EPOCH_BYTES = 8_388_608
MAX_OPERATION_INDEX = 65_535

TEXT_PROFILE = "dt-text-v1"
ASSIGNED_KINDS = (
    "native-change-v1",
    "metadata-v1",
    "text-baseline-v1",
    "opaque-baseline-v1",
    "symlink-target-v1",
    "branch-descriptor-v1",
)
RESERVED_KINDS = ("named-snapshot-v1", "archive-coverage-v1")

DOMAINS = {
    "batch": b"puyudb/batch/v1",
    "signature": b"puyudb/signature/v1",
    "frontier": b"puyudb/frontier/v1",
    "chunk": b"puyudb/chunk/v1",
    "artifact": b"puyudb/artifact/v1",
    "content": b"puyudb/content/v1",
    "epoch": b"puyudb/epoch/v1",
    "conflict": b"puyudb/conflict/v1",
    "branch_descriptor": b"puyudb/branch-descriptor/v1",
    "branch_signature": b"puyudb/branch-signature/v1",
    "semantic_manifest": b"puyudb/semantic-manifest/v1",
    "dt_seed_actor": b"puyudb/dt-seed-actor/v1",
    "dt_actor": b"puyudb/dt-actor/v1",
}

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
# Digests and presentation (spec §2, §3)
# --------------------------------------------------------------------------

def domain_digest(domain: str, *fields: bytes) -> bytes:
    return hashlib.sha256(DOMAINS[domain] + b"\x00" + b"".join(fields)).digest()


def presentation(prefix: str, raw: bytes) -> str:
    return f"{prefix}:{raw.hex()}"


def parse_presentation(prefix: str, value: str, width: int) -> bytes:
    head, _, body = value.partition(":")
    if head != prefix:
        raise ProfileError("invalid_encoding", "unknown presentation prefix")
    if len(body) != width * 2:
        raise ProfileError("invalid_encoding", "wrong presentation width")
    if body != body.lower():
        raise ProfileError("invalid_encoding", "presentation must be lowercase hex")
    try:
        return bytes.fromhex(body)
    except ValueError as error:
        raise ProfileError("invalid_encoding", "presentation is not hex") from error


# --------------------------------------------------------------------------
# Strict field helpers
# --------------------------------------------------------------------------

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


def _text(value, what: str, maximum: int = MAX_KIND_TEXT_BYTES) -> str:
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
# Record encoders (spec §4 - §9)
# --------------------------------------------------------------------------

def batch_body(space, branch, public_key, writer_epoch, sequence, previous,
               parents, lamport, native_artifact_ids, metadata_artifact) -> bytes:
    return array([
        uint(PROFILE_VERSION),
        byt(space), byt(branch), byt(public_key), byt(writer_epoch),
        uint(sequence),
        NULL if previous is None else byt(previous),
        array(byt(parent) for parent in parents),
        uint(lamport),
        array(byt(artifact) for artifact in native_artifact_ids),
        byt(metadata_artifact),
    ])


def batch_id(body: bytes) -> bytes:
    return domain_digest("batch", body)


def signature_preimage(space: bytes, batch: bytes) -> bytes:
    return DOMAINS["signature"] + b"\x00" + space + batch


def batch_signature(public_key: bytes, signature: bytes) -> bytes:
    return array([uint(PROFILE_VERSION), byt(public_key), byt(signature)])


def frontier(space: bytes, heads) -> bytes:
    return array([uint(PROFILE_VERSION), byt(space),
                  array(byt(head) for head in heads)])


def version_id(body: bytes) -> bytes:
    return domain_digest("frontier", body)


def chunk_id(raw: bytes) -> bytes:
    return domain_digest("chunk", raw)


def raw_digest(kind: str, payload: bytes) -> bytes:
    return domain_digest("content", kind.encode("utf-8"), b"\x00", payload)


def chunk_payload(payload: bytes) -> list:
    if len(payload) > MAX_ARTIFACT_RAW_BYTES:
        raise ProfileError("invalid_encoding", "artifact exceeds the profile cap")
    return [payload[start:start + CHUNK_RAW_BYTES]
            for start in range(0, len(payload), CHUNK_RAW_BYTES)]


def manifest(kind: str, payload: bytes) -> bytes:
    chunks = chunk_payload(payload)
    if len(chunks) > MAX_ARTIFACT_CHUNKS:
        raise ProfileError("invalid_encoding", "artifact needs too many chunks")
    return array([
        uint(PROFILE_VERSION),
        text(kind),
        uint(len(payload)),
        byt(raw_digest(kind, payload)),
        array(byt(chunk_id(chunk)) for chunk in chunks),
    ])


def artifact_id(body: bytes) -> bytes:
    return domain_digest("artifact", body)


def native_artifact(epoch: bytes, profile: str, native_bytes: bytes) -> bytes:
    return array([uint(PROFILE_VERSION), byt(epoch), text(profile), byt(native_bytes)])


def git_oid(algorithm_tag: int, digest: bytes) -> bytes:
    if algorithm_tag not in GIT_ALGORITHM_WIDTHS:
        raise ProfileError("unsupported_capability", "unknown Git OID algorithm")
    if len(digest) != GIT_ALGORITHM_WIDTHS[algorithm_tag]:
        raise ProfileError("invalid_encoding", "Git OID digest has wrong width")
    return array([uint(PROFILE_VERSION), uint(algorithm_tag), byt(digest)])


def epoch_descriptor(file_id, nonce, kind_tag, baseline_artifact, seed_profile,
                     oid) -> bytes:
    return array([
        uint(PROFILE_VERSION),
        byt(file_id), byt(nonce), uint(kind_tag), byt(baseline_artifact),
        NULL if seed_profile is None else text(seed_profile),
        NULL if oid is None else oid,
    ])


def epoch_id(body: bytes) -> bytes:
    return domain_digest("epoch", body)


def conflict(kind_tag: int, file_id: bytes, epochs, participants) -> bytes:
    return array([
        uint(PROFILE_VERSION),
        uint(kind_tag), byt(file_id),
        array(byt(epoch) for epoch in epochs),
        array(array([byt(batch), uint(index)]) for batch, index in participants),
    ])


def conflict_id(body: bytes) -> bytes:
    return domain_digest("conflict", body)


def branch_descriptor(space, branch, initial_version, creator_key) -> bytes:
    return array([uint(PROFILE_VERSION), byt(space), byt(branch),
                  byt(initial_version), byt(creator_key)])


def branch_descriptor_id(body: bytes) -> bytes:
    return domain_digest("branch_descriptor", body)


def branch_signature_preimage(body: bytes) -> bytes:
    return DOMAINS["branch_signature"] + b"\x00" + body


def branch_envelope(body: bytes, signature: bytes) -> bytes:
    return array([uint(PROFILE_VERSION), byt(body), byt(signature)])


def metadata_artifact(operations, context_assertions) -> bytes:
    return array([uint(PROFILE_VERSION), array(operations),
                  array(context_assertions)])


def semantic_manifest(space: bytes, entries) -> bytes:
    return array([uint(PROFILE_VERSION), byt(space),
                  array(sorted(entries, key=lambda entry: decode(entry)[1]))])


def semantic_manifest_id(body: bytes) -> bytes:
    return domain_digest("semantic_manifest", body)


def file_entry(file_id, live, paths, epochs, contents, modes, conflicts) -> bytes:
    distinct = {path.encode("utf-8"): path for path in paths}
    return array([
        uint(PROFILE_VERSION),
        byt(file_id),
        array(uint(tag) for tag in sorted(set(live))),
        array(text(distinct[raw]) for raw in sorted(distinct)),
        array(byt(epoch) for epoch in sorted(set(epochs))),
        array(byt(content) for content in sorted(set(contents))),
        array(uint(tag) for tag in sorted(set(modes))),
        array(byt(item) for item in sorted(set(conflicts))),
    ])


# --------------------------------------------------------------------------
# DT actor names (spec §12)
# --------------------------------------------------------------------------

_B64URL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"


def base64url_unpadded_43(digest: bytes) -> str:
    """RFC 4648 section 5 alphabet, no padding, exactly 43 characters for 32 bytes."""
    if len(digest) != 32:
        raise ProfileError("invalid_encoding", "actor name digest must be 32 bytes")
    encoded = base64.urlsafe_b64encode(digest).decode("ascii").rstrip("=")
    assert len(encoded) == 43 and set(encoded) <= set(_B64URL)
    return encoded


def seed_actor_name(epoch: bytes) -> str:
    return "s:" + base64url_unpadded_43(domain_digest("dt_seed_actor", epoch))


def writer_actor_name(public_key, branch, writer_epoch, epoch) -> str:
    return "w:" + base64url_unpadded_43(
        domain_digest("dt_actor", public_key, branch, writer_epoch, epoch))


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
# Ed25519 (RFC 8032 reference procedure; fixtures only)
# --------------------------------------------------------------------------

_P = 2 ** 255 - 19
_L = 2 ** 252 + 27742317777372353535851937790883648493
_D = -121665 * pow(121666, _P - 2, _P) % _P
_I = pow(2, (_P - 1) // 4, _P)


def _inv(value: int) -> int:
    return pow(value, _P - 2, _P)


def _recover_x(y: int) -> int:
    xx = (y * y - 1) * _inv(_D * y * y + 1) % _P
    x = pow(xx, (_P + 3) // 8, _P)
    if (x * x - xx) % _P != 0:
        x = x * _I % _P
    if x % 2 != 0:
        x = _P - x
    return x


_BY = 4 * _inv(5) % _P
_BX = _recover_x(_BY)
_BASE = (_BX % _P, _BY % _P, 1, _BX * _BY % _P)


def _add(point, other):
    x1, y1, z1, t1 = point
    x2, y2, z2, t2 = other
    a = (y1 - x1) * (y2 - x2) % _P
    b = (y1 + x1) * (y2 + x2) % _P
    c = t1 * 2 * _D * t2 % _P
    d = z1 * 2 * z2 % _P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % _P, g * h % _P, f * g % _P, e * h % _P)


def _multiply(point, scalar: int):
    result = (0, 1, 1, 0)
    while scalar > 0:
        if scalar & 1:
            result = _add(result, point)
        point = _add(point, point)
        scalar >>= 1
    return result


def _encode_point(point) -> bytes:
    x, y, z, _ = point
    z_inv = _inv(z)
    x, y = x * z_inv % _P, y * z_inv % _P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def _decode_point(encoded: bytes):
    if len(encoded) != 32:
        raise ProfileError("invalid_signature", "Ed25519 point has wrong width")
    value = int.from_bytes(encoded, "little")
    y, sign = value & ((1 << 255) - 1), value >> 255
    x = _recover_x(y)
    if x & 1 != sign:
        x = _P - x
    if (-x * x + y * y - 1 - _D * x * x * y * y) % _P != 0:
        raise ProfileError("invalid_signature", "Ed25519 point is not on the curve")
    return (x, y, 1, x * y % _P)


def _ed25519_scalar(expanded: bytes) -> int:
    """RFC 8032 clamping; without it a self-consistent but non-standard keypair."""
    raw = bytearray(expanded[:32])
    raw[0] &= 248
    raw[31] &= 127
    raw[31] |= 64
    return int.from_bytes(raw, "little")


def ed25519_public_key(seed: bytes) -> bytes:
    if len(seed) != 32:
        raise ProfileError("invalid_signature", "Ed25519 seed has wrong width")
    secret = _ed25519_scalar(hashlib.sha512(seed).digest())
    return _encode_point(_multiply(_BASE, secret))


def ed25519_sign(message: bytes, seed: bytes) -> bytes:
    if len(seed) != 32:
        raise ProfileError("invalid_signature", "Ed25519 seed has wrong width")
    expanded = hashlib.sha512(seed).digest()
    secret = _ed25519_scalar(expanded)
    prefix = expanded[32:]
    public = _encode_point(_multiply(_BASE, secret))
    nonce = int.from_bytes(hashlib.sha512(prefix + message).digest(), "little") % _L
    encoded_r = _encode_point(_multiply(_BASE, nonce))
    challenge = int.from_bytes(
        hashlib.sha512(encoded_r + public + message).digest(), "little") % _L
    return encoded_r + ((nonce + challenge * secret) % _L).to_bytes(32, "little")


def ed25519_verify(message: bytes, signature: bytes, public_key: bytes) -> bool:
    """R + H(R,A,m)A == S B, which is the fixture-level check only."""
    if len(signature) != 64 or len(public_key) != 32:
        return False
    encoded_r, encoded_s = signature[:32], signature[32:]
    scalar_s = int.from_bytes(encoded_s, "little")
    if scalar_s >= _L:
        return False
    try:
        point_r = _decode_point(encoded_r)
        point_a = _decode_point(public_key)
    except ProfileError:
        return False
    challenge = int.from_bytes(
        hashlib.sha512(encoded_r + public_key + message).digest(), "little") % _L
    return _encode_point(_multiply(_BASE, scalar_s)) == _encode_point(
        _add(point_r, _multiply(point_a, challenge)))


# --------------------------------------------------------------------------
# Record validators (spec §4 - §9)
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
        _text(profile, "seed profile")
        if profile != TEXT_PROFILE:
            raise ProfileError("unsupported_capability",
                               "only dt-text-v1 is a frozen text profile")
    elif profile is not None:
        raise ProfileError("invalid_encoding",
                           "non-text epochs must not declare a native seed profile")
    if record[6] is not None:
        validate_git_oid(record[6])


def _validate_operation(item) -> None:
    if not isinstance(item, list) or not item:
        raise ProfileError("invalid_encoding", "operation is not a non-empty array")
    tag = _tag(item[0], OPERATION_TAGS, "operation tag")
    if tag == 0:
        record = _list(item, 6, "create operation")
        _bytes(record[1], 16, "repository")
        _bytes(record[2], 16, "file")
        validate_path(_text(record[3], "path", MAX_PATH_BYTES))
        _tag(record[4], MODE_TAGS, "mode tag")
        validate_epoch_descriptor(record[5])
    elif tag == 1:
        record = _list(item, 3, "path operation")
        _bytes(record[1], 16, "file")
        validate_path(_text(record[2], "path", MAX_PATH_BYTES))
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
        _validate_operation(operation)
    assertions = record[2]
    if not isinstance(assertions, list):
        raise ProfileError("invalid_encoding", "context assertions is not an array")
    if len(assertions) > MAX_CONTEXT_ASSERTIONS:
        raise ProfileError("invalid_encoding", "context assertions exceed the cap")


def validate_batch_body(item) -> None:
    record = _list(item, 11, "batch_body")
    _version(record[0])
    _bytes(record[1], 16, "space")
    _bytes(record[2], 16, "branch")
    _bytes(record[3], 32, "public key")
    _bytes(record[4], 16, "writer epoch")
    sequence = _uint(record[5], "sequence")
    previous = record[6]
    if previous is not None:
        _bytes(previous, 32, "previous batch")
    parents = record[7]
    if not isinstance(parents, list):
        raise ProfileError("invalid_encoding", "parents is not an array")
    if len(parents) > MAX_BATCH_PARENTS:
        raise ProfileError("invalid_encoding", "parents exceed the profile cap")
    for parent in parents:
        _bytes(parent, 32, "parent")
    _sorted_unique(parents, "parents")
    lamport = _uint(record[8], "lamport")
    if sequence < 1:
        raise ProfileError("invalid_encoding", "sequences start at 1")
    if previous is None:
        if sequence != 1:
            raise ProfileError("invalid_encoding",
                               "a stream's first batch has sequence 1")
        if parents:
            raise ProfileError("invalid_encoding",
                               "a genesis batch cannot declare parents")
    elif not parents:
        raise ProfileError("invalid_encoding",
                           "previous must be inside the declared parent closure")
    if not parents and lamport != 1:
        raise ProfileError("invalid_encoding", "a genesis batch has lamport 1")
    if lamport < 1:
        raise ProfileError("invalid_encoding", "lamport is at least 1")
    artifacts = record[9]
    if not isinstance(artifacts, list):
        raise ProfileError("invalid_encoding", "native artifacts is not an array")
    if len(artifacts) > MAX_BATCH_NATIVE_ARTIFACTS:
        raise ProfileError("invalid_encoding", "native artifacts exceed the cap")
    for artifact in artifacts:
        _bytes(artifact, 32, "native artifact")
    _bytes(record[10], 32, "metadata artifact")


def validate_batch_signature(item, preimage: bytes) -> None:
    record = _list(item, 3, "batch_signature")
    _version(record[0])
    public_key = _bytes(record[1], 32, "signing key")
    signature = _bytes(record[2], 64, "signature")
    if not ed25519_verify(preimage, signature, public_key):
        raise ProfileError("invalid_signature",
                           "signature does not verify for the declared preimage")


def validate_frontier(item) -> None:
    record = _list(item, 3, "frontier")
    _version(record[0])
    _bytes(record[1], 16, "space")
    heads = record[2]
    if not isinstance(heads, list):
        raise ProfileError("invalid_encoding", "heads is not an array")
    if len(heads) > MAX_FRONTIER_HEADS:
        raise ProfileError("invalid_encoding", "heads exceed the profile cap")
    for head in heads:
        _bytes(head, 32, "head")
    _sorted_unique(heads, "frontier heads")


def validate_artifact_manifest(item) -> None:
    record = _list(item, 5, "artifact_manifest")
    _version(record[0])
    kind = _text(record[1], "artifact kind")
    if kind in RESERVED_KINDS:
        raise ProfileError("unsupported_capability",
                           "reserved artifact kinds are not decodable here")
    if kind not in ASSIGNED_KINDS:
        raise ProfileError("unsupported_capability", "unassigned artifact kind")
    raw_length = _uint(record[2], "raw length")
    if raw_length > MAX_ARTIFACT_RAW_BYTES:
        raise ProfileError("invalid_encoding",
                           "raw_length exceeds the profile cap")
    _bytes(record[3], 32, "raw digest")
    chunks = record[4]
    if not isinstance(chunks, list):
        raise ProfileError("invalid_encoding", "chunk list is not an array")
    if len(chunks) > MAX_ARTIFACT_CHUNKS:
        raise ProfileError("invalid_encoding", "chunk list exceeds the profile cap")
    for chunk in chunks:
        _bytes(chunk, 32, "chunk id")
    expected_chunks = -(-raw_length // CHUNK_RAW_BYTES) if raw_length else 0
    if len(chunks) != expected_chunks:
        raise ProfileError("hash_mismatch",
                           "declared length disagrees with the chunk list")


def validate_native_artifact(item) -> None:
    record = _list(item, 4, "native_artifact")
    _version(record[0])
    _bytes(record[1], 32, "epoch")
    profile = _text(record[2], "native profile")
    if profile != TEXT_PROFILE:
        raise ProfileError("unsupported_capability",
                           "only the dt-text-v1 profile is frozen")
    if not isinstance(record[3], bytes):
        raise ProfileError("invalid_encoding", "native bytes is not a byte string")
    if len(record[3]) > MAX_ARTIFACT_RAW_BYTES:
        raise ProfileError("invalid_encoding", "native bytes exceed the profile cap")


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


def validate_branch_descriptor(item) -> None:
    record = _list(item, 5, "branch_descriptor")
    _version(record[0])
    _bytes(record[1], 16, "space")
    _bytes(record[2], 16, "branch")
    _bytes(record[3], 32, "initial version")
    _bytes(record[4], 32, "creator key")


def validate_branch_envelope(item) -> None:
    record = _list(item, 3, "branch_envelope")
    _version(record[0])
    body = record[1]
    if not isinstance(body, bytes):
        raise ProfileError("invalid_encoding", "descriptor body is not bytes")
    validate_branch_descriptor(decode(body))
    signature = _bytes(record[2], 64, "signature")
    creator = decode(body)[4]
    if not ed25519_verify(branch_signature_preimage(body), signature, creator):
        raise ProfileError("invalid_signature",
                           "descriptor signature does not verify")


def validate_file_entry(item) -> None:
    record = _list(item, 8, "file_entry")
    _version(record[0])
    _bytes(record[1], 16, "file")
    groups = (
        (record[2], "live tags", LIVE_TAGS),
        (record[6], "mode tags", MODE_TAGS),
    )
    for values, what, allowed in groups:
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
    "batch_body": validate_batch_body,
    "batch_signature": validate_batch_signature,
    "frontier": validate_frontier,
    "artifact_manifest": validate_artifact_manifest,
    "native_artifact": validate_native_artifact,
    "metadata_artifact": validate_metadata_artifact,
    "epoch_descriptor": validate_epoch_descriptor,
    "git_oid": validate_git_oid,
    "conflict": validate_conflict,
    "branch_descriptor": validate_branch_descriptor,
    "branch_envelope": validate_branch_envelope,
    "semantic_manifest": validate_semantic_manifest,
    "file_entry": validate_file_entry,
}


def validate_record(record: str, data: bytes, **context) -> None:
    """Decode and validate one frozen record, raising ProfileError on failure."""
    if record not in VALIDATORS:
        raise KeyError(f"unknown record {record}")
    item = decode(data)
    validator = VALIDATORS[record]
    if record == "batch_signature":
        validator(item, context["preimage"])
    else:
        validator(item)


# --------------------------------------------------------------------------
# Deterministic fixture data
# --------------------------------------------------------------------------

def _fixture_bytes(label: str, width: int) -> bytes:
    digest = hashlib.sha256(b"puyudb/canonical-v1/fixture/" + label.encode()).digest()
    return digest[:width]


SPACE = _fixture_bytes("space", 16)
BRANCH = _fixture_bytes("branch", 16)
FILE = _fixture_bytes("file", 16)
NONCE = _fixture_bytes("generation-nonce", 16)
WRITER_EPOCH = _fixture_bytes("writer-epoch", 16)
EPOCH = _fixture_bytes("epoch", 32)
SIGNING_SEED = _fixture_bytes("signing-seed", 32)
OTHER_SIGNING_SEED = _fixture_bytes("other-signing-seed", 32)
PUBLIC_KEY = ed25519_public_key(SIGNING_SEED)
OTHER_PUBLIC_KEY = ed25519_public_key(OTHER_SIGNING_SEED)

BASELINE_TEXT = b"PuyuDB canonical vector"
PARENTS = sorted([_fixture_bytes("parent-a", 32), _fixture_bytes("parent-b", 32)])
PARENT_A, PARENT_B = PARENTS


def _metadata_operations() -> list:
    return [
        array([uint(0), byt(_fixture_bytes("repository", 16)), byt(FILE),
               text("docs/canonical.md"), uint(0),
               epoch_descriptor(FILE, NONCE, 0, _fixture_bytes("baseline", 32),
                                TEXT_PROFILE,
                                git_oid(1, _fixture_bytes("git-oid", 20)))]),
        array([uint(1), byt(FILE), text("docs/canonical-renamed.md")]),
        array([uint(2), byt(FILE), uint(1)]),
        array([uint(3), byt(FILE),
               epoch_descriptor(FILE, _fixture_bytes("nonce-2", 16), 0,
                                _fixture_bytes("baseline", 32), TEXT_PROFILE, None)]),
        array([uint(4), byt(FILE), byt(_fixture_bytes("opaque-epoch", 32)),
               byt(_fixture_bytes("opaque-artifact", 32))]),
        array([uint(5), uint(1), byt(_fixture_bytes("event", 16)),
               byt(b"thread-vector")]),
        array([uint(6), byt(_fixture_bytes("repository", 16)),
               git_oid(2, _fixture_bytes("git-oid-256", 32)),
               byt(_fixture_bytes("version", 32)), uint(0)]),
        array([uint(7), byt(FILE), uint(1)]),
        array([uint(8), byt(_fixture_bytes("conflict-id", 32)), uint(2), byt(EPOCH)]),
        array([uint(9), array([byt(_fixture_bytes("source-batch", 32))]), uint(1), NULL]),
    ]


def build_vectors() -> dict:
    """Every frozen record once, with its expected digests."""
    metadata = metadata_artifact(_metadata_operations(), [])
    native_payload = b"DMNDTYPS-vector-native-bytes"
    native = native_artifact(EPOCH, TEXT_PROFILE, native_payload)
    empty_native = native_artifact(EPOCH, TEXT_PROFILE, b"")
    opaque_payload = b"\x5a" * (CHUNK_RAW_BYTES + 7)
    baseline_manifest = manifest("text-baseline-v1", BASELINE_TEXT)
    epoch_body = epoch_descriptor(
        FILE, NONCE, 0, artifact_id(baseline_manifest), TEXT_PROFILE,
        git_oid(1, _fixture_bytes("git-oid", 20)))
    opaque_epoch = epoch_descriptor(
        FILE, _fixture_bytes("nonce-opaque", 16), 1,
        artifact_id(manifest("opaque-baseline-v1", b"\x00\x01")), None, None)
    body = batch_body(
        SPACE, BRANCH, PUBLIC_KEY, WRITER_EPOCH, 7, PARENT_A, PARENTS, 12,
        [artifact_id(manifest("native-change-v1", native)),
         artifact_id(manifest("native-change-v1", empty_native))],
        artifact_id(metadata))
    genesis = batch_body(SPACE, BRANCH, PUBLIC_KEY, WRITER_EPOCH, 1, None, [], 1, [],
                         artifact_id(metadata))
    chained = batch_body(SPACE, BRANCH, PUBLIC_KEY, WRITER_EPOCH, 8, batch_id(body),
                         [batch_id(body)], 13, [], artifact_id(metadata))
    heads = sorted([_fixture_bytes("head-a", 32), _fixture_bytes("head-b", 32)])
    frontier_body = frontier(SPACE, heads)
    branch_body = branch_descriptor(SPACE, BRANCH, version_id(frontier_body),
                                    PUBLIC_KEY)
    conflicts = {
        "register": conflict(0, FILE, [], [(PARENT_A, 0), (PARENT_B, 0)]),
        "delete-edit": conflict(1, FILE, [EPOCH], [(PARENT_A, 0), (PARENT_B, 3)]),
        "epoch-edit": conflict(2, FILE, sorted([EPOCH, _fixture_bytes("epoch-2", 32)]),
                               sorted([(PARENT_A, 1), (PARENT_B, 2)])),
    }
    entry_a = file_entry(FILE, [0, 1],
                         ["docs/canonical.md", "docs/canonical-renamed.md"],
                         [EPOCH],
                         [artifact_id(manifest("opaque-baseline-v1", b"\x00\x01"))],
                         [0, 1], [conflict_id(conflicts["delete-edit"])])
    entry_b = file_entry(_fixture_bytes("file-2", 16), [1], ["README.md"], [], [], [0], [])
    semantic_body = semantic_manifest(SPACE, [entry_b, entry_a])

    batch_signature_value = ed25519_sign(
        signature_preimage(SPACE, batch_id(body)), SIGNING_SEED)
    branch_signature_value = ed25519_sign(
        branch_signature_preimage(branch_body), SIGNING_SEED)

    def digest_value(value):
        return value if isinstance(value, list) else value.hex()

    def entry(name, record, body_bytes, digests=None, **extra):
        result = {"name": name, "record": record, "body_hex": body_bytes.hex(),
                  "digests": {key: digest_value(value)
                              for key, value in (digests or {}).items()}}
        result.update(extra)
        return result

    return {
        "profile": PROFILE_VERSION,
        "generated_by": "scripts/i03_canonical_vectors.py",
        "spec": "docs/spec/canonical-profile-v1.md",
        "vectors": [
            entry("batch/minimal", "batch_body", genesis,
                  {"batch_id": batch_id(genesis)}),
            entry("batch/two-parents", "batch_body", body,
                  {"batch_id": batch_id(body)}),
            entry("batch/chained", "batch_body", chained,
                  {"batch_id": batch_id(chained)}),
            entry("batch/signature", "batch_signature",
                  batch_signature(PUBLIC_KEY, batch_signature_value), None,
                  preimage_hex=signature_preimage(SPACE, batch_id(body)).hex(),
                  signature_hex=batch_signature_value.hex(),
                  public_key_hex=PUBLIC_KEY.hex()),
            entry("frontier/two-heads", "frontier", frontier_body,
                  {"version_id": version_id(frontier_body)}),
            entry("artifact/text-baseline", "artifact_manifest", baseline_manifest,
                  {"artifact_id": artifact_id(baseline_manifest),
                   "raw_digest": raw_digest("text-baseline-v1", BASELINE_TEXT),
                   "chunk_ids": [chunk_id(chunk).hex()
                                 for chunk in chunk_payload(BASELINE_TEXT)]}),
            entry("artifact/empty", "artifact_manifest",
                  manifest("text-baseline-v1", b""),
                  {"artifact_id": artifact_id(manifest("text-baseline-v1", b"")),
                   "raw_digest": raw_digest("text-baseline-v1", b""),
                   "chunk_ids": []}),
            entry("artifact/multi-chunk", "artifact_manifest",
                  manifest("opaque-baseline-v1", opaque_payload),
                  {"artifact_id": artifact_id(manifest("opaque-baseline-v1",
                                                       opaque_payload)),
                   "raw_digest": raw_digest("opaque-baseline-v1", opaque_payload),
                   "chunk_ids": [chunk_id(chunk).hex()
                                 for chunk in chunk_payload(opaque_payload)]}),
            entry("native/text-change", "native_artifact", native),
            entry("native/empty-seed", "native_artifact", empty_native),
            entry("metadata/all-operations", "metadata_artifact", metadata),
            entry("epoch/text", "epoch_descriptor", epoch_body,
                  {"epoch_id": epoch_id(epoch_body)}),
            entry("epoch/opaque", "epoch_descriptor", opaque_epoch,
                  {"epoch_id": epoch_id(opaque_epoch)}),
            entry("conflict/register", "conflict", conflicts["register"],
                  {"conflict_id": conflict_id(conflicts["register"])}),
            entry("conflict/delete-edit", "conflict", conflicts["delete-edit"],
                  {"conflict_id": conflict_id(conflicts["delete-edit"])}),
            entry("conflict/epoch-edit", "conflict", conflicts["epoch-edit"],
                  {"conflict_id": conflict_id(conflicts["epoch-edit"])}),
            entry("branch/descriptor", "branch_descriptor", branch_body,
                  {"branch_descriptor_id": branch_descriptor_id(branch_body)}),
            entry("branch/envelope", "branch_envelope",
                  branch_envelope(branch_body, branch_signature_value), None,
                  preimage_hex=branch_signature_preimage(branch_body).hex(),
                  signature_hex=branch_signature_value.hex(),
                  public_key_hex=PUBLIC_KEY.hex()),
            entry("semantic/manifest", "semantic_manifest", semantic_body,
                  {"semantic_manifest_id": semantic_manifest_id(semantic_body)}),
            entry("git/oid-sha256", "git_oid",
                  git_oid(2, _fixture_bytes("git-oid-256", 32))),
        ],
    }


def _unsorted_semantic_manifest() -> bytes:
    """Entries deliberately out of file order; the encoder never sorts for you."""
    bigger, smaller = sorted([FILE, _fixture_bytes("file-2", 16)], reverse=True)
    return array([
        uint(PROFILE_VERSION),
        byt(SPACE),
        array([file_entry(bigger, [1], ["b.md"], [], [], [0], []),
               file_entry(smaller, [1], ["a.md"], [], [], [0], [])]),
    ])


def build_negative_corpus() -> dict:
    """Rejected inputs with their stable codes; the fixture half of T08."""
    metadata_id = _fixture_bytes("metadata-artifact", 32)
    body = batch_body(SPACE, BRANCH, PUBLIC_KEY, WRITER_EPOCH, 1, None, [], 1, [],
                      metadata_id)
    signature = ed25519_sign(signature_preimage(SPACE, batch_id(body)), SIGNING_SEED)
    wrong_signature = ed25519_sign(signature_preimage(SPACE, batch_id(body)),
                                  OTHER_SIGNING_SEED)

    def case(name, record, data, code, why, **extra):
        result = {"name": name, "record": record, "body_hex": data.hex(),
                  "code": code, "why": why}
        result.update(extra)
        return result

    def genesis(**overrides):
        fields = dict(space=SPACE, branch=BRANCH, public_key=PUBLIC_KEY,
                      writer_epoch=WRITER_EPOCH, sequence=1, previous=None,
                      parents=[], lamport=1, native_artifact_ids=[],
                      metadata_artifact=metadata_id)
        fields.update(overrides)
        return batch_body(**fields)

    cases = [
        case("encoding/trailing-bytes", "batch_body", body + b"\x00",
             "invalid_encoding", "trailing bytes after a record"),
        case("encoding/truncated", "batch_body", body[:-1],
             "invalid_encoding", "record cut short"),
        case("encoding/non-minimal-int", "frontier", b"\x82\x01\x18\x00",
             "invalid_encoding", "uint 0 encoded in a 1-byte form"),
        case("encoding/indefinite-array", "frontier",
             b"\x9f\x01\x50" + SPACE + b"\x80\xff",
             "invalid_encoding", "indefinite length"),
        case("encoding/float", "frontier",
             array([uint(PROFILE_VERSION), byt(SPACE),
                    b"\xfb\x3f\xf0\x00\x00\x00\x00\x00\x00"]),
             "invalid_encoding", "floats are outside the profile"),
        case("encoding/map", "frontier", b"\xa1\x01\x02",
             "invalid_encoding", "maps are outside the profile"),
        case("encoding/cbor-tag", "frontier", b"\xc1\x01",
             "invalid_encoding", "CBOR tag items are outside the profile"),
        case("shape/batch-space-width", "batch_body", genesis(space=SPACE[:15]),
             "invalid_encoding", "space must be 16 bytes"),
        case("shape/batch-sequence-zero", "batch_body", genesis(sequence=0),
             "invalid_encoding", "sequences start at 1"),
        case("shape/batch-first-sequence", "batch_body", genesis(sequence=4),
             "invalid_encoding", "a stream's first batch has sequence 1"),
        case("shape/batch-genesis-lamport", "batch_body", genesis(lamport=5),
             "invalid_encoding", "a genesis batch has lamport 1"),
        case("shape/batch-genesis-parents", "batch_body",
             genesis(parents=[PARENT_A], lamport=2),
             "invalid_encoding", "a genesis batch cannot declare parents"),
        case("shape/batch-previous-without-parents", "batch_body",
             genesis(sequence=2, previous=PARENT_A),
             "invalid_encoding", "previous must be inside the parent closure"),
        case("shape/batch-too-many-parents", "batch_body",
             genesis(sequence=2, previous=batch_id(body),
                     parents=sorted(_fixture_bytes(f"parent-{index}", 32)
                                    for index in range(MAX_BATCH_PARENTS + 1)),
                     lamport=2),
             "invalid_encoding", "parent array exceeds the profile cap"),
        case("shape/batch-unsorted-parents", "batch_body",
             genesis(sequence=2, previous=batch_id(body),
                     parents=list(reversed(PARENTS)), lamport=2),
             "invalid_encoding", "parents must be sorted and unique"),
        case("shape/batch-duplicate-parents", "batch_body",
             genesis(sequence=2, previous=batch_id(body),
                     parents=[PARENT_A, PARENT_A], lamport=2),
             "invalid_encoding", "parents must be sorted and unique"),
        case("shape/frontier-unsorted", "frontier",
             frontier(SPACE, list(reversed(PARENTS))),
             "invalid_encoding", "frontier heads must be sorted"),
        case("shape/frontier-duplicate", "frontier",
             frontier(SPACE, [PARENT_A, PARENT_A]),
             "invalid_encoding", "frontier heads must be unique"),
        case("shape/manifest-length-mismatch", "artifact_manifest",
             array([uint(PROFILE_VERSION), text("text-baseline-v1"),
                    uint(len(BASELINE_TEXT) + CHUNK_RAW_BYTES),
                    byt(raw_digest("text-baseline-v1", BASELINE_TEXT)),
                    array([byt(chunk_id(BASELINE_TEXT))])]),
             "hash_mismatch", "declared length disagrees with the chunk list"),
        case("shape/manifest-missing-chunks", "artifact_manifest",
             array([uint(PROFILE_VERSION), text("opaque-baseline-v1"),
                    uint(CHUNK_RAW_BYTES), byt(b"\x00" * 32), array([])]),
             "hash_mismatch", "declared length without chunks"),
        case("shape/manifest-empty-with-length", "artifact_manifest",
             array([uint(PROFILE_VERSION), text("text-baseline-v1"), uint(1),
                    byt(b"\x00" * 32), array([])]),
             "hash_mismatch", "non-empty length with an empty chunk list"),
        case("shape/manifest-raw-length-over-cap", "artifact_manifest",
             array([uint(PROFILE_VERSION), text("opaque-baseline-v1"),
                    uint(MAX_ARTIFACT_RAW_BYTES + 1), byt(b"\x00" * 32),
                    array([])]),
             "invalid_encoding", "raw_length is checked before allocation"),
        case("tag/unknown-kind", "artifact_manifest",
             array([uint(PROFILE_VERSION), text("unknown-kind-v1"), uint(0),
                    byt(b"\x00" * 32), array([])]),
             "unsupported_capability", "unassigned artifact kind"),
        case("tag/reserved-kind", "artifact_manifest",
             array([uint(PROFILE_VERSION), text("named-snapshot-v1"), uint(0),
                    byt(b"\x00" * 32), array([])]),
             "unsupported_capability", "reserved kind is not decodable here"),
        case("tag/unknown-operation", "metadata_artifact",
             metadata_artifact([array([uint(10), byt(FILE)])], []),
             "unsupported_capability", "operation tag 10 is unassigned"),
        case("tag/unknown-mode", "metadata_artifact",
             metadata_artifact([array([uint(7), byt(FILE), uint(3)])], []),
             "unsupported_capability", "mode tag 3 is unassigned"),
        case("tag/unknown-live", "metadata_artifact",
             metadata_artifact([array([uint(2), byt(FILE), uint(2)])], []),
             "unsupported_capability", "liveness tag 2 is unassigned"),
        case("tag/unknown-resolution", "metadata_artifact",
             metadata_artifact([array([uint(8), byt(_fixture_bytes("conflict", 32)),
                                       uint(5), NULL])], []),
             "unsupported_capability", "resolution tag 5 is unassigned"),
        case("tag/unknown-context-schema", "metadata_artifact",
             metadata_artifact([array([uint(5), uint(8),
                                       byt(_fixture_bytes("event", 16)), byt(b"x")])],
                               []),
             "unsupported_capability", "context schema tag 8 is unassigned"),
        case("tag/unknown-epoch-kind", "epoch_descriptor",
             epoch_descriptor(FILE, NONCE, 3, _fixture_bytes("baseline", 32), None, None),
             "unsupported_capability", "epoch kind tag 3 is unassigned"),
        case("tag/profile-version-2", "batch_body",
             array([uint(2), byt(SPACE), byt(BRANCH), byt(PUBLIC_KEY),
                    byt(WRITER_EPOCH), uint(1), NULL, array([]), uint(1),
                    array([]), byt(metadata_id)]),
             "unsupported_version", "only profile version 1 is frozen"),
        case("tag/am-seed-profile", "epoch_descriptor",
             epoch_descriptor(FILE, NONCE, 0, _fixture_bytes("baseline", 32),
                              "am-text-v1", None),
             "unsupported_capability", "AM profile string is reserved and unused"),
        case("shape/epoch-text-without-seed-profile", "epoch_descriptor",
             epoch_descriptor(FILE, NONCE, 0, _fixture_bytes("baseline", 32), None, None),
             "invalid_encoding", "text epochs require the DT seed profile"),
        case("shape/epoch-opaque-with-seed-profile", "epoch_descriptor",
             epoch_descriptor(FILE, NONCE, 1, _fixture_bytes("baseline", 32),
                              TEXT_PROFILE, None),
             "invalid_encoding", "non-text epochs have no native seed"),
        case("shape/git-oid-width", "git_oid",
             array([uint(PROFILE_VERSION), uint(1), byt(b"\x00" * 32)]),
             "invalid_encoding", "SHA-1 OIDs are 20 bytes"),
        case("tag/unknown-git-algorithm", "git_oid",
             array([uint(PROFILE_VERSION), uint(3), byt(b"\x00" * 32)]),
             "unsupported_capability", "Git OID algorithm 3 is unassigned"),
        case("shape/conflict-unsorted-participants", "conflict",
             conflict(1, FILE, [EPOCH], [(EPOCH, 1), (FILE, 0)]),
             "invalid_encoding", "participants sort by batch then index"),
        case("shape/conflict-duplicate-epochs", "conflict",
             conflict(2, FILE, [EPOCH, EPOCH], [(FILE, 0)]),
             "invalid_encoding", "epoch list must be unique"),
        case("shape/conflict-register-with-epoch", "conflict",
             conflict(0, FILE, [EPOCH], [(FILE, 0)]),
             "invalid_encoding", "register conflicts concern no epoch"),
        case("shape/conflict-operation-index-cap", "conflict",
             conflict(1, FILE, [EPOCH], [(FILE, MAX_OPERATION_INDEX + 1)]),
             "invalid_encoding", "operation index exceeds the profile cap"),
        case("tag/unknown-conflict-kind", "conflict",
             conflict(3, FILE, [], [(FILE, 0)]),
             "unsupported_capability", "conflict kind 3 is unassigned"),
        case("shape/native-wrong-profile", "native_artifact",
             native_artifact(EPOCH, "am-text-v1", b"x"),
             "unsupported_capability", "only dt-text-v1 is frozen"),
        case("shape/metadata-path-absolute", "metadata_artifact",
             metadata_artifact([array([uint(1), byt(FILE), text("/etc/passwd")])], []),
             "invalid_encoding", "absolute paths are invalid"),
        case("shape/metadata-control-segment", "metadata_artifact",
             metadata_artifact([array([uint(1), byt(FILE), text("docs/.git/config")])], []),
             "invalid_encoding", "reserved control segment"),
        case("shape/metadata-resolution-payload", "metadata_artifact",
             metadata_artifact([array([uint(8), byt(_fixture_bytes("conflict", 32)),
                                       uint(0), byt(b"x")])], []),
             "invalid_encoding", "retain-deletion takes a null payload"),
        case("shape/semantic-unsorted-entries", "semantic_manifest",
             _unsorted_semantic_manifest(),
             "invalid_encoding", "manifest entries sort by raw file id"),
        case("signature/wrong-key", "batch_signature",
             batch_signature(PUBLIC_KEY, wrong_signature),
             "invalid_signature", "signature does not verify for the preimage",
             preimage_hex=signature_preimage(SPACE, batch_id(body)).hex()),
        case("signature/non-canonical-s", "batch_signature",
             batch_signature(PUBLIC_KEY, signature[:32] + _L.to_bytes(32, "little")),
             "invalid_signature", "S must be below the group order",
             preimage_hex=signature_preimage(SPACE, batch_id(body)).hex()),
        case("signature/truncated", "batch_signature",
             batch_signature(PUBLIC_KEY, signature[:63]),
             "invalid_encoding", "signatures are 64 bytes",
             preimage_hex=signature_preimage(SPACE, batch_id(body)).hex()),
        case("signature/branch-mismatch", "branch_envelope",
             branch_envelope(branch_descriptor(SPACE, BRANCH,
                                               _fixture_bytes("version", 32),
                                               PUBLIC_KEY),
                             ed25519_sign(branch_signature_preimage(
                                 branch_descriptor(SPACE, BRANCH,
                                                   _fixture_bytes("other-version", 32),
                                                   PUBLIC_KEY)), SIGNING_SEED)),
             "invalid_signature", "descriptor signature binds the exact body"),
    ]
    return {
        "profile": PROFILE_VERSION,
        "generated_by": "scripts/i03_canonical_vectors.py",
        "scope": (
            "Record-level negatives only. DAG-level rules (equivocation, "
            "ancestor dominance, native scope) need history and are tested in T4/T5."
        ),
        "cases": cases,
        "path_cases": [
            {"name": "reject-absolute", "value": "/absolute", "accepted": False},
            {"name": "reject-dot-dot", "value": "a/../b", "accepted": False},
            {"name": "reject-empty-component", "value": "a//b", "accepted": False},
            {"name": "reject-trailing-slash", "value": "a/", "accepted": False},
            {"name": "reject-empty", "value": "", "accepted": False},
            {"name": "reject-dot", "value": "a/./b", "accepted": False},
            {"name": "reject-git-segment", "value": "a/.git/config", "accepted": False},
            {"name": "reject-puyu-segment", "value": "a/.puyu/state", "accepted": False},
            {"name": "reject-nul", "value": "a/b\x00c", "accepted": False},
            {"name": "reject-component-cap",
             "value": "a" * (MAX_PATH_COMPONENT_BYTES + 1), "accepted": False},
            {"name": "reject-path-cap", "value": "/".join(["a"] * 3000),
             "accepted": False},
            {"name": "accept-simple", "value": "docs/canonical.md", "accepted": True},
            {"name": "accept-case", "value": "A/b", "accepted": True},
            {"name": "accept-nfc", "value": "docs/\u00e9.md", "accepted": True},
            {"name": "accept-nfd", "value": "docs/e\u0301.md", "accepted": True},
            {"name": "accept-dotfile", "value": ".github/workflows/ci.yml",
             "accepted": True},
            {"name": "accept-component-cap", "value": "a" * MAX_PATH_COMPONENT_BYTES,
             "accepted": True},
        ],
    }


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    target = root / "tests" / "fixtures" / "canonical-v1"
    target.mkdir(parents=True, exist_ok=True)
    for name, payload in (("vectors.json", build_vectors()),
                          ("negative.json", build_negative_corpus())):
        body = json.dumps(payload, indent=2, sort_keys=True) + "\n"
        (target / name).write_text(body, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
