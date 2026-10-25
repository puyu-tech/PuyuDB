# SPDX-License-Identifier: Apache-2.0
"""Optional developer source transfer and native baseline collection, not PuyuDB.

Python 3.9+ standard library only. No push, commit, deployment, package publication,
private architecture document, environment dump, or automatic gate approval.
"""

import argparse
import csv
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import sys
import time
import zipfile

MANIFEST = "I02_SOURCE_MANIFEST.json"
ROOT_FILES = (
    "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "README.md",
    "LICENSE", "NOTICE", "THIRD_PARTY_NOTICES.md", "deny.toml",
    ".gitattributes", ".gitignore", "build.rs",
    "docs/validation/I02-platform-runs.md",
)


def fingerprint(files):
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def source_files(root):
    if any((root / ".cargo" / name).exists() for name in ("config", "config.toml")):
        raise ValueError("project Cargo config requires explicit source review before collection")
    candidates = [root / name for name in ROOT_FILES]
    for directory in ("apps", "crates", "scripts"):
        parent = root / directory
        if parent.is_symlink():
            raise ValueError("source directory is a symlink: " + directory)
        if parent.exists():
            candidates.extend(parent.rglob("*"))
    files = {}
    for path in candidates:
        relative = path.relative_to(root)
        if any(part in ("target", "__pycache__", ".git") for part in relative.parts):
            continue
        fixture = "tests" in relative.parts and (
            "fixtures" in relative.parts or "snapshots" in relative.parts
        )
        included = relative.as_posix() in ROOT_FILES or path.suffix in (
            ".rs", ".py",
        ) or path.name == "Cargo.toml" or (
            fixture and path.suffix in (".txt", ".bin", ".am", ".toml", ".json")
        )
        if not included:
            continue
        if path.is_symlink() or any(
            parent.is_symlink() for parent in path.parents if parent != root
            and root in parent.parents
        ):
            raise ValueError("source symlink is not transferable: " + str(relative))
        if path.is_file():
            files[relative.as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    for required in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "README.md", "LICENSE"):
        if required not in files:
            raise ValueError("required source file is missing: " + required)
    return dict(sorted(files.items()))


def verify_source(root):
    files = source_files(root)
    manifest = root / MANIFEST
    if manifest.is_symlink():
        raise ValueError("source manifest cannot be a symlink")
    if manifest.exists():
        expected = json.loads(manifest.read_text(encoding="utf-8"))
        if expected["files"] != files or expected["fingerprint"] != fingerprint(files):
            raise ValueError("source differs from the transferred manifest")
    return files


def timestamp():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")

def target_directory(root):
    target = root / "target"
    if target.is_symlink() or (target / "i02-baselines").is_symlink():
        raise ValueError("baseline target directories cannot be symlinks")
    return target


def pack_source(root):
    files = verify_source(root)
    output = target_directory(root) / ("i02-source-" + timestamp() + ".zip")
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "x", compression=zipfile.ZIP_DEFLATED) as bundle:
        for name in files:
            data = (root / name).read_bytes()
            if hashlib.sha256(data).hexdigest() != files[name]:
                raise ValueError("source changed during packing: " + name)
            bundle.writestr(name, data)
        bundle.writestr(MANIFEST, json.dumps({
            "tool_report_format": 1, "files": files,
            "fingerprint": fingerprint(files),
        }, indent=2) + "\n")
    return output


def runtime_kind(system, release, wsl_hint=False):
    if system == "Windows":
        return "windows-native"
    if system == "Linux":
        return "wsl" if "microsoft" in release.lower() or wsl_hint else "linux-unclassified"
    return "macos-native" if system == "Darwin" else "unsupported"


def check_profile(profile, observed, root, filesystem):
    allowed = {
        "wsl2": ("wsl", "linux-unclassified"),
        "linux-native": ("linux-unclassified",),
        "windows-native": ("windows-native",),
        "macos-native": ("macos-native",),
    }
    if observed not in allowed[profile]:
        raise ValueError("requested profile does not match observed operating system")
    if profile == "wsl2" and (
        re.match(r"^/mnt/[a-zA-Z]/", root.as_posix())
        or filesystem.lower() in ("drvfs", "9p")
    ):
        raise ValueError("WSL collection requires the Linux filesystem, not a Windows mount")
    if profile == "wsl2" and filesystem == "unknown":
        raise ValueError("cannot verify the WSL source filesystem; install findmnt")


def probe(command, cwd, env):
    try:
        result = subprocess.run(
            command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30, check=False,
        )
        return {
            "exit_code": result.returncode,
            "stdout": result.stdout.decode("utf-8", errors="replace").strip(),
            "stderr": result.stderr.decode("utf-8", errors="replace").strip(),
        }
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"exit_code": None, "error": str(error)}


def unix_resources(text, system):
    if system == "Linux":
        pattern, scale = r"Maximum resident set size \(kbytes\):\s*(\d+)", 1024
    else:
        pattern, scale = r"(\d+)\s+maximum resident set size", 1
    match = re.search(pattern, text)
    return {
        "method": "GNU time -v" if system == "Linux" else "Darwin time -l",
        "maximum_resident_set_bytes": int(match[1]) * scale if match else None,
        "memory_quality": "kernel_lifetime_maximum" if match else "unavailable",
        "scope": "standalone workload process, not Cargo or process tree",
    }


def validate_measurement(name, path):
    with path.open(encoding="utf-8", newline="") as stream:
        rows = list(csv.DictReader(stream))
    if name == "w02":
        values = {row["metric"]: row["value"] for row in rows}
        if len(values) != len(rows):
            raise ValueError("duplicate W02 metrics")
        for key, expected in {
            "baseline_bytes": "1048576", "native_edit_transactions": "100000",
            "retained_native_changes": "100001",
            "final_text_sha256": "d4e1c82b2e791a77690d5c80a22021e8915ccc2a47c58b9d5c1153654d9a6b0f",
        }.items():
            if values.get(key) != expected:
                raise ValueError("unexpected W02 result: " + key)
        for key in (
            "seed_ns", "history_build_ns", "raw_native_snapshot_bytes",
            "compressed_native_snapshot_bytes", "cold_native_decode_ns",
        ) + tuple("historical_cut_" + str(i) + "_read_ns" for i in range(5)):
            if int(values.get(key, "0")) <= 0:
                raise ValueError("missing or invalid W02 metric: " + key)
        return {"transactions": 100000, "historical_cuts": 5, "text_digest_matches": True}
    groups = {}
    for row in rows:
        size = int(row["baseline_bytes"])
        groups.setdefault(size, []).append(row)
        if name == "w01":
            if row["workload"] != "W01" or int(row["native_commit_ns"]) <= 0:
                raise ValueError("invalid W01 sample")
        else:
            if int(row["native_ns"]) <= 0:
                raise ValueError("invalid allocation timing")
            for key in ("allocations", "deallocations", "reallocations",
                        "bytes_allocated", "bytes_deallocated"):
                if int(row[key]) < 0:
                    raise ValueError("invalid allocation count: " + key)
            int(row["bytes_reallocated"])
    if set(groups) != {10240, 102400}:
        raise ValueError("missing W01 dataset")
    for samples in groups.values():
        if len(samples) != 64 or {int(row["sample"]) for row in samples} != set(range(64)):
            raise ValueError("missing, duplicate or incomplete W01 samples")
    return {"samples": 128, "baseline_sizes": [10240, 102400]}


def run_command(root, output, name, command, env, measurement=False, timeout_seconds=1800):
    resource_path = output / (name + ".resources.txt")
    actual = list(command)
    system = platform.system()
    if measurement and system in ("Linux", "Darwin"):
        if not Path("/usr/bin/time").is_file():
            raise ValueError("/usr/bin/time is required for native resource capture")
        actual = (
            ["/usr/bin/time", "-v", "-o", str(resource_path)] + actual
            if system == "Linux" else ["/usr/bin/time", "-l"] + actual
        )
    started = time.perf_counter_ns()
    with (output / (name + ".stdout")).open("wb") as stdout, (
        output / (name + ".stderr")
    ).open("wb") as stderr:
        if measurement and system == "Windows":
            from i02_windows import capture
            exit_code, resources = capture(
                actual, root, env, stdout, stderr, timeout_seconds=timeout_seconds,
            )
        else:
            process = subprocess.Popen(
                actual, cwd=root, env=env, stdin=subprocess.DEVNULL,
                stdout=stdout, stderr=stderr, start_new_session=system != "Windows",
            )
            def stop_owned_group():
                if system == "Windows":
                    if process.poll() is None:
                        process.kill()
                else:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
            try:
                exit_code, resources = process.wait(timeout=timeout_seconds), None
                if exit_code != 0:
                    stop_owned_group()
            except BaseException:
                # A failed/exited time wrapper can still leave its workload in
                # our session. Group cleanup must not depend on a live leader.
                stop_owned_group()
                process.wait()
                raise
    if measurement and system in ("Linux", "Darwin"):
        text = (
            resource_path if system == "Linux" else output / (name + ".stderr")
        ).read_text(encoding="utf-8", errors="replace")
        resources = unix_resources(text, system)
    return {
        "name": name, "command": list(command), "exit_code": exit_code,
        "elapsed_wall_ns": time.perf_counter_ns() - started, "resources": resources,
    }


def metadata(root, env):
    system, release = platform.system(), platform.release()
    filesystem = probe(["findmnt", "-n", "-o", "FSTYPE,OPTIONS", "-T", str(root)], root, env)
    fs_text = filesystem.get("stdout", "")
    cpu_model = "unrecorded"
    if Path("/proc/cpuinfo").is_file():
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                cpu_model = line.split(":", 1)[1].strip()
                break
    result = {
        "system": system, "kernel": release, "machine": platform.machine(),
        "observed_runtime": runtime_kind(system, release, bool(
            env.get("WSL_DISTRO_NAME") or env.get("WSL_INTEROP")
        )),
        "python": platform.python_version(), "logical_cpu_count": os.cpu_count(),
        "cpu_model": cpu_model, "filesystem_probe": filesystem,
        "filesystem": fs_text.split()[0] if fs_text else "unknown",
        "rustc": probe(["rustc", "-vV"], root, env),
        "cargo": probe(["cargo", "-vV"], root, env),
        "operator_context_required": {
            "physical_cpu_cores": "unrecorded", "host_ram_and_guest_limit": "unrecorded",
            "ssd_model": "unrecorded", "encryption": "unrecorded",
            "power_mode": "unrecorded", "background_load": "unrecorded",
        },
    }
    if Path("/etc/os-release").is_file():
        result["distribution"] = Path("/etc/os-release").read_text()
    if Path("/proc/meminfo").is_file():
        result["guest_memory"] = {
            line.split(":", 1)[0]: line.split(":", 1)[1].strip()
            for line in Path("/proc/meminfo").read_text().splitlines()
            if line.startswith(("MemTotal:", "MemAvailable:"))
        }
    if system == "Windows":
        result["windows_hardware"] = probe([
            "powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
            "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding;"
            "@{cpu=(Get-CimInstance Win32_Processor | Select-Object Name,"
            "NumberOfCores,NumberOfLogicalProcessors);"
            "os=(Get-CimInstance Win32_OperatingSystem | Select-Object Caption,"
            "Version,BuildNumber,OSArchitecture,TotalVisibleMemorySize);"
            "disk=(Get-CimInstance Win32_DiskDrive | Select-Object Model,Size,MediaType)}"
            " | ConvertTo-Json -Depth 4 -Compress",
        ], root, env)
    return result


def collect(root, profile, comparison=False):
    files = verify_source(root)
    overrides = [
        key for key in (
            "RUSTC", "CARGO_BUILD_RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
            "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
        ) if os.environ.get(key)
    ]
    if overrides:
        raise ValueError("unset compiler overrides for baseline collection: " + ", ".join(overrides))
    compiler = shutil.which("rustc")
    if compiler is None:
        raise ValueError("rustc must be available through the native rustup installation")
    channel_match = re.search(
        r'channel\s*=\s*"(\d+\.\d+\.\d+)"',
        (root / "rust-toolchain.toml").read_text(encoding="utf-8"),
    )
    if not channel_match:
        raise ValueError("rust-toolchain.toml must pin a released numeric toolchain")
    channel = channel_match[1]
    target = target_directory(root)
    env = dict(os.environ, RUSTUP_TOOLCHAIN=channel, CARGO_TERM_COLOR="never",
               CARGO_TERM_PROGRESS_WHEN="never", GIT_TERMINAL_PROMPT="0",
               CARGO_TARGET_DIR=str(target), LC_ALL="C",
               RUSTC=compiler, CARGO_BUILD_RUSTC=compiler,
               RUSTC_WRAPPER="", RUSTC_WORKSPACE_WRAPPER="")
    facts = metadata(root, env)
    check_profile(profile, facts["observed_runtime"], root, facts["filesystem"])
    rustc = facts["rustc"].get("stdout", "")
    host = re.search(r"^host: (.+)$", rustc, re.MULTILINE)
    if facts["rustc"]["exit_code"] != 0 or not rustc.startswith("rustc " + channel + " ") or not host:
        raise ValueError("the pinned rustup toolchain must be available before collection")
    host = host[1]
    cargo_host = re.search(
        r"^host: (.+)$", facts["cargo"].get("stdout", ""), re.MULTILINE
    )
    if facts["cargo"]["exit_code"] != 0 or not cargo_host or cargo_host[1] != host:
        raise ValueError("cargo and rustc must be native tools for the same host")
    if (
        profile == "windows-native" and not host.endswith("windows-msvc")
        or profile in ("wsl2", "linux-native") and "linux" not in host
        or profile == "macos-native" and "apple-darwin" not in host
        or profile != "macos-native" and not host.startswith("x86_64-")
    ):
        raise ValueError("compiler host must match the native x86-64 profile")
    env["CARGO_BUILD_TARGET"] = host
    output = target / "i02-baselines" / (profile + "-" + timestamp())
    output.mkdir(parents=True)
    report = {
        "tool_report_format": 1, "status": "running", "profile": profile,
        "wsl_version_assertion": "operator-confirmed WSL2" if profile == "wsl2" else None,
        "reference_gate": "not_evaluated", "source_fingerprint": fingerprint(files),
        "source_files": files, "lockfile_sha256": files["Cargo.lock"],
        "environment": facts, "build_target": host, "steps": [],
        "compiler_executable": compiler,
        "build_flag_fingerprints": {
            key: hashlib.sha256(env[key].encode()).hexdigest() if key in env else None
            for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS")
        },
    }
    suffix = ".exe" if profile == "windows-native" else ""
    examples = target / host / "release" / "examples"
    commands = [
        ("fetch", ["cargo", "fetch", "--locked"], False),
        ("fmt", ["cargo", "fmt", "--check"], False),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"], False),
        ("tests", ["cargo", "test", "--workspace", "--frozen"], False),
        ("calibration", ["cargo", "test", "-p", "puyudb-crdt", "--example", "native_allocations", "--frozen"], False),
        ("long-test", ["cargo", "test", "--release", "-p", "puyudb-crdt", "--test",
                      "mixed_histories", "thousands_of_seeded_partition_duplicate_restart_schedules",
                      "--frozen", "--", "--ignored"], False),
        ("build", ["cargo", "build", "--release", "-p", "puyudb-crdt", "--examples", "--frozen"], False),
        ("w01", [str(examples / ("native_workloads" + suffix)), "w01"], True),
        ("w02", [str(examples / ("native_workloads" + suffix)), "w02"], True),
        ("w01-allocations", [str(examples / ("native_allocations" + suffix))], True),
    ]
    if comparison:
        import i02_candidates
        commands = commands[:7] + [
            ("candidate-fixtures", ["cargo", "test", "--release", "-p", "puyudb-crdt",
              "--example", "candidate_workloads", "--frozen", "--", "--ignored"], False),
        ] + i02_candidates.commands(examples, output, suffix)
        report["comparison_scope"] = {
            "version": 2, "engines": list(i02_candidates.ENGINES),
            "native_case_timeout_seconds": 300,
            "note": "native fixture costs; no production adapter, original-update ledger or engine selection",
        }
    def save():
        path = output / "run.json"
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        temporary.replace(path)
    save()
    candidate_failures = []
    try:
        for name, command, measurement in commands:
            print("Running " + name, flush=True)
            is_candidate = name.startswith("candidate-") and name != "candidate-fixtures"
            try:
                if is_candidate:
                    record = run_command(
                        root, output, name, command, env, measurement, timeout_seconds=300,
                    )
                else:
                    record = run_command(root, output, name, command, env, measurement)
            except (subprocess.TimeoutExpired, OSError) as error:
                if not is_candidate:
                    raise
                record = {
                    "name": name, "command": list(command), "exit_code": None,
                    "execution_error": f"{type(error).__name__}: {error}", "resources": None,
                }
                if isinstance(error, subprocess.TimeoutExpired):
                    record["timed_out_seconds"] = error.timeout
            report["steps"].append(record)
            save()
            if is_candidate:
                try:
                    if record["exit_code"] != 0:
                        raise ValueError("candidate command did not finish successfully")
                    record["validated_output"] = i02_candidates.validate(
                        name, output / (name + ".stdout"),
                    )
                except (OSError, ValueError, KeyError, csv.Error) as error:
                    record["validation_error"] = f"{type(error).__name__}: {error}"
                    candidate_failures.append(name)
                save()
                continue
            if record["exit_code"] != 0:
                raise RuntimeError("step failed: " + name + "; inspect " + str(output))
            if name == "fetch":
                env["CARGO_NET_OFFLINE"] = "true"
            if name in ("tests", "calibration", "long-test", "candidate-fixtures"):
                summaries = re.findall(
                    r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored",
                    (output / (name + ".stdout")).read_text(encoding="utf-8"),
                )
                if not summaries or sum(int(row[0]) for row in summaries) == 0:
                    raise ValueError("no passing tests were executed by " + name)
            if name == "build":
                binaries = ("candidate_workloads", "candidate_allocations") if comparison else (
                    "native_workloads", "native_allocations",
                )
                report["executables_sha256"] = {
                    path.name: hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in (examples / (name + suffix) for name in binaries)
                }
            if measurement:
                record["validated_output"] = validate_measurement(
                    name, output / (name + ".stdout")
                )
        if source_files(root) != files:
            raise ValueError("source changed during collection")
        if comparison:
            report["seed_repeatability"] = i02_candidates.seed_results(report["steps"])
            report["candidate_failures"] = candidate_failures
            if candidate_failures:
                raise RuntimeError(
                    "candidate cases failed; all remaining cases were still recorded; inspect " + str(output)
                )
        report["status"] = "comparison_recorded" if comparison else "commands_passed"
        return output
    except BaseException as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        save()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("pack", "collect"))
    parser.add_argument("--profile", choices=(
        "windows-native", "wsl2", "linux-native", "macos-native",
    ))
    parser.add_argument("--comparison", action="store_true",
                        help="collect all four candidate-native workload profiles")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    if args.operation == "collect" and not args.profile:
        parser.error("collect requires --profile")
    if args.operation == "pack" and args.comparison:
        parser.error("--comparison applies only to collect")
    result = pack_source(root) if args.operation == "pack" else collect(
        root, args.profile, args.comparison,
    )
    print(result)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        print("baseline collection failed: " + str(error), file=sys.stderr)
        sys.exit(1)
