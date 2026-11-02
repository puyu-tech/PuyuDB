# SPDX-License-Identifier: Apache-2.0
"""Contract checks for the I05 P5 metadata-artifact golden fixtures."""

import json
from pathlib import Path
import unittest

import i05_semantic_manifest as manifest

FIXTURES = Path(__file__).resolve().parent.parent / "tests" / "fixtures" / "metadata-v1"


def load(name):
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


def bytes_of(value):
    return bytes.fromhex(value)


class CanonicalSubsetTests(unittest.TestCase):
    def test_integer_encodings_are_minimal_and_round_trip(self):
        expected = {
            0: "00", 23: "17", 24: "1818", 255: "18ff", 256: "190100",
            65535: "19ffff", 65536: "1a00010000",
            4294967296: "1b0000000100000000",
        }
        for value, encoded in expected.items():
            self.assertEqual(manifest.uint(value).hex(), encoded)
            self.assertEqual(manifest.decode(manifest.uint(value)), value)

    def test_non_canonical_forms_are_rejected(self):
        for encoded in ("1800", "190000", "9f01ff", "a10102", "c101",
                        "fb3ff0000000000000", "62ff80", "0100", "8200"):
            with self.subTest(encoded=encoded):
                with self.assertRaises(manifest.ProfileError):
                    manifest.decode(bytes_of(encoded))

    def test_unknown_record_name_is_a_programming_error(self):
        with self.assertRaises(KeyError):
            manifest.validate_record("unknown", b"\x01")


class ScenarioFixtureTests(unittest.TestCase):
    def test_committed_fixtures_match_the_generator(self):
        for name, payload in (("scenarios.json", manifest.build_scenarios()),
                              ("negative.json", manifest.build_negatives())):
            committed = (FIXTURES / name).read_text(encoding="utf-8")
            self.assertEqual(committed,
                             json.dumps(payload, indent=2, sort_keys=True) + "\n",
                             f"{name} drifted from the generator")

    def test_scenarios_cover_the_five_planned_cuts(self):
        names = [scenario["name"] for scenario in load("scenarios.json")["scenarios"]]
        self.assertEqual(names, ["create-rename", "concurrent-path-alternatives",
                                 "delete-concurrent-edit", "mode-conflict",
                                 "resolution"])

    def test_operations_reencode_to_their_expected_hex(self):
        checked = 0
        for scenario in load("scenarios.json")["scenarios"]:
            ops = [manifest.encode_operation(op) for op in scenario["operations"]]
            with self.subTest(scenario=scenario["name"]):
                self.assertEqual([op.hex() for op in ops],
                                 scenario["operations_hex"])
                artifact = manifest.metadata_artifact(
                    ops, [manifest.unhex(a) for a in scenario["context_assertions"]])
                self.assertEqual(artifact.hex(), scenario["artifact_hex"])
                checked += len(ops)
        self.assertGreaterEqual(checked, 12)

    def test_every_artifact_and_epoch_validates_and_hashes(self):
        for scenario in load("scenarios.json")["scenarios"]:
            with self.subTest(scenario=scenario["name"]):
                manifest.validate_record("metadata_artifact",
                                         bytes_of(scenario["artifact_hex"]))
                for epoch in scenario["epochs"]:
                    body = manifest.encode_epoch_descriptor(epoch["descriptor"])
                    self.assertEqual(body.hex(), epoch["body_hex"])
                    self.assertEqual(manifest.epoch_id(body).hex(),
                                     epoch["epoch_id"])
                    manifest.validate_record("epoch_descriptor", body)

    def test_conflict_ids_rederive_and_validate(self):
        checked = 0
        for scenario in load("scenarios.json")["scenarios"]:
            for conflict_rec in scenario["conflicts"]:
                with self.subTest(scenario=scenario["name"]):
                    body = manifest.encode_conflict(conflict_rec)
                    self.assertEqual(body.hex(), conflict_rec["body_hex"])
                    self.assertEqual(manifest.conflict_id(body).hex(),
                                     conflict_rec["conflict_id"])
                    manifest.validate_record("conflict", body)
                    checked += 1
        self.assertGreaterEqual(checked, 3)

    def test_semantic_manifest_ids_rederive_and_validate(self):
        for scenario in load("scenarios.json")["scenarios"]:
            manifest_rec = scenario["manifest"]
            with self.subTest(scenario=scenario["name"]):
                body = manifest.semantic_manifest(manifest_rec)
                self.assertEqual(body.hex(), manifest_rec["body_hex"])
                self.assertEqual(manifest.semantic_manifest_id(body).hex(),
                                 manifest_rec["semantic_manifest_id"])
                manifest.validate_record("semantic_manifest", body)

    def test_cli_verification_passes_on_the_committed_fixtures(self):
        self.assertEqual(manifest.main([]), 0)
        self.assertEqual(manifest.verify_fixtures(), [])


class DerivationSemanticsTests(unittest.TestCase):
    """GATE B audit M4: the derivation semantics the corpus blesses must agree
    with the Rust rules in crates/puyudb-core/src/projection.rs."""

    def _scenario(self, name):
        return {s["name"]: s for s in load("scenarios.json")["scenarios"]}[name]

    def _entry(self, scenario, file_hex):
        return {e["file"]: e for e in scenario["manifest"]["entries"]}[file_hex]

    def test_resolution_batch_carries_the_fresh_tag3_epoch_write(self):
        scenario = self._scenario("resolution")
        resolutions = [op for op in scenario["operations"] if op["tag"] == 8]
        self.assertEqual(len(resolutions), 1)
        op = resolutions[0]
        self.assertEqual(op["resolution"], 2)
        fresh = [o for o in scenario["operations"] if o["tag"] == 3 and
                 manifest.epoch_id(manifest.encode_epoch_descriptor(o["epoch"])).hex()
                 == op["payload"]]
        self.assertEqual(len(fresh), 1,
                         "spec §9 / validate_resolutions: tag 2 requires a new "
                         "selection of that epoch in the same batch")

    def test_resolved_conflict_disappears_from_the_recomputed_cut(self):
        scenario = self._scenario("resolution")
        self.assertEqual(scenario["conflicts"], [])
        entry = self._entry(scenario, manifest.FILE_A.hex())
        self.assertEqual(entry["conflicts"], [],
                         "tag8_resolution_publishes_fresh_assignment_and_"
                         "supersedes_alternatives: the resolved ConflictId is "
                         "absent from the recomputed cut")

    def test_delete_edit_conflict_names_the_edited_epoch_only(self):
        scenario = self._scenario("delete-concurrent-edit")
        kind_one = [c for c in scenario["conflicts"] if c["kind"] == 1]
        self.assertEqual(len(kind_one), 1)
        entry = self._entry(scenario, manifest.FILE_A.hex())
        self.assertEqual(kind_one[0]["epochs"], [manifest.EPOCH_OPAQUE_A.hex()])
        self.assertNotEqual(kind_one[0]["epochs"], entry["epochs"],
                            "conflicts_impl: delete_edit epochs are the edited "
                            "epoch, not every maximal epoch selection of the file")


class NegativeFixtureTests(unittest.TestCase):
    def test_every_case_matches_its_declared_expectation(self):
        failures = manifest.verify_negatives(load("negative.json"))
        self.assertEqual(failures, [])

    def test_rejection_cases_raise_their_declared_code(self):
        for case in load("negative.json")["cases"]:
            if not case["code"]:
                continue
            with self.subTest(case=case["name"]):
                with self.assertRaises(manifest.ProfileError) as raised:
                    manifest.validate_record(case["record"],
                                             bytes_of(case["body_hex"]))
                self.assertEqual(raised.exception.code, case["code"],
                                 f"{case['name']}: {raised.exception}")

    def test_unsorted_participants_are_rejected_and_change_the_conflict_id(self):
        case = self._case("conflict/unsorted-participants")
        wrong = manifest.conflict_id(bytes_of(case["body_hex"]))
        canonical = manifest.conflict_id(bytes_of(case["canonical_body_hex"]))
        self.assertNotEqual(wrong, canonical)
        self.assertEqual(wrong.hex(), case["other_id_hex"])
        self.assertEqual(canonical.hex(), case["canonical_id_hex"])

    def test_unsorted_entries_are_rejected_and_change_the_manifest_id(self):
        for name in ("manifest/unsorted-entries", "manifest/entry-unsorted-paths"):
            case = self._case(name)
            with self.subTest(case=name):
                wrong = manifest.semantic_manifest_id(bytes_of(case["body_hex"]))
                canonical = manifest.semantic_manifest_id(
                    bytes_of(case["canonical_body_hex"]))
                self.assertNotEqual(wrong, canonical)
                self.assertEqual(wrong.hex(), case["other_id_hex"])

    def test_wrong_domain_separator_changes_the_hash(self):
        for name in ("domain/conflict-wrong-separator",
                     "domain/manifest-wrong-separator", "domain/cross-domain"):
            case = self._case(name)
            with self.subTest(case=name):
                body = bytes_of(case["body_hex"])
                wrong = manifest.digest_with_separator(
                    case["separator"].encode("ascii"), body)
                self.assertEqual(wrong.hex(), case["other_id_hex"])
                self.assertNotEqual(wrong.hex(), case["canonical_id_hex"])

    def _case(self, name):
        return {case["name"]: case
                for case in load("negative.json")["cases"]}[name]


class OrderingTests(unittest.TestCase):
    def test_manifest_encoding_is_entry_order_independent(self):
        entries = [manifest._entry(manifest.FILE_A, [1], ["a.md"], [], [], [0], []),
                   manifest._entry(manifest.FILE_B, [1], ["b.md"], [], [], [0], [])]
        forward = manifest.semantic_manifest({"space": manifest.SPACE.hex(),
                                              "entries": entries})
        reverse = manifest.semantic_manifest({"space": manifest.SPACE.hex(),
                                              "entries": list(reversed(entries))})
        self.assertEqual(forward, reverse)
        decoded = manifest.decode(forward)[2]
        self.assertEqual([entry[1] for entry in decoded],
                         sorted(entry[1] for entry in decoded))

    def test_file_entry_encoding_sorts_and_dedups_inner_arrays(self):
        shuffled = {"file": manifest.FILE_A.hex(), "live": [1, 0, 1],
                    "paths": ["b.md", "a.md", "b.md"], "epochs": [],
                    "contents": [], "modes": [0, 0], "conflicts": []}
        canonical = manifest.file_entry({"file": manifest.FILE_A.hex(),
                                         "live": [0, 1], "paths": ["a.md", "b.md"],
                                         "epochs": [], "contents": [],
                                         "modes": [0], "conflicts": []})
        self.assertEqual(manifest.file_entry(shuffled), canonical)
        self.assertEqual(manifest.decode(canonical)[3], ["a.md", "b.md"])

    def test_conflict_encoder_does_not_sort_participants(self):
        def conflict_rec(participants):
            return {"kind": 1, "file": manifest.FILE_A.hex(),
                    "epochs": [manifest.EPOCH_OPAQUE_A.hex()],
                    "participants": participants}
        first = {"batch": manifest.BATCH_A.hex(), "index": 0}
        second = {"batch": manifest.BATCH_B.hex(), "index": 3}
        ordered = sorted([first, second], key=lambda p: (bytes_of(p["batch"]),
                                                         p["index"]))
        forward = manifest.encode_conflict(conflict_rec(ordered))
        reverse = manifest.encode_conflict(conflict_rec(list(reversed(ordered))))
        self.assertNotEqual(forward, reverse,
                            "the encoder must not sort; participants are ordered")
        manifest.validate_conflict(manifest.decode(forward))
        with self.assertRaises(manifest.ProfileError):
            manifest.validate_conflict(manifest.decode(reverse))


if __name__ == "__main__":
    unittest.main()
