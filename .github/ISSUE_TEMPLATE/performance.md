---
name: Reproducible performance
about: Report a measured performance problem with a repeatable workload.
title: ""
labels: ""
assignees: ""
---

Use synthetic workloads; do not upload private data or sensitive logs. Security
concerns belong under the [security policy](https://github.com/puyu-tech/puyudb/security/policy).

## Implemented path and problem

What implemented operation is slow, and why does it matter? Do not present
scaffolding or CLI help measurements as database performance.

## Environment

Commit or version, Rust toolchain, build profile and flags, operating system,
CPU, memory, storage/filesystem, and relevant background load:

## Reproducible workload

Provide commands or a small benchmark, synthetic data generation steps and seed,
data sizes, operation counts, concurrency, and warm/cold cache conditions.

## Raw results

Include units, number of runs, raw timings and relevant memory or I/O measurements,
and variability. State the measurement tool and method.

## Comparison and expected improvement

If comparing versions or alternatives, use the same environment and workload and
include both results. Explain any target and its basis; do not invent a project
performance budget.
