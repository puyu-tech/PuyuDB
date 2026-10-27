# SPDX-License-Identifier: Apache-2.0
"""Contract checks for the frozen canonical profile vectors and validator."""

import json
from pathlib import Path
import unittest

import i03_canonical_vectors as canonical

FIXTURES = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "canonical-v1"

# RFC 8032 section 7.1, TEST 1 and TEST 2. These are external anchors: if they
# ever fail, the fixture signer is not Ed25519 and no vector in this file means
# anything.
RFC8032 = (
    (
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        "",
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555f"
        "b8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
    ),
    (
        "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
        "72",
        "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da"
        "085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
    ),
)

# Frozen textual form of the DT actor names for the fixture context.
EXPECTED_SEED_NAME = "s:c_YvYXW18fQ2Iibsk_BtqVNIsicmchegnYlyr_my_5Y"
EXPECTED_WRITER_NAME = "w:ntACkklc9D0_PDozKdA6fiYklNXOODKTndwV9kgVuTk"

LOWER_HEX = set("0123456789abcdef")


def load(name):
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def bytes_of(value):
    return bytes.fromhex(value)


class Ed25519Tests(unittest.TestCase):
    def test_matches_rfc8032_known_answers(self):
        for seed_hex, message_hex, public_hex, signature_hex in RFC8032:
            seed = bytes_of(seed_hex)
            message = bytes_of(message_hex)
            public_key = canonical.ed25519_public_key(seed)
            self.assertEqual(public_key.hex(), public_hex)
            self.assertEqual(canonical.ed25519_sign(message, seed).hex(), signature_hex)
            self.assertTrue(canonical.ed25519_verify(
                message, bytes_of(signature_hex), bytes_of(public_hex)))

    def test_rejects_wrong_key_message_and_non_canonical_scalar(self):
        seed, message, public_hex, signature_hex = RFC8032[1]
        signature = bytes_of(signature_hex)
        other = canonical.ed25519_public_key(bytes_of(RFC8032[0][0]))
        self.assertFalse(canonical.ed25519_verify(bytes_of(message), signature, other))
        self.assertFalse(canonical.ed25519_verify(b"x" + bytes_of(message), signature,
                                                 bytes_of(public_hex)))
        overflow = signature[:32] + canonical._L.to_bytes(32, "little")
        self.assertFalse(canonical.ed25519_verify(bytes_of(message), overflow,
                                                 bytes_of(public_hex)))


class CanonicalEncodingTests(unittest.TestCase):
    def test_integer_encodings_are_minimal_and_round_trip(self):
        expected = {
            0: "00", 23: "17", 24: "1818", 255: "18ff", 256: "190100",
            65535: "19ffff", 65536: "1a00010000",
            4294967296: "1b0000000100000000",
        }
        for value, encoded in expected.items():
            self.assertEqual(canonical.uint(value).hex(), encoded)
            self.assertEqual(canonical.decode(canonical.uint(value)), value)

    def test_over_long_integer_forms_are_rejected(self):
        for encoded in ("1800", "190000", "1a00000000", "1b0000000000000000"):
            with self.assertRaises(canonical.ProfileError):
                canonical.decode(bytes.fromhex(encoded))

    def test_structural_non_canonical_forms_are_rejected(self):
        for encoded in ("9f01ff", "a10102", "c101", "fb3ff0000000000000",
                        "2001", "0100", "82", "48010203"):
            with self.assertRaises(canonical.ProfileError):
                canonical.decode(bytes.fromhex(encoded))

    def test_text_requires_valid_utf8(self):
        with self.assertRaises(canonical.ProfileError):
            canonical.decode(bytes.fromhex("62ff80"))


class VectorTests(unittest.TestCase):
    def test_committed_fixtures_match_the_generator(self):
        for name, payload in (("vectors.json", canonical.build_vectors()),
                              ("negative.json", canonical.build_negative_corpus())):
            committed = (FIXTURES / name).read_text(encoding="utf-8")
            self.assertEqual(committed,
                             json.dumps(payload, indent=2, sort_keys=True) + "\n",
                             f"{name} drifted from the generator")

    def test_every_vector_validates(self):
        for vector in load("vectors.json")["vectors"]:
            context = {}
            if "preimage_hex" in vector:
                context["preimage"] = bytes_of(vector["preimage_hex"])
            with self.subTest(vector=vector["name"]):
                canonical.validate_record(vector["record"],
                                          bytes_of(vector["body_hex"]), **context)

    def test_digests_rederive_from_the_frozen_layouts(self):
        rederive = {
            "batch_body": {"batch_id": canonical.batch_id},
            "frontier": {"version_id": canonical.version_id},
            "artifact_manifest": {"artifact_id": canonical.artifact_id},
            "epoch_descriptor": {"epoch_id": canonical.epoch_id},
            "conflict": {"conflict_id": canonical.conflict_id},
            "branch_descriptor": {"branch_descriptor_id":
                                  canonical.branch_descriptor_id},
            "semantic_manifest": {"semantic_manifest_id":
                                  canonical.semantic_manifest_id},
        }
        checked = 0
        for vector in load("vectors.json")["vectors"]:
            if vector["record"] not in rederive:
                continue
            body = bytes_of(vector["body_hex"])
            for key, function in rederive[vector["record"]].items():
                with self.subTest(vector=vector["name"], digest=key):
                    self.assertEqual(vector["digests"][key], function(body).hex())
                    checked += 1
        self.assertGreaterEqual(checked, 9)

    def test_artifact_payload_digests_rederive(self):
        payloads = {
            "artifact/text-baseline": canonical.BASELINE_TEXT,
            "artifact/empty": b"",
        }
        vectors = {vector["name"]: vector for vector in load("vectors.json")["vectors"]}
        for name, payload in payloads.items():
            vector = vectors[name]
            kind = "text-baseline-v1"
            with self.subTest(vector=name):
                self.assertEqual(vector["digests"]["raw_digest"],
                                 canonical.raw_digest(kind, payload).hex())
                self.assertEqual(vector["digests"]["chunk_ids"],
                                 [canonical.chunk_id(chunk).hex()
                                  for chunk in canonical.chunk_payload(payload)])
                self.assertEqual(vector["body_hex"],
                                 canonical.manifest(kind, payload).hex())

    def test_signature_vectors_verify_and_are_embedded(self):
        vectors = {vector["name"]: vector for vector in load("vectors.json")["vectors"]}
        for name in ("batch/signature", "branch/envelope"):
            vector = vectors[name]
            with self.subTest(vector=name):
                self.assertTrue(canonical.ed25519_verify(
                    bytes_of(vector["preimage_hex"]),
                    bytes_of(vector["signature_hex"]),
                    bytes_of(vector["public_key_hex"])))
                self.assertIn(vector["signature_hex"], vector["body_hex"])
                self.assertIn(vector["public_key_hex"], vector["body_hex"])

    def test_unknown_record_name_is_a_programming_error(self):
        with self.assertRaises(KeyError):
            canonical.validate_record("unknown", b"\x01")


class NegativeCorpusTests(unittest.TestCase):
    def test_every_case_is_rejected_with_its_declared_code(self):
        corpus = load("negative.json")
        self.assertGreaterEqual(len(corpus["cases"]), 40)
        for case in corpus["cases"]:
            context = {}
            if "preimage_hex" in case:
                context["preimage"] = bytes_of(case["preimage_hex"])
            with self.subTest(case=case["name"]):
                with self.assertRaises(canonical.ProfileError) as raised:
                    canonical.validate_record(case["record"],
                                              bytes_of(case["body_hex"]), **context)
                self.assertEqual(raised.exception.code, case["code"],
                                 f"{case['name']}: {raised.exception}")

    def test_negative_corpus_covers_every_assigned_code(self):
        codes = {case["code"] for case in load("negative.json")["cases"]}
        self.assertEqual(codes,
                         {"invalid_encoding", "unsupported_capability",
                          "unsupported_version", "invalid_signature", "hash_mismatch"})

    def test_path_rules(self):
        for case in load("negative.json")["path_cases"]:
            with self.subTest(case=case["name"]):
                if case["accepted"]:
                    canonical.validate_path(case["value"])
                else:
                    with self.assertRaises(canonical.ProfileError):
                        canonical.validate_path(case["value"])


class IdentityTests(unittest.TestCase):
    def test_actor_names_are_forty_five_ascii_bytes(self):
        names = (canonical.seed_actor_name(canonical.EPOCH),
                 canonical.writer_actor_name(canonical.PUBLIC_KEY, canonical.BRANCH,
                                             canonical.WRITER_EPOCH, canonical.EPOCH))
        for name in names:
            self.assertEqual(len(name.encode()), 45)
            self.assertLess(len(name.encode()), 50)
            self.assertTrue(name.isascii())
            self.assertNotEqual(name, "ROOT")
            self.assertTrue(set(name[2:]) <= set(canonical._B64URL))

    def test_actor_names_are_frozen_literals(self):
        self.assertEqual(canonical.seed_actor_name(canonical.EPOCH),
                         EXPECTED_SEED_NAME)
        self.assertEqual(
            canonical.writer_actor_name(canonical.PUBLIC_KEY, canonical.BRANCH,
                                        canonical.WRITER_EPOCH, canonical.EPOCH),
            EXPECTED_WRITER_NAME)

    def test_writer_name_binds_every_context_field(self):
        base = canonical.writer_actor_name(canonical.PUBLIC_KEY, canonical.BRANCH,
                                           canonical.WRITER_EPOCH, canonical.EPOCH)
        variants = (
            canonical.writer_actor_name(canonical.OTHER_PUBLIC_KEY, canonical.BRANCH,
                                        canonical.WRITER_EPOCH, canonical.EPOCH),
            canonical.writer_actor_name(canonical.PUBLIC_KEY,
                                        canonical._fixture_bytes("branch-2", 16),
                                        canonical.WRITER_EPOCH, canonical.EPOCH),
            canonical.writer_actor_name(canonical.PUBLIC_KEY, canonical.BRANCH,
                                        canonical._fixture_bytes("writer-epoch-2", 16),
                                        canonical.EPOCH),
            canonical.writer_actor_name(canonical.PUBLIC_KEY, canonical.BRANCH,
                                        canonical.WRITER_EPOCH,
                                        canonical._fixture_bytes("epoch-2", 32)),
        )
        self.assertEqual(len(set(variants + (base,))), 5)

    def test_seed_and_writer_namespaces_do_not_overlap(self):
        seed = canonical.seed_actor_name(canonical.EPOCH)
        writer = canonical.writer_actor_name(canonical.PUBLIC_KEY, canonical.BRANCH,
                                             canonical.WRITER_EPOCH, canonical.EPOCH)
        self.assertTrue(seed.startswith("s:"))
        self.assertTrue(writer.startswith("w:"))
        self.assertNotEqual(seed[2:], writer[2:])

    def test_empty_seed_is_epoch_bound_without_a_sentinel(self):
        other_epoch = canonical._fixture_bytes("epoch-2", 32)
        first = canonical.native_artifact(canonical.EPOCH, canonical.TEXT_PROFILE, b"")
        second = canonical.native_artifact(other_epoch, canonical.TEXT_PROFILE, b"")
        self.assertEqual(canonical.decode(first)[3], b"")
        self.assertNotEqual(canonical.artifact_id(canonical.manifest(
            "native-change-v1", first)),
            canonical.artifact_id(canonical.manifest("native-change-v1", second)))
        self.assertNotEqual(canonical.seed_actor_name(canonical.EPOCH),
                            canonical.seed_actor_name(other_epoch))

    def test_presentation_round_trips_and_rejects_loose_forms(self):
        for prefix, raw in (("batch1", canonical.batch_id(b"\x01" * 8)),
                            ("space1", canonical.SPACE),
                            ("epoch1", canonical.EPOCH)):
            text = canonical.presentation(prefix, raw)
            self.assertEqual(canonical.parse_presentation(prefix, text, len(raw)), raw)
            for broken in (text.upper(), text.replace(":", ""), text + "0",
                           text.replace(prefix, "other1")):
                with self.assertRaises(canonical.ProfileError):
                    canonical.parse_presentation(prefix, broken, len(raw))
        self.assertTrue(set(canonical.presentation("batch1", canonical.SPACE))
                        <= set("0123456789abcdefbatch1:"))


class OrderingTests(unittest.TestCase):
    def test_semantic_manifest_is_order_independent(self):
        first = canonical.file_entry(canonical.FILE, [1, 0],
                                     ["b.md", "a.md"], [], [], [1, 0], [])
        second = canonical.file_entry(canonical._fixture_bytes("file-2", 16), [1],
                                      ["README.md"], [], [], [0], [])
        forward = canonical.semantic_manifest(canonical.SPACE, [first, second])
        reverse = canonical.semantic_manifest(canonical.SPACE, [second, first])
        self.assertEqual(forward, reverse)
        decoded = canonical.decode(forward)[2]
        self.assertEqual([entry[1] for entry in decoded], sorted(entry[1] for entry in decoded))
        self.assertEqual(decoded[0][2], [0, 1])
        self.assertEqual(decoded[0][3], ["a.md", "b.md"])

    def test_conflict_identity_binds_the_frozen_participant_order(self):
        first, second = sorted([canonical._fixture_bytes("parent-a", 32),
                                canonical._fixture_bytes("parent-b", 32)])
        forward = canonical.conflict(1, canonical.FILE, [canonical.EPOCH],
                                     [(first, 0), (second, 2)])
        reverse = canonical.conflict(1, canonical.FILE, [canonical.EPOCH],
                                     [(second, 2), (first, 0)])
        self.assertNotEqual(forward, reverse,
                            "the encoder must not sort; participants are ordered")
        canonical.validate_conflict(canonical.decode(forward))
        with self.assertRaises(canonical.ProfileError):
            canonical.validate_conflict(canonical.decode(reverse))


if __name__ == "__main__":
    unittest.main()
