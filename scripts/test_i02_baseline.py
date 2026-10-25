# SPDX-License-Identifier: Apache-2.0
"""Contract checks for the optional source-transfer/baseline collector."""

import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import zipfile

import i02_baseline as baseline
import i02_windows


class BaselineTests(unittest.TestCase):
    def make_source(self, root):
        for name, content in {
            "Cargo.toml": "[workspace]\n",
            "Cargo.lock": "# lock\n",
            "rust-toolchain.toml": '[toolchain]\nchannel = "1.99.1"\n',
            "README.md": "# PuyuDB\n",
            "LICENSE": "Apache-2.0\n",
            "crates/example/src/lib.rs": "// source\n",
            "crates/example/tests/fixtures/seed.txt": "fixture\n",
            "scripts/i02_baseline.py": "# collector\n",
            ".env": "SECRET=do-not-transfer\n",
            ".cargo/credentials.toml": 'token = "do-not-transfer"\n',
            "target/private.txt": "do-not-transfer\n",
            "crates/example/credentials.env": "do-not-transfer\n",
        }.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")

    def test_source_bundle_contains_current_uncommitted_inputs_not_git_history(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_source(root)
            archive = baseline.pack_source(root)
            with zipfile.ZipFile(archive) as bundle:
                names = set(bundle.namelist())
                self.assertIn("crates/example/src/lib.rs", names)
                self.assertIn("crates/example/tests/fixtures/seed.txt", names)
                self.assertNotIn(".env", names)
                self.assertNotIn(".cargo/credentials.toml", names)
                self.assertNotIn("target/private.txt", names)
                self.assertNotIn("crates/example/credentials.env", names)
                manifest = json.loads(bundle.read(baseline.MANIFEST))
                self.assertEqual(
                    manifest["fingerprint"], baseline.fingerprint(manifest["files"])
                )

    def test_bundle_verification_rejects_changed_source_before_commands_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_source(root)
            files = baseline.source_files(root)
            (root / baseline.MANIFEST).write_text(
                json.dumps({"files": files, "fingerprint": baseline.fingerprint(files)}),
                encoding="utf-8",
            )
            baseline.verify_source(root)
            (root / "crates/example/src/lib.rs").write_text("// changed\n")
            with self.assertRaisesRegex(ValueError, "source"):
                baseline.verify_source(root)

    def test_source_symlinks_are_not_followed_into_a_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_source(root)
            source = root / "crates/example/src/lib.rs"
            source.unlink()
            try:
                source.symlink_to(root / ".env")
            except OSError as error:
                self.skipTest("source symlinks unavailable on this host: " + str(error))
            with self.assertRaisesRegex(ValueError, "symlink"):
                baseline.pack_source(root)

    def test_both_cargo_configuration_names_require_review(self):
        for name in ("config", "config.toml"):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.make_source(root)
                config = root / ".cargo" / name
                config.write_text('[build]\nrustflags = ["-C", "opt-level=0"]\n')
                with self.assertRaisesRegex(ValueError, "Cargo config"):
                    baseline.source_files(root)

    def test_compiler_override_is_rejected_before_any_collection_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_source(root)
            facts = {
                "observed_runtime": "macos-native", "filesystem": "unknown",
                "rustc": {"exit_code": 0, "stdout": "rustc 1.99.1 (fixture)\nhost: aarch64-apple-darwin"},
                "cargo": {"exit_code": 0, "stdout": "cargo 1.99.1\nhost: aarch64-apple-darwin"},
            }
            with mock.patch.dict(os.environ, {"RUSTC": "/different/compiler"}), mock.patch.object(
                baseline, "metadata", return_value=facts
            ), mock.patch.object(
                baseline, "run_command", return_value={"exit_code": 7, "name": "fetch"}
            ) as run:
                with self.assertRaisesRegex(ValueError, "compiler override"):
                    baseline.collect(root, "macos-native")
            run.assert_not_called()

    def test_profiles_do_not_relabel_wsl_as_native_linux(self):
        self.assertEqual(baseline.runtime_kind("Linux", "6.6.0-microsoft-standard-WSL2"), "wsl")
        self.assertEqual(baseline.runtime_kind("Linux", "6.6.0"), "linux-unclassified")
        self.assertEqual(baseline.runtime_kind("Linux", "custom-kernel", True), "wsl")
        baseline.check_profile("wsl2", "wsl", Path("/home/user/puyudb"), "ext4")
        with self.assertRaises(ValueError):
            baseline.check_profile("linux-native", "wsl", Path("/home/user/puyudb"), "ext4")
        for path, filesystem in [
            ("/mnt/c/puyudb", "9p"),
            ("/home/user/shared", "drvfs"),
        ]:
            with self.assertRaises(ValueError):
                baseline.check_profile("wsl2", "wsl", Path(path), filesystem)

    def test_failed_command_retains_outputs_and_cannot_be_success(self):
        import sys

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            record = baseline.run_command(
                root, root, "failure",
                [sys.executable, "-c", "import sys; print('captured'); sys.exit(7)"],
                dict(os.environ),
            )
            self.assertEqual(record["exit_code"], 7)
            self.assertIn("captured", (root / "failure.stdout").read_text())
            self.assertNotEqual(record["exit_code"], 0)

    def test_fingerprint_changes_with_content_not_local_checkout_path(self):
        self.assertEqual(
            baseline.fingerprint({"a.rs": "abc", "b.rs": "def"}),
            baseline.fingerprint({"b.rs": "def", "a.rs": "abc"}),
        )
        self.assertNotEqual(
            baseline.fingerprint({"a.rs": "abc"}),
            baseline.fingerprint({"a.rs": "changed"}),
        )

    def test_owned_command_timeout_is_bounded_and_not_reported_as_success(self):
        import subprocess
        import sys

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(subprocess.TimeoutExpired):
                baseline.run_command(
                    root, root, "timeout",
                    [sys.executable, "-c", "import time; time.sleep(30)"],
                    dict(os.environ), timeout_seconds=0.05,
                )

    @unittest.skipIf(os.name == "nt", "Unix process-group supervision")
    def test_failed_wrapper_group_is_cleaned_even_when_its_leader_has_exited(self):
        import signal

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            process = mock.Mock(pid=12345)
            process.wait.return_value = 7
            process.poll.return_value = 7
            with mock.patch.object(baseline.subprocess, "Popen", return_value=process), mock.patch.object(
                baseline.os, "killpg"
            ) as kill:
                result = baseline.run_command(root, root, "wrapper", ["fixture"], dict(os.environ))
            self.assertEqual(result["exit_code"], 7)
            kill.assert_called_once_with(12345, signal.SIGKILL)

    def test_failed_collection_persists_a_failed_report_and_stops(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_source(root)
            facts = {
                "observed_runtime": "macos-native", "filesystem": "unknown",
                "rustc": {"exit_code": 0, "stdout": "rustc 1.99.1 (fixture)\nhost: aarch64-apple-darwin"},
                "cargo": {"exit_code": 0, "stdout": "cargo 1.99.1\nhost: aarch64-apple-darwin"},
            }
            with mock.patch.object(baseline, "metadata", return_value=facts), mock.patch.object(
                baseline, "run_command", return_value={"exit_code": 7, "name": "fetch"}
            ) as run:
                with self.assertRaises(RuntimeError):
                    baseline.collect(root, "macos-native")
            self.assertEqual(run.call_count, 1)
            reports = list((root / "target/i02-baselines").glob("*/run.json"))
            report = json.loads(reports[0].read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["reference_gate"], "not_evaluated")
            self.assertEqual(report["steps"][0]["exit_code"], 7)

    def test_comparison_records_other_candidates_after_one_case_fails(self):
        self.check_comparison_failure("exit")

    def test_comparison_records_other_candidates_after_launch_or_csv_exceptions(self):
        for kind in ("launch", "csv"):
            with self.subTest(kind=kind):
                self.check_comparison_failure(kind)

    def check_comparison_failure(self, kind):
        import csv

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.make_source(root)
            facts = {
                "observed_runtime": "macos-native", "filesystem": "unknown",
                "rustc": {"exit_code": 0, "stdout": "rustc 1.99.1 (fixture)\nhost: aarch64-apple-darwin"},
                "cargo": {"exit_code": 0, "stdout": "cargo 1.99.1\nhost: aarch64-apple-darwin"},
            }
            def run(_root, output, name, command, _env, *args, **kwargs):
                if name == "candidate-automerge-seed-0" and kind == "launch":
                    raise PermissionError("test launch refusal")
                if name == "build":
                    binaries = root / "target/aarch64-apple-darwin/release/examples"
                    binaries.mkdir(parents=True)
                    for binary in ("candidate_workloads", "candidate_allocations"):
                        (binaries / binary).write_bytes(b"test executable")
                (output / (name + ".stdout")).write_text(
                    "test result: ok. 1 passed; 0 failed; 0 ignored\n"
                )
                return {"name": name, "command": command,
                        "exit_code": 7 if name == "candidate-automerge-seed-0" and kind == "exit" else 0}
            def validate(name, path):
                if name == "candidate-automerge-seed-0" and kind == "csv":
                    raise csv.Error("test field larger than parser limit")
                return {"seed_signature": "same"}
            with mock.patch.object(baseline, "metadata", return_value=facts), mock.patch(
                "i02_candidates.validate", side_effect=validate
            ), mock.patch.object(baseline, "run_command", side_effect=run), mock.patch("builtins.print"):
                with self.assertRaisesRegex(RuntimeError, "candidate cases failed"):
                    baseline.collect(root, "macos-native", comparison=True)
            path = next((root / "target/i02-baselines").glob("*/run.json"))
            report = json.loads(path.read_text())
            self.assertEqual(report["status"], "failed")
            self.assertEqual(report["candidate_failures"], ["candidate-automerge-seed-0"])
            self.assertEqual(report["steps"][-1]["name"], "candidate-diamond-types-w02")
            self.assertEqual(len(report["steps"]), 40)

    def test_resource_parsers_preserve_units_and_unknowns(self):
        linux = baseline.unix_resources("Maximum resident set size (kbytes): 123", "Linux")
        darwin = baseline.unix_resources("123 maximum resident set size", "Darwin")
        self.assertEqual(linux["maximum_resident_set_bytes"], 123 * 1024)
        self.assertEqual(darwin["maximum_resident_set_bytes"], 123)
        missing = baseline.unix_resources("", "Linux")
        self.assertIsNone(missing["maximum_resident_set_bytes"])
        self.assertEqual(missing["memory_quality"], "unavailable")

    def test_windows_report_never_upgrades_live_samples_to_an_exact_peak(self):
        resources = i02_windows.WindowsResources.__new__(i02_windows.WindowsResources)
        resources.handle = None
        resources.errors = []
        resources.peak = None
        resources.samples = 0
        self.assertIsNone(resources.finish()["peak_working_set_bytes"])
        resources.peak, resources.samples = 1234, 1
        report = resources.finish()
        self.assertEqual(report["memory_quality"], "lower_bound")
        self.assertIsNone(report["cpu_user_ns"])

    def test_windows_probe_initialization_failure_does_not_fake_zero_resources(self):
        import sys

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with (root / "out").open("wb") as stdout, (root / "err").open("wb") as stderr:
                with mock.patch.object(i02_windows, "WindowsResources", side_effect=OSError("unavailable")):
                    code, resources = i02_windows.capture(
                        [sys.executable, "-c", "print('completed')"],
                        root, dict(os.environ), stdout, stderr,
                    )
            self.assertEqual(code, 0)
            self.assertIsNone(resources["peak_working_set_bytes"])
            self.assertEqual(resources["memory_quality"], "unavailable")

    def test_measurement_success_requires_complete_native_output(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "w01.csv"
            rows = ["workload,baseline_bytes,sample,native_commit_ns"]
            rows += [
                "W01,{},{},1".format(size, index)
                for size in (10240, 102400) for index in range(64)
            ]
            path.write_text("\n".join(rows) + "\n")
            self.assertEqual(baseline.validate_measurement("w01", path)["samples"], 128)
            path.write_text("\n".join(rows[:-1]) + "\n")
            with self.assertRaises(ValueError):
                baseline.validate_measurement("w01", path)
            path.write_text("metric,value\nnative_edit_transactions,1\n")
            with self.assertRaises(ValueError):
                baseline.validate_measurement("w02", path)


if __name__ == "__main__":
    unittest.main()
