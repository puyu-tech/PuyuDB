# SPDX-License-Identifier: Apache-2.0
"""Fixed comparative native workload contract, not an engine-selection score."""

import csv
from functools import lru_cache
import hashlib
import re

ENGINES = ("automerge", "yrs", "loro", "diamond-types")
CUTS = (0, 1000, 1001, 1002, 2000, 2001, 2002, 25000, 50000, 75000, 100000)


@lru_cache(maxsize=1)
def expected_cuts():
    """Independent byte-array oracle for the fixed ASCII workload."""
    text = bytearray(b"a" * 1048576)
    expected = {0: hashlib.sha256(text).hexdigest()}
    restore = b""
    for index in range(100000):
        phase = index % 10000
        if phase == 2000:
            restore = bytes(text[:65536])
        if phase == 1000:
            text[:0] = b"p" * 65536
        elif phase in (1001, 2000):
            del text[:65536]
        elif phase == 2001:
            text[:0] = restore
        else:
            text[index % 1024] = ord("x") if index % 2 == 0 else ord("y")
        if index + 1 in CUTS:
            expected[index + 1] = hashlib.sha256(text).hexdigest()
    assert expected[100000] == "d4e1c82b2e791a77690d5c80a22021e8915ccc2a47c58b9d5c1153654d9a6b0f"
    return expected


def commands(examples, output, suffix):
    result = []
    executable = str(examples / ("candidate_workloads" + suffix))
    # Rotate native process order; never pool samples from different processes.
    for trial in range(3):
        order = ENGINES[trial:] + ENGINES[:trial]
        for engine in order:
            name = f"candidate-{engine}-seed-{trial}"
            result.append((name, [
                executable, engine, "seed", str(output / (name + ".native")),
            ], False))
    for trial in range(3):
        for engine in ENGINES[trial:] + ENGINES[:trial]:
            result.append((f"candidate-{engine}-w01-{trial}", [
                executable, engine, "w01",
            ], True))
    for engine in ENGINES:
        result.append((f"candidate-{engine}-allocations", [
            str(examples / ("candidate_allocations" + suffix)), engine, "w01",
        ], True))
        name = f"candidate-{engine}-w02"
        result.append((name, [
            executable, engine, "w02", str(output / (name + ".native")),
        ], True))
    return result


def _artifact(path, size, digest):
    if not path.is_file():
        raise ValueError("native artifact missing: " + str(path))
    data = path.read_bytes()
    if len(data) != int(size) or hashlib.sha256(data).hexdigest() != digest:
        raise ValueError("native artifact length/hash mismatch: " + str(path))


def validate(name, path):
    match = re.fullmatch(
        r"candidate-(automerge|yrs|loro|diamond-types)-(seed|w01|allocations|w02)(?:-[0-2])?",
        name,
    )
    if not match:
        raise ValueError("unknown candidate measurement")
    engine, workload = match.groups()
    with path.open(encoding="utf-8", newline="") as stream:
        reader = csv.DictReader(stream)
        expected_fields = {
            "w02": {"metric", "value"},
            "seed": {"engine", "case", "artifact_bytes", "artifact_sha256", "text_sha256"},
            "w01": {"engine", "baseline_bytes", "sample", "coordinate_ns", "native_ns",
                    "call_ns", "materialize_ns"},
        }
        expected_fields["allocations"] = expected_fields["w01"] | {
            "allocations", "deallocations", "reallocations", "bytes_allocated",
            "bytes_deallocated", "bytes_reallocated",
        }
        fields = reader.fieldnames
        if not fields or len(fields) != len(set(fields)) or set(fields) != expected_fields[workload]:
            raise ValueError("missing, duplicate or unexpected candidate CSV headers")
        rows = list(reader)
        if not rows or any(
            None in row or any(v is None or v == "" for v in row.values()) for row in rows
        ):
            raise ValueError("missing candidate output cells")
    if workload == "w02":
        values = {row["metric"]: row["value"] for row in rows}
        if len(values) != len(rows) or values.get("engine") != engine:
            raise ValueError("duplicate W02 metric or wrong engine")
        if (values.get("baseline_bytes"), values.get("logical_edit_batches"),
                values.get("verified_cuts")) != ("1048576", "100000", "11"):
            raise ValueError("wrong W02 workload scope")
        expected = expected_cuts()
        if values.get("final_text_sha256") != expected[100000]:
            raise ValueError("wrong W02 final text")
        for index, (completed, digest) in enumerate(expected.items()):
            if values.get(f"cut_{index}_after_batches") != str(completed):
                raise ValueError("missing/reordered saved cut")
            if values.get(f"cut_{index}_text_sha256") != digest:
                raise ValueError("wrong saved-cut text")
            for field in ("reload_ns", "latest_text_ns", "read_ns"):
                if int(values.get(f"cut_{index}_{field}", "-1")) < 0:
                    raise ValueError("missing saved-cut timing")
        for key in (
            "seed_and_writer_setup_ns", "seed_materialize_ns", "history_build_ns", "native_edit_ns_sum",
            "native_call_ns_sum", "coordinate_ns_sum", "native_save_ns",
            "native_load_ns", "latest_text_materialization_ns",
        ):
            if int(values.get(key, "-1")) < 0:
                raise ValueError("missing native stage timing")
        if int(values["native_call_ns_sum"]) < int(values["native_edit_ns_sum"]):
            raise ValueError("native interval exceeds its enclosing call")
        _artifact(path.with_suffix(".native"), values["native_artifact_bytes"],
                  values["native_artifact_sha256"])
        return {"verified_cuts": 11, "logical_edit_batches": 100000,
                "artifact_sha256": values["native_artifact_sha256"]}
    if any(row.get("engine") != engine for row in rows):
        raise ValueError("output belongs to another engine")
    if workload == "seed":
        baselines = ("", "A😀e\u0301\r\n\0", "abcdef")
        if len(rows) != 3 or {row["case"] for row in rows} != {"0", "1", "2"}:
            raise ValueError("missing seed probe case")
        for row in rows:
            case = int(row["case"])
            if row["text_sha256"] != hashlib.sha256(baselines[case].encode()).hexdigest():
                raise ValueError("wrong seed text")
            _artifact(path.with_suffix(f".{case}.native"), row["artifact_bytes"],
                      row["artifact_sha256"])
        return {"seed_signature": [
            [r["case"], r["artifact_bytes"], r["artifact_sha256"]]
            for r in sorted(rows, key=lambda row: row["case"])
        ]}
    groups = {}
    for row in rows:
        size = int(row["baseline_bytes"])
        groups.setdefault(size, []).append(row)
        native, call, coordinate, materialize = (int(row[k]) for k in (
            "native_ns", "call_ns", "coordinate_ns", "materialize_ns",
        ))
        if min(native, coordinate, materialize) < 0 or call < native:
            raise ValueError("invalid nested native/coordinate timing")
        if workload == "allocations":
            for key in ("allocations", "deallocations", "reallocations",
                        "bytes_allocated", "bytes_deallocated"):
                if int(row[key]) < 0:
                    raise ValueError("invalid allocator observation")
            int(row["bytes_reallocated"])  # Signed realloc delta is not live heap.
    if set(groups) != {10240, 102400}:
        raise ValueError("wrong W01 baselines")
    for group in groups.values():
        if len(group) != 64 or {int(r["sample"]) for r in group} != set(range(64)):
            raise ValueError("incomplete/duplicate W01 samples")
    return {"samples": 128, "baseline_sizes": [10240, 102400]}


def seed_results(steps):
    results = {}
    for engine in ENGINES:
        records = [
            next((s for s in steps if s["name"] == f"candidate-{engine}-seed-{trial}"
                  and s["exit_code"] == 0 and "validated_output" in s), None)
            for trial in range(3)
        ]
        if not all(records):
            results[engine] = {"status": "not_tested"}
            continue
        signatures = [r["validated_output"]["seed_signature"] for r in records]
        results[engine] = {
            "status": "observed_repeatable" if all(s == signatures[0] for s in signatures)
                      else "observed_nonrepeatable",
            "profile": "native full-history seed artifact; not a frozen seed-change format",
            "signatures": signatures,
        }
    return results
