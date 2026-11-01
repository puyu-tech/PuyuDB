---
name: Correctness or convergence
about: Report incorrect behavior or a reproducible convergence failure.
title: ""
labels: ""
assignees: ""
---

Do not include private data, secrets, or security exploit details. For security
concerns, follow the [security policy](https://github.com/puyu-tech/puyudb/security/policy).
Use synthetic inputs and redacted logs.

## Version and environment

Commit or version, Rust toolchain, operating system, and relevant configuration:

## Expected and observed behavior

What invariant or documented behavior was violated? Identify the implemented
component involved; distinguish a bug from a capability not implemented yet.

## Minimal reproduction

Provide exact commands or a small test, inputs, operation order, and observed output.
For convergence problems, include actors, concurrent operations, merge order,
offline periods, and the final state at each actor, where applicable.

## Repeatability and evidence

How often does it reproduce? Include a seed if randomized, relevant redacted logs,
and any smaller passing or failing case.
