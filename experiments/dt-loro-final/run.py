# SPDX-License-Identifier: Apache-2.0
"""Isolated, fixed-protocol DT–Loro comparison; never selects an engine."""

import argparse
import csv
import hashlib
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[2]
INPUTS = Path(__file__).resolve().parent
PRIMARY_MATRIX = [(case, count) for case in ("ascii10k", "unicode100k")
                  for count in (0, 1000, 10000)] + [("churn1m", 1000)]
SUPPLEMENTAL_MATRIX = [("churn1m_bounded", 1000)]
MATRIX = PRIMARY_MATRIX + SUPPLEMENTAL_MATRIX
ENGINES = ("dt", "loro")
SCREEN_ENGINES = ("dt-binding", "dt-native", "loro-binding", "loro-native")
BENCH_FIELDS = (
    "schema_version", "engine", "case", "prefill", "trusted_generated_prefill",
    "sample", "writer", "author_ns", "inspect_ns", "publish_ns",
    "anchor_create_ns", "anchor_resolve_ns", "historical_read_ns", "save_ns",
    "load_ns", "digest_retain_ns", "total_ns", "original_update_bytes",
    "retained_original_bytes", "retained_digest_bytes", "trusted_prefill_original_bytes",
    "snapshot_bytes", "text_scalars", "text_utf8_bytes", "text_sha256",
)
SCREEN_FIELDS = (
    "engine", "layer", "case_index", "family", "expectation", "risk", "checksum",
    "encoded_bytes", "count", "errors", "accepts", "panics", "base_mutated",
    "snapshot_panics", "violations", "elapsed_us",
)


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


spike = load_module("dt_interfaces", ROOT / "experiments/diamond-types-interfaces/run.py")
sys.path.insert(0, str(ROOT / "scripts"))
baseline = load_module("i02_baseline", ROOT / "scripts/i02_baseline.py")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_inputs():
    paths = [ROOT / p for p in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")]
    paths += list(INPUTS.glob("*.rs")) + list(INPUTS.glob("*.py")) + [INPUTS / "README.md"]
    paths += [ROOT / "crates/puyudb-crdt/tests/support" / (p + ".rs")
              for p in ("loro_fit", "diamond_fit")]
    paths += [ROOT / "experiments/diamond-types-interfaces" / (p + ".patch")
              for p in ("anchors", "strict")]
    paths += [ROOT / "experiments/diamond-types-interfaces/tests" / (p + ".rs")
              for p in ("anchors", "strict")]
    paths += [ROOT / p for p in ("experiments/diamond-types-interfaces/run.py",
                                "scripts/i02_baseline.py", "scripts/i02_windows.py")]
    return {p.relative_to(ROOT).as_posix(): digest(p) for p in sorted(paths)}


def validate_benchmark(data, engine, case, prefill):
    reader = csv.DictReader(io.StringIO(data))
    if reader.fieldnames is None or tuple(reader.fieldnames) != BENCH_FIELDS:
        raise ValueError("wrong/duplicate benchmark CSV headers")
    rows = list(reader)
    if len(rows) != 64 or {r.get("sample") for r in rows} != {str(i) for i in range(64)}:
        raise ValueError("benchmark must retain exactly 64 distinct samples")
    for index, row in enumerate(rows):
        if None in row or any(v is None or v == "" for v in row.values()):
            raise ValueError("incomplete benchmark row")
        if (row["schema_version"], row["engine"], row["case"],
                row["prefill"], row["trusted_generated_prefill"], row["sample"]) != (
                "1", engine, case, str(prefill), str(prefill), str(index)):
            raise ValueError("benchmark output belongs to another process/workload")
        if int(row["writer"]) != index % 4:
            raise ValueError("wrong fixture writer schedule")
        for key in BENCH_FIELDS:
            if key not in ("engine", "case", "text_sha256") and int(row[key]) < 0:
                raise ValueError("negative benchmark observation: " + key)
        if not re.fullmatch(r"[0-9a-f]{64}", row["text_sha256"]):
            raise ValueError("invalid text digest")
        if any(int(row[k]) > int(row["total_ns"])
               for k in BENCH_FIELDS if k.endswith("_ns") and k != "total_ns"):
            raise ValueError("component exceeds enclosing total")
    return rows


def nearest_rank(values, percentile):
    if not values or not 0 < percentile <= 100:
        raise ValueError("invalid percentile input")
    return sorted(values)[math.ceil(len(values) * percentile / 100) - 1]


def validate_screen(data, engine, case):
    lines = list(csv.reader(io.StringIO(data)))
    if len(lines) != 1 or len(lines[0]) != len(SCREEN_FIELDS):
        raise ValueError("missing/invalid screening observation")
    row = dict(zip(SCREEN_FIELDS, lines[0]))
    if row["engine"] != engine or row["count"] != "1":
        raise ValueError("screening observation belongs to another engine")
    for field in ("case_index", "family", "expectation", "risk", "checksum", "encoded_bytes"):
        if row[field] != case[field]:
            raise ValueError("screening observation belongs to another trial: " + field)
    for field in SCREEN_FIELDS[7:]:
        if field == "base_mutated":
            if row[field] not in ("0", "1", "unknown"):
                raise ValueError("invalid state observation")
            continue
        if int(row[field]) < 0:
            raise ValueError("negative screening observation")
    if sum(int(row[field]) for field in ("errors", "accepts", "panics")) != 1:
        raise ValueError("screening attempt has inconsistent outcome counts")
    return row


def environment():
    for key in ("RUSTC", "CARGO_BUILD_RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
                "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET"):
        if os.environ.get(key):
            raise ValueError("unset compiler/target override: " + key)
    channel = re.search(r'channel\s*=\s*"(\d+\.\d+\.\d+)"',
                        (ROOT / "rust-toolchain.toml").read_text())[1]
    env = dict(os.environ, RUSTUP_TOOLCHAIN=channel, CARGO_TERM_COLOR="never",
               CARGO_TARGET_DIR=str(ROOT / "target/dt-loro-final-build"),
               GIT_TERMINAL_PROMPT="0")
    rustc = subprocess.check_output(["rustc", "-Vv"], cwd=ROOT, env=env).decode()
    cargo = subprocess.check_output(["cargo", "-Vv"], cwd=ROOT, env=env).decode()
    host = re.search(r"^host: (.+)$", rustc, re.MULTILINE)
    cargo_host = re.search(r"^host: (.+)$", cargo, re.MULTILINE)
    if (not rustc.startswith("rustc " + channel + " ") or not host or not cargo_host
            or host[1] != cargo_host[1]):
        raise ValueError("native pinned rustc/Cargo host mismatch")
    # Override config-file build.target and use its matching artifact directory.
    env["CARGO_BUILD_TARGET"] = host[1]
    return env


def verify_files(output, manifest):
    for name, record in manifest.get("files", {}).items():
        file = output / name
        if (not file.is_file() or file.stat().st_size != record["bytes"]
                or digest(file) != record["sha256"]):
            raise ValueError("retained evidence changed or missing: " + name)


def save_manifest(output, manifest):
    verify_files(output, manifest)
    current = {
        p.relative_to(output).as_posix(): {"bytes": p.stat().st_size, "sha256": digest(p)}
        for p in sorted(output.rglob("*"))
        if p.is_file() and "upstream" not in p.relative_to(output).parts
        and p.name != "manifest.json"
    }
    manifest.setdefault("files", {}).update(
        (name, record) for name, record in current.items() if name not in manifest["files"])
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def verify_generated(output, manifest, env, binaries=False):
    for name, expected in manifest["generated_inputs"].items():
        if digest(output / name) != expected:
            raise ValueError("generated build input changed: " + name)
    if binaries:
        for name in ("compare", "screen"):
            if digest(binary(env, name)) != manifest["binary_sha256"][name]:
                raise ValueError("executable changed since the checked build: " + name)


def command(output, manifest, name, args, env, measurement=False, timeout=300):
    if (output / (name + ".stdout")).exists():
        raise ValueError("refusing to overwrite retained command: " + name)
    try:
        step = baseline.run_command(ROOT, output, name, list(map(str, args)), env,
                                    measurement=measurement, timeout_seconds=timeout)
    except subprocess.TimeoutExpired:
        step = {"name": name, "command": list(map(str, args)),
                "exit_code": None, "timeout_seconds": timeout, "resources": None}
    manifest["steps"].append(step)
    save_manifest(output, manifest)
    print(name, step["exit_code"], flush=True)
    return step


def require(step):
    if step["exit_code"] != 0:
        raise RuntimeError("command failed; retained evidence: " + step["name"])


def prepare(env):
    original = source_inputs()
    for path in (ROOT / "target", ROOT / "target/dt-loro-final",
                 ROOT / "target/dt-loro-final-build"):
        if path.is_symlink():
            raise ValueError("output directories cannot be symlinks")
    output = ROOT / "target/dt-loro-final" / baseline.timestamp()
    output.mkdir(parents=True)
    manifest = {
        "format": 1, "status": "prepared", "source_inputs": original,
        "source_fingerprint": baseline.fingerprint(original),
        "environment": baseline.metadata(ROOT, env), "matrix": MATRIX, "trials": 3,
        "primary_matrix": PRIMARY_MATRIX, "supplemental_matrix": SUPPLEMENTAL_MATRIX,
        "samples_per_process": 64, "steps": [],
    }
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--frozen", "--format-version", "1"], cwd=ROOT, env=env))
    package = next(p for p in metadata["packages"] if p["name"] == "diamond-types")
    if not package["source"].endswith("#" + spike.REVISION):
        raise ValueError("DT pin differs from reviewed source")
    cache = Path(package["manifest_path"]).parent
    archive = subprocess.check_output(
        ["git", "-C", str(cache), "archive", "--format=tar", spike.REVISION])
    manifest["upstream_revision"] = spike.REVISION
    manifest["upstream_archive_sha256"] = hashlib.sha256(archive).hexdigest()
    spike.extract_archive(archive, output / "upstream")
    for name in ("anchors", "strict"):
        patch = ROOT / "experiments/diamond-types-interfaces" / (name + ".patch")
        args = ["git", "-c", "core.autocrlf=false", "apply",
                "--directory=" + (output / "upstream").relative_to(ROOT).as_posix()]
        require(command(output, manifest, name + "-check", args + ["--check", str(patch)], env))
        require(command(output, manifest, name + "-apply", args + [str(patch)], env))
    harness = output / "harness"
    harness.mkdir()
    for source in INPUTS.glob("*.rs"):
        shutil.copyfile(source, harness / source.name)
    for name in ("loro_fit", "diamond_fit"):
        shutil.copyfile(ROOT / "crates/puyudb-crdt/tests/support" / (name + ".rs"),
                        harness / (name + ".rs"))
    (harness / "tests").mkdir()
    for name in ("anchors", "strict"):
        shutil.copyfile(ROOT / "experiments/diamond-types-interfaces/tests" / (name + ".rs"),
                        harness / "tests" / (name + ".rs"))
    (harness / "Cargo.toml").write_text(
        '[package]\nname="puyudb-dt-loro-final"\nversion="0.0.0"\nedition="2024"\n'
        'publish=false\n[workspace]\n[dependencies]\n'
        'diamond-types={path="../upstream",version="=2.0.0",default-features=false}\n'
        'loro={version="=1.17.1",default-features=false}\n'
        'sha2={version="=0.11.1",default-features=false}\nthiserror="=2.0.21"\n'
        '[[bin]]\nname="compare"\npath="compare.rs"\n'
        '[[bin]]\nname="screen"\npath="screen.rs"\n')
    shutil.copyfile(ROOT / "Cargo.lock", harness / "Cargo.lock")
    require(command(output, manifest, "resolve", [
        "cargo", "metadata", "--offline", "--manifest-path", harness / "Cargo.toml",
        "--format-version", "1"], env))
    graph = json.loads((output / "resolve.stdout").read_text())
    known = {(p["name"], p["version"], p["source"]) for p in metadata["packages"] if p["source"]}
    for package in graph["packages"]:
        if package["source"] and (package["name"], package["version"], package["source"]) not in known:
            raise ValueError("unreviewed registry dependency: " + package["name"])
        if not package["source"] and package["name"] not in (
                "puyudb-dt-loro-final", "diamond-types", "rle"):
            raise ValueError("unreviewed path dependency: " + package["name"])
    manifest["registry_packages"] = sorted(
        (p["name"], p["version"], p["source"]) for p in graph["packages"] if p["source"])
    native = next(p["id"] for p in graph["packages"] if p["name"] == "diamond-types")
    manifest["native_features"] = next(n["features"] for n in graph["resolve"]["nodes"]
                                       if n["id"] == native)
    if manifest["native_features"]:
        raise ValueError("native DT features unexpectedly enabled")
    manifest["generated_inputs"] = {
        p.relative_to(output).as_posix(): digest(p)
        for directory in (output / "upstream", harness)
        for p in sorted(directory.rglob("*")) if p.is_file()
    }
    if original != source_inputs():
        raise ValueError("source inputs changed during preparation")
    save_manifest(output, manifest)
    return output, manifest


def check(output, manifest, env):
    verify_generated(output, manifest, env)
    path = output / "harness/Cargo.toml"
    for name, args in (
        ("fmt", ["cargo", "fmt", "--manifest-path", path, "--check"]),
        ("clippy", ["cargo", "clippy", "--offline", "--locked", "--manifest-path", path,
                    "--all-targets", "--", "-D", "warnings"]),
        ("tests-debug", ["cargo", "test", "--offline", "--locked", "--manifest-path", path,
                         "--", "--skip", "compare::tests::"]),
        ("tests-release", ["cargo", "test", "--release", "--offline", "--locked",
                          "--manifest-path", path, "--", "--skip", "compare::tests::"]),
        ("build-release", ["cargo", "build", "--release", "--offline", "--locked",
                          "--manifest-path", path, "--bins"]),
    ):
        require(command(output, manifest, name, args, env, timeout=600))
    require(command(output, manifest, "correctness",
                    [binary(env, "compare"), "correctness"], env, timeout=180))
    manifest["binary_sha256"] = {name: digest(binary(env, name)) for name in ("compare", "screen")}
    manifest["checks_complete"] = True


def binary(env, name):
    return (Path(env["CARGO_TARGET_DIR"]) / env["CARGO_BUILD_TARGET"] / "release"
            / (name + (".exe" if os.name == "nt" else "")))


def measure(output, manifest, env):
    if not manifest.get("checks_complete"):
        raise ValueError("correctness/lint checks must pass before measurement")
    verify_generated(output, manifest, env, binaries=True)
    failures = []
    for trial in range(3):
        for case, prefill in MATRIX:
            order = ENGINES if (trial + MATRIX.index((case, prefill))) % 2 == 0 else ENGINES[::-1]
            for engine in order:
                name = f"bench-{case}-{prefill}-{engine}-{trial}"
                step = command(output, manifest, name, [
                    binary(env, "compare"), "bench", engine, case, prefill],
                    env, measurement=True, timeout=180)
                if step["exit_code"] != 0:
                    failures.append(name)
                    continue
                validate_benchmark((output / (name + ".stdout")).read_text(),
                                   engine, case, prefill)
    manifest["measurement_failures"] = failures
    manifest["measurements_complete"] = True


def child_limits():
    """Child-only best-effort limits; this is not an allocator safety proof."""
    import resource
    kind = resource.RLIMIT_AS if sys.platform.startswith("linux") else resource.RLIMIT_DATA
    size = (1024 if sys.platform.startswith("linux") else 512) * 1024 * 1024
    result = {}
    for name, limit, value in (
            ("core", resource.RLIMIT_CORE, 0), ("cpu", resource.RLIMIT_CPU, 3),
            ("memory", kind, size)):
        try:
            resource.setrlimit(limit, (value, value))
            result[name] = {"applied": True, "value": value}
        except (ValueError, OSError) as error:
            result[name] = {"applied": False, "value": value, "error": str(error)}
            os.write(2, ("LIMIT_UNAVAILABLE " + name + ": " + str(error) + "\n").encode())
    return result


def screen(output, manifest, env):
    if not manifest.get("checks_complete"):
        raise ValueError("build/check the native binaries before screening")
    verify_generated(output, manifest, env, binaries=True)
    manifest["screen_limits"] = {
        "wall_seconds": 3, "cpu_seconds": 3 if os.name != "nt" else None,
        "requested_memory": ("Linux RLIMIT_AS 1 GiB" if sys.platform.startswith("linux")
                             else "macOS RLIMIT_DATA 512 MiB" if sys.platform == "darwin"
                             else "no hard memory limit"),
        "qualification": "child-only best effort; no general resource-safety guarantee",
    }
    if os.name != "nt":
        require(command(output, manifest, "screen-limit-probe", [
            sys.executable, "-c",
            "import json,runpy; m=runpy.run_path(" + repr(str(Path(__file__).resolve()))
            + "); print(json.dumps(m['child_limits']()))",
        ], env))
        manifest["screen_limits"]["observed_probe"] = json.loads(
            (output / "screen-limit-probe.stdout").read_text())
    findings = []
    total = 0
    for engine in SCREEN_ENGINES:
        name = "screen-list-" + engine
        require(command(output, manifest, name, [binary(env, "screen"), engine, "list"], env))
        cases = list(csv.DictReader((output / (name + ".stdout")).open()))
        if not cases or len(cases) > 600:
            raise ValueError("screening corpus exceeds the predeclared bound")
        for case in cases:
            index = case["case_index"]
            name = f"screen-{engine}-{index}"
            if (output / (name + ".stdout")).exists():
                raise ValueError("refusing to overwrite screening evidence")
            args = list(map(str, [binary(env, "screen"), engine, index]))
            started = time.perf_counter_ns()
            with (output / (name + ".stdout")).open("wb") as stdout, (
                    output / (name + ".stderr")).open("wb") as stderr:
                process = subprocess.Popen(
                    args, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                    stdout=stdout, stderr=stderr,
                    preexec_fn=child_limits if os.name != "nt" else None)
                try:
                    code = process.wait(timeout=3)
                    timed_out = False
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                    code, timed_out = process.returncode, True
            result = {"name": name, "command": args, "exit_code": code,
                      "timeout": timed_out, "elapsed_wall_ns": time.perf_counter_ns() - started,
                      "case": case}
            try:
                result["observation"] = validate_screen(
                    (output / (name + ".stdout")).read_text(), engine, case)
            except ValueError as error:
                result["observation_error"] = str(error)
            if code != 0 or "observation" not in result:
                findings.append(name)
            manifest["steps"].append(result)
            total += 1
            if total % 100 == 0:
                print("screened", total, "findings", len(findings), flush=True)
    manifest["screen_findings"] = findings
    manifest["screen_trials"] = total
    manifest["screen_complete"] = True


def summarize(output, manifest):
    verify_files(output, manifest)
    summaries = []
    signatures = {}
    for trial in range(3):
        for case, prefill in MATRIX:
            for engine in ENGINES:
                name = f"bench-{case}-{prefill}-{engine}-{trial}"
                step = next((s for s in manifest["steps"] if s["name"] == name), None)
                if not step or step["exit_code"] != 0:
                    continue
                rows = validate_benchmark((output / (name + ".stdout")).read_text(),
                                          engine, case, prefill)
                sig = [(r["text_sha256"], r["text_scalars"], r["text_utf8_bytes"]) for r in rows]
                key = (case, prefill)
                if key in signatures and signatures[key] != sig:
                    raise ValueError("oracle/text observations differ across engines/trials")
                signatures[key] = sig
                entry = {"engine": engine, "case": case, "prefill": prefill, "trial": trial}
                for field in BENCH_FIELDS:
                    if field.endswith("_ns"):
                        values = [int(r[field]) for r in rows]
                        entry[field + "_p50"] = nearest_rank(values, 50)
                        entry[field + "_p95"] = nearest_rank(values, 95)
                for field in ("retained_original_bytes", "trusted_prefill_original_bytes",
                              "retained_digest_bytes", "snapshot_bytes"):
                    entry[field] = int(rows[-1][field])
                entry["resources"] = step.get("resources")
                summaries.append(entry)
    expected = len(MATRIX) * len(ENGINES) * 3
    manifest["summary"] = {
        "completed_processes": len(summaries), "expected_processes": expected,
        "method": "nearest-rank within each 64-sample process; no pooled percentiles",
        "processes": summaries,
        "medians": [
            {"engine": engine, "case": case, "prefill": prefill,
             "process_count": len(group),
             **{field: statistics.median(r[field] for r in group)
                for field in summaries[0] if field.endswith(("_p50", "_p95"))}}
            for case, prefill in MATRIX for engine in ENGINES
            for group in [[r for r in summaries if (r["engine"], r["case"], r["prefill"])
                           == (engine, case, prefill)]]
            if group
        ] if summaries else [],
    }
    (output / "summary.json").write_text(json.dumps(manifest["summary"], indent=2) + "\n")
    print("SUMMARY", len(summaries), "/", expected, "measurement processes", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("prepare", "check", "measure", "screen", "summarize", "all"))
    parser.add_argument("--run", type=Path)
    args = parser.parse_args()
    env = environment()
    if args.phase in ("prepare", "all"):
        if args.run:
            raise ValueError("prepare/all always creates fresh evidence")
        output, manifest = prepare(env)
    else:
        if not args.run:
            raise ValueError("this phase requires --run")
        output = args.run.resolve()
        if not output.is_relative_to(ROOT / "target/dt-loro-final"):
            raise ValueError("run must be inside this workspace's generated evidence directory")
        manifest = json.loads((output / "manifest.json").read_text())
        verify_files(output, manifest)
        if manifest["source_inputs"] != source_inputs():
            raise ValueError("current source inputs differ from this run")
    try:
        if args.phase in ("check", "all"):
            check(output, manifest, env)
        if args.phase in ("screen", "all"):
            screen(output, manifest, env)
        if args.phase in ("measure", "all"):
            measure(output, manifest, env)
        if args.phase in ("summarize", "all"):
            summarize(output, manifest)
        if manifest["source_inputs"] != source_inputs():
            raise ValueError("inputs changed during execution")
        manifest["last_completed_phase"] = args.phase
    finally:
        save_manifest(output, manifest)
        print("EVIDENCE", output, flush=True)


if __name__ == "__main__":
    main()
