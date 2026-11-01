# SPDX-License-Identifier: Apache-2.0
"""Contract checks for comparable, candidate-native evidence."""

import hashlib
from pathlib import Path
import tempfile
import unittest

import i02_candidates as candidates


class CandidateEvidenceTests(unittest.TestCase):
    def test_w01_requires_complete_samples_matching_the_requested_engine(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate-yrs-w01-0.stdout"
            rows = ["engine,baseline_bytes,sample,coordinate_ns,native_ns,call_ns,materialize_ns"]
            rows += [
                f"yrs,{size},{sample},5,10,20,30"
                for size in (10240, 102400) for sample in range(64)
            ]
            path.write_text("\n".join(rows) + "\n")
            self.assertEqual(candidates.validate(path.stem, path)["samples"], 128)
            path.write_text("\n".join(rows[:-1]) + "\n")
            with self.assertRaises(ValueError):
                candidates.validate(path.stem, path)
            path.write_text("\n".join(rows).replace("yrs,", "loro,") + "\n")
            with self.assertRaises(ValueError):
                candidates.validate(path.stem, path)

    def test_w02_requires_deleted_interval_cuts_and_actual_exported_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate-yrs-w02.stdout"
            artifact = path.with_suffix(".native")
            artifact.write_bytes(b"native-test-fixture")
            values = {
                "engine": "yrs", "baseline_bytes": "1048576",
                "logical_edit_batches": "100000", "seed_and_writer_setup_ns": "1",
                "seed_materialize_ns": "1",
                "history_build_ns": "100", "native_edit_ns_sum": "20",
                "native_call_ns_sum": "30", "coordinate_ns_sum": "10",
                "native_save_ns": "1", "native_load_ns": "1",
                "latest_text_materialization_ns": "1",
                "native_artifact_bytes": str(artifact.stat().st_size),
                "native_artifact_sha256": hashlib.sha256(artifact.read_bytes()).hexdigest(),
                "verified_cuts": "11",
                "final_text_sha256": "d4e1c82b2e791a77690d5c80a22021e8915ccc2a47c58b9d5c1153654d9a6b0f",
            }
            expected = candidates.expected_cuts()
            for i, (completed, digest) in enumerate(expected.items()):
                values.update({
                    f"cut_{i}_after_batches": str(completed),
                    f"cut_{i}_reload_ns": "1", f"cut_{i}_read_ns": "1",
                    f"cut_{i}_latest_text_ns": "1",
                    f"cut_{i}_text_sha256": digest,
                })
            def save():
                path.write_text("metric,value\n" + "".join(f"{k},{v}\n" for k,v in values.items()))
            save()
            self.assertEqual(candidates.validate(path.stem, path)["verified_cuts"], 11)
            values["cut_2_text_sha256"] = "0" * 64
            save()
            with self.assertRaises(ValueError):
                candidates.validate(path.stem, path)
            values["cut_2_text_sha256"] = list(expected.values())[2]
            save()
            artifact.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                candidates.validate(path.stem, path)

    def test_duplicate_or_unexpected_headers_cannot_hide_wrong_engine_or_timings(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "candidate-yrs-w01-0.stdout"
            header = "engine,engine,baseline_bytes,sample,coordinate_ns,native_ns,call_ns,materialize_ns"
            rows = [
                f"loro,yrs,{size},{sample},5,10,20,30"
                for size in (10240, 102400) for sample in range(64)
            ]
            path.write_text(header + "\n" + "\n".join(rows) + "\n")
            with self.assertRaises(ValueError):
                candidates.validate(path.stem, path)
            path.write_text(
                header.replace("engine,engine", "ignored,engine")
                + "\n" + "\n".join(rows) + "\n"
            )
            with self.assertRaises(ValueError):
                candidates.validate(path.stem, path)

    def test_nonrepeatable_seed_bytes_are_reported_not_hidden_as_engine_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            steps = []
            for trial, signature in enumerate(("one", "two", "one")):
                steps.append({
                    "name": f"candidate-yrs-seed-{trial}", "exit_code": 0,
                    "validated_output": {"seed_signature": signature},
                })
            report = candidates.seed_results(steps)
            self.assertEqual(report["yrs"]["status"], "observed_nonrepeatable")
            self.assertEqual(report["loro"]["status"], "not_tested")


if __name__ == "__main__":
    unittest.main()
