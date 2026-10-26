# SPDX-License-Identifier: Apache-2.0
"""Contracts for retained benchmark evidence, independent of native engines."""

import csv
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("dt_loro_final", Path(__file__).with_name("run.py"))
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


def csv_data():
    stream = io.StringIO()
    writer = csv.DictWriter(stream, fieldnames=runner.BENCH_FIELDS)
    writer.writeheader()
    for sample in range(64):
        row = {key: "1" for key in runner.BENCH_FIELDS}
        row.update(
            schema_version="1", engine="dt", case="ascii10k", prefill="0",
            trusted_generated_prefill="0", sample=str(sample), writer=str(sample % 4),
            total_ns="100", text_sha256="a" * 64,
        )
        writer.writerow(row)
    return stream.getvalue()


class EvidenceTests(unittest.TestCase):
    def test_full_process_keeps_samples_and_uses_nearest_rank(self):
        rows = runner.validate_benchmark(csv_data(), "dt", "ascii10k", 0)
        self.assertEqual(len(rows), 64)
        self.assertEqual(runner.nearest_rank(list(range(1, 65)), 95), 61)
        self.assertEqual(runner.nearest_rank(list(range(1, 65)), 50), 32)

    def test_missing_duplicate_and_wrong_process_samples_are_rejected(self):
        for data in (
            csv_data().rsplit("\n", 2)[0] + "\n",
            csv_data().replace(",63,3,", ",62,3,"),
            csv_data().replace("ascii10k", "unicode100k"),
        ):
            with self.subTest(data=data[-100:]), self.assertRaises(ValueError):
                runner.validate_benchmark(data, "dt", "ascii10k", 0)

    def test_negative_timing_and_duplicate_headers_are_rejected(self):
        data = csv_data()
        lines = data.splitlines()
        fields = lines[1].split(",")
        fields[runner.BENCH_FIELDS.index("inspect_ns")] = "-1"
        lines[1] = ",".join(fields)
        with self.assertRaises(ValueError):
            runner.validate_benchmark("\n".join(lines), "dt", "ascii10k", 0)
        lines = data.splitlines()
        lines[0] = lines[0].replace("author_ns", "inspect_ns")
        with self.assertRaises(ValueError):
            runner.validate_benchmark("\n".join(lines), "dt", "ascii10k", 0)

    def test_screen_observation_must_belong_to_the_requested_trial(self):
        case = {"case_index": "7", "family": "truncate", "expectation": "must_reject",
                "risk": "low", "checksum": "unchanged", "encoded_bytes": "10"}
        row = [
            "dt-binding", "strict_binding", "7", "truncate", "must_reject", "low",
            "unchanged", "10", "1", "1", "0", "0", "0", "0", "0", "100",
        ]
        observation = runner.validate_screen(",".join(row), "dt-binding", case)
        self.assertEqual(observation["errors"], "1")
        row[2] = "8"
        with self.assertRaises(ValueError):
            runner.validate_screen(",".join(row), "dt-binding", case)

    def test_unobservable_state_is_not_reported_as_observed_mutation(self):
        case = {"case_index": "7", "family": "truncate", "expectation": "observe",
                "risk": "low", "checksum": "unchanged", "encoded_bytes": "10"}
        row = [
            "dt-native", "native_decode_and_add", "7", "truncate", "observe", "low",
            "unchanged", "10", "1", "0", "1", "0", "unknown", "0", "1", "100",
        ]
        observation = runner.validate_screen(",".join(row), "dt-native", case)
        self.assertEqual(observation["base_mutated"], "unknown")

    @unittest.skipIf(runner.os.name == "nt", "Unix child resource API")
    def test_unavailable_memory_limit_is_recorded_without_preventing_exec(self):
        with patch("resource.setrlimit", side_effect=[None, None, ValueError("unsupported")]), (
                patch.object(runner.os, "write")):
            limits = runner.child_limits()
        self.assertTrue(limits["cpu"]["applied"])
        self.assertFalse(limits["memory"]["applied"])
        self.assertIn("unsupported", limits["memory"]["error"])

    def test_retained_output_cannot_be_silently_resealed(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            evidence = output / "bench.stdout"
            evidence.write_text("original timing evidence\n")
            manifest = {}
            runner.save_manifest(output, manifest)
            evidence.write_text("altered timing evidence\n")
            with self.assertRaises(ValueError):
                runner.save_manifest(output, manifest)


if __name__ == "__main__":
    unittest.main()
