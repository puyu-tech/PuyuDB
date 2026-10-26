# SPDX-License-Identifier: Apache-2.0
"""Rebuild a disposable, patched DT interface spike without changing the main graph.

Run from any directory: python3 experiments/diamond-types-interfaces/run.py
--mode baseline|anchors|strict|combined. Requires the provisioned pinned toolchain
and dependency cache. Upstream source is extracted from its exact Git object into
ignored target/, never edited in Cargo's cache or permanently vendored.
"""

import argparse
import datetime
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile

REVISION = "89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922"
FEATURE_ARCHIVES = {
    "lz4_flex-0.14.0": "ecbdfe44b1bd960b68170b417450a628c43f7cf56bb3c5317e61cb230ee7f226",
    "twox-hash-2.1.2": "9ea3136b675547379c4bd395ca6b938e5ad3c3d20fad76e7fe85f9e0d011419c",
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extract_archive(data, destination):
    """Accept only regular files/directories contained in the generated tree."""
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for member in archive:
            name = PurePosixPath(member.name)
            if (name.is_absolute() or ".." in name.parts or "\\" in member.name
                    or ":" in member.name or not (member.isfile() or member.isdir())):
                raise ValueError("unsupported Git archive entry: " + member.name)
            path = destination.joinpath(*name.parts)
            if member.isdir():
                path.mkdir(parents=True, exist_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as source:
                    path.write_bytes(source.read())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("baseline", "anchors", "strict", "combined"), default="combined")
    parser.add_argument("--native-defaults-archives", type=Path,
                        help="also check native defaults using the two checksum-pinned .crate files here")
    parser.add_argument("--native-feature-profile", choices=("defaults", "lz4"), default="defaults",
                        help="optional check: upstream defaults, or LZ4 without the Unix-only storage module")
    args = parser.parse_args()
    if args.native_defaults_archives and args.mode not in ("strict", "combined"):
        raise ValueError("native-default compatibility check requires the strict patch")
    if args.native_feature_profile != "defaults" and not args.native_defaults_archives:
        raise ValueError("feature profile requires the checksum-pinned archives")
    inputs = Path(__file__).resolve().parent
    root = inputs.parents[1]
    for key in (
        "RUSTC", "CARGO_BUILD_RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET",
    ):
        if os.environ.get(key):
            raise ValueError("unset compiler/target override: " + key)
    channel = re.search(r'channel\s*=\s*"(\d+\.\d+\.\d+)"', (root / "rust-toolchain.toml").read_text())[1]
    env = dict(os.environ, RUSTUP_TOOLCHAIN=channel, CARGO_TERM_COLOR="never", GIT_TERMINAL_PROMPT="0")
    cargo = shutil.which("cargo", path=env.get("PATH"))
    git = shutil.which("git", path=env.get("PATH"))
    rustc = shutil.which("rustc", path=env.get("PATH"))
    if not cargo or not git or not rustc:
        raise ValueError("native cargo/rustup and Git must be on PATH")
    metadata = json.loads(subprocess.check_output(
        [cargo, "metadata", "--frozen", "--format-version", "1"], cwd=root, env=env,
    ))
    package = next(p for p in metadata["packages"] if p["name"] == "diamond-types")
    if package["version"] != "2.0.0" or not package["source"].endswith("#" + REVISION):
        raise ValueError("main workspace no longer uses the reviewed DT pin")
    cache = Path(package["manifest_path"]).parent
    archive = subprocess.check_output([git, "-C", str(cache), "archive", "--format=tar", REVISION])
    for path in (root / "target", root / "target/dt-interfaces", root / "target/dt-interface-build"):
        if path.is_symlink():
            raise ValueError("spike output directories cannot be symlinks")
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    output = root / "target" / "dt-interfaces" / (stamp + "-" + args.mode)
    output.mkdir(parents=True)
    upstream, harness = output / "upstream", output / "harness"
    extract_archive(archive, upstream)
    harness.mkdir()
    (harness / "tests").mkdir()
    (harness / "examples").mkdir()
    (harness / "Cargo.toml").write_text(
        '[package]\nname = "puyudb-dt-interface-spike"\nversion = "0.0.0"\n'
        'edition = "2024"\npublish = false\n[workspace]\n'
        '[dependencies]\ndiamond-types = { version = "=2.0.0", path = "../upstream", default-features = false }\n'
    )
    shutil.copyfile(root / "Cargo.lock", harness / "Cargo.lock")
    shutil.copyfile(inputs / "examples" / "compat.rs", harness / "examples" / "compat.rs")
    selected = {"baseline": [], "anchors": ["anchors"], "strict": ["strict"], "combined": ["anchors", "strict"]}[args.mode]
    for name in selected:
        shutil.copyfile(inputs / "tests" / (name + ".rs"), harness / "tests" / (name + ".rs"))
    original = {name: digest(root / name) for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")}
    env["CARGO_TARGET_DIR"] = str(root / "target" / "dt-interface-build")
    steps = []
    registry_packages = []
    native_features = []
    feature_archives = {}
    optional_native_features = []
    succeeded = False
    spike_inputs = {
        p.relative_to(root).as_posix(): digest(p) for p in sorted(inputs.rglob("*"))
        if p.is_file() and p.suffix in (".rs", ".py", ".patch")
    }

    def run(name, command):
        result = subprocess.run(command, cwd=root, env=env, stdin=subprocess.DEVNULL, capture_output=True)
        (output / (name + ".stdout")).write_bytes(result.stdout)
        (output / (name + ".stderr")).write_bytes(result.stderr)
        steps.append({"name": name, "command": command, "exit_code": result.returncode})
        print(name, result.returncode, flush=True)
        if result.returncode:
            print(result.stdout.decode(errors="replace"))
            print(result.stderr.decode(errors="replace"))
            raise RuntimeError("spike check failed: " + name)
        return result.stdout

    manifest_path = str(harness / "Cargo.toml")
    try:
        compiler = run("rustc", [rustc, "-Vv"]).decode()
        if not compiler.startswith("rustc " + channel + " "):
            raise ValueError("the pinned compiler was not selected")
        run("cargo", [cargo, "-Vv"])
        run("runner-contracts", [sys.executable, "-m", "unittest", "discover",
                                "-s", str(inputs / "tests"), "-p", "test_*.py"])
        # Reuse existing exact registry selections, then reject any new registry
        # version/source. Only the harness and the two native path packages differ.
        generated = json.loads(run("resolve", [cargo, "metadata", "--offline", "--manifest-path", manifest_path, "--format-version", "1"]))
        known = {(p["name"], p["version"], p["source"]) for p in metadata["packages"] if p["source"]}
        for package in generated["packages"]:
            if package["source"] and (package["name"], package["version"], package["source"]) not in known:
                raise ValueError("spike introduced an unreviewed dependency: " + package["name"])
            if package["source"]:
                registry_packages.append((package["name"], package["version"], package["source"]))
            elif (package["name"], package["version"]) not in {
                ("diamond-types", "2.0.0"), ("rle", "0.2.0"), ("puyudb-dt-interface-spike", "0.0.0"),
            }:
                raise ValueError("unexpected path package: " + package["name"])
        native_id = next(p["id"] for p in generated["packages"] if p["name"] == "diamond-types")
        native_features = next(n["features"] for n in generated["resolve"]["nodes"] if n["id"] == native_id)
        if {"lz4", "storage"}.intersection(native_features):
            raise ValueError("spike unexpectedly enabled native compression/storage")
        common = [cargo, "run", "--offline", "--locked", "--manifest-path", manifest_path, "--example", "compat"]
        before = run("compat-original", common)
        for name in selected:
            patch = inputs / (name + ".patch")
            directory = upstream.relative_to(root).as_posix()
            command = [git, "-c", "core.autocrlf=false", "apply", "--directory=" + directory]
            run(name + "-patch-stat", [git, "apply", "--numstat", str(patch)])
            run(name + "-patch-check", command + ["--check", str(patch)])
            run(name + "-patch", command + [str(patch)])
        after = run("compat-patched", common)
        if before != after:
            raise ValueError("native text/encoding compatibility observations changed")
        run("tests", [cargo, "test", "--offline", "--locked", "--manifest-path", manifest_path])
        run("tests-release", [cargo, "test", "--release", "--offline", "--locked", "--manifest-path", manifest_path])
        run("clippy", [cargo, "clippy", "--offline", "--locked", "--manifest-path", manifest_path, "--all-targets", "--", "-D", "warnings"])
        run("fmt", [cargo, "fmt", "--manifest-path", manifest_path, "--check"])
        if args.native_defaults_archives:
            # Separate feature-compatibility graph, not the default-off experiment
            # or main graph. Published archives are extracted locally; no cache
            # checkout or registry source is patched, fetched or substituted by a
            # different version. Tests execute the actual 0.14.0 implementation.
            sources = output / "feature-dependencies"
            for name, checksum in FEATURE_ARCHIVES.items():
                file = args.native_defaults_archives.resolve() / (name + ".crate")
                data = file.read_bytes()
                if hashlib.sha256(data).hexdigest() != checksum:
                    raise ValueError("optional-feature archive checksum mismatch: " + name)
                feature_archives[name] = checksum
                extract_archive(data, sources)
            feature = output / "feature-harness"
            (feature / "tests").mkdir(parents=True)
            shutil.copyfile(inputs / "tests/lz4.rs", feature / "tests/lz4.rs")
            shutil.copyfile(harness / "Cargo.lock", feature / "Cargo.lock")
            native_options = ("default-features = true" if args.native_feature_profile == "defaults"
                              else 'default-features = false, features = ["lz4"]')
            (feature / "Cargo.toml").write_text(
                '[package]\nname = "puyudb-dt-defaults-check"\nversion = "0.0.0"\n'
                'edition = "2024"\npublish = false\n[workspace]\n'
                '[dependencies]\ndiamond-types = { path = "../upstream", ' + native_options + ' }\n'
                '[patch.crates-io]\nlz4_flex = { path = "../feature-dependencies/lz4_flex-0.14.0" }\n'
                'twox-hash = { path = "../feature-dependencies/twox-hash-2.1.2" }\n'
            )
            feature_manifest = str(feature / "Cargo.toml")
            graph = json.loads(run("defaults-resolve", [
                cargo, "metadata", "--offline", "--manifest-path", feature_manifest, "--format-version", "1",
            ]))
            for package in graph["packages"]:
                if package["source"] and (package["name"], package["version"], package["source"]) not in known:
                    raise ValueError("optional feature introduced an unreviewed registry package")
            native_id = next(p["id"] for p in graph["packages"] if p["name"] == "diamond-types")
            optional_native_features = next(n["features"] for n in graph["resolve"]["nodes"] if n["id"] == native_id)
            expected = {"default", "lz4", "storage"} if args.native_feature_profile == "defaults" else {"lz4"}
            if set(optional_native_features) != expected:
                raise ValueError("requested native feature configuration was not selected")
            for profile in ([], ["--release"]):
                name = "defaults-release" if profile else "defaults-debug"
                run(name, [cargo, "test", *profile, "--offline", "--locked", "--manifest-path", feature_manifest])
        succeeded = True
    finally:
        if original != {name: digest(root / name) for name in original}:
            raise ValueError("main dependency/toolchain inputs changed")
        if any(digest(root / name) != value for name, value in spike_inputs.items()):
            raise ValueError("spike inputs changed during execution")
        inventory = {p.relative_to(output).as_posix(): {"bytes": p.stat().st_size, "sha256": digest(p)}
                     for p in [*output.iterdir(), harness / "Cargo.toml", harness / "Cargo.lock",
                               output / "feature-harness/Cargo.toml", output / "feature-harness/Cargo.lock"] if p.is_file()}
        (output / "manifest.json").write_text(json.dumps({
            "mode": args.mode, "succeeded": succeeded, "upstream_revision": REVISION,
            "upstream_archive_sha256": hashlib.sha256(archive).hexdigest(),
            "main_inputs": original,
            "spike_inputs": spike_inputs, "native_features": native_features,
            "registry_packages": sorted(registry_packages),
            "optional_feature_archives": feature_archives,
            "optional_feature_profile": args.native_feature_profile if feature_archives else None,
            "optional_native_features": optional_native_features,
            "steps": steps, "files": inventory,
        }, indent=2) + "\n")
        print("EVIDENCE", output, flush=True)


if __name__ == "__main__":
    main()
