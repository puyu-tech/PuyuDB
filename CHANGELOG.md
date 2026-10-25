# Changelog

No releases have been published.

## Unreleased

- I01: four-package Rust workspace, shared Apache-2.0 metadata/lints, a pinned
  development toolchain, declared MSRV, and workspace dependency lockfile.
- Help/version-only `puyu` CLI with the `db` namespace and process-level tests.
- Initial CI, dependency policy, contributor/security/provenance policies, and
  implementation/compatibility tracking.
- Confirmed GitHub Private Vulnerability Reporting enabled, closing I01.
- Follow the latest stable toolchain policy: Rust 1.99.1 is now both the pinned
  compiler and minimum; the former 1.85 floor is no longer supported.
- I02 in progress: pinned native validation experiments, byte/hash seed fixtures,
  history/convergence/Unicode/scope tests, executable Automerge/Yrs/Loro
  comparisons, and native-only W01/W02 workloads. No native format is frozen.
- I02 follow-up: safe no-LZ4 upstream DT comparison pin, larger mixed-observation
  delivery/reload corpus with an explicit long release test, expanded historical
  anchors and native-wire negatives, attributed adversarial diagnostics,
  calibrated allocator measurements, and a proposed ADR evidence addendum.
- Optional, tested source-transfer and baseline-collection helpers for native
  Windows and Debian WSL2; no automatic upload or I02 acceptance.
- Executed native Windows, Debian WSL2 and refreshed macOS validation/resource
  baselines; fixed canonical `puyu` help naming exposed by Windows snapshots.
- Maintainer-approved provisional WSL2 scope for I02, retained WSL2 W01/allocation
  samples and a descriptive native-cost assessment. Independent reference Linux
  is deferred; native-semantic/cost acceptance still gates I02 closure and I03.
- Common four-candidate W01/W02 runner with separate translation/native/text and
  allocator stages, 11 saved cuts, fresh-process seed artifacts, resource/failure
  capture and retained comparative evidence from WSL2/macOS. Alternatives remain
  under evaluation; no engine selection, production binding or format freeze.

There is no working database engine, persisted format, server, or SDK.
