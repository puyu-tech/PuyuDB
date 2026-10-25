# I02 platform runs — 2026-10-25

**Native Windows, Debian WSL2 and macOS completed the collection workflow.**
These are native-engine portability/resource observations, not I02 acceptance,
production database benchmarks, supported-platform declarations or CI results.
The maintainer subsequently [accepted provisional WSL2 evidence for I02](../adr/01-wsl2-validation-scope.md).
The native-profile/cost stop-go remains unaccepted; native reference-Linux
measurements are deferred, not relabeled as passed.

## Inputs and provenance

All three successful runs used the same transferred source inventory:

- Source fingerprint: `dc9cc73733fed3e5ea9a6da3b18606dc5ead01fbf4fdaf1ce3191d123787c289`.
- Source archive SHA-256: `b24342477b9072f8776de50563d5c6f9f6d355216cca5b13b21b60d4100729a7`.
- Cargo.lock SHA-256: `a31fc17f20dda26b52772c23d3500842fdc372e8c691102987baf73db56851b4`.
- Rust/Cargo 1.99.1; rustc `58cf8f9c35b355199727627d07ae34a4e2846d67`,
  LLVM 23.1.1. `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` were unset.
- Native versions remain those in the dependency ledger: Automerge 0.12.0,
  Yrs 0.28.0, Loro 1.17.1 and the full-revision, unpublished DT 2.0.0 pin.

The archived input includes the platform guide as well as code. Subsequent
guide/status edits documenting these results do not change the measured Rust
or collector code, but do change a newly packed inventory's fingerprint.
Use the recorded archive/report hashes, not an assumption that later documentation
has identical bytes.

Windows and WSL used new, separate non-Git test directories on the maintainer's
machine, accessed through the authorized SSH connection. Existing checkouts were
not used or modified. The collection workflow did not commit, push, merge,
publish or deploy.

## Actual environments

| Run | Compiler host | Environment |
| --- | --- | --- |
| `windows-native-20261025T190412724673Z` | `x86_64-pc-windows-msvc` | Windows 11 Pro, build 26200; Python 3.14.7 |
| `wsl2-20261025T190808885690Z` | `x86_64-unknown-linux-gnu` | Debian 12; kernel `6.18.54.1-microsoft-standard-WSL2`; Python 3.11.2; ext4 |
| `macos-native-20261025T191247038915Z` | `aarch64-apple-darwin` | macOS 27.0.1, Darwin 27.0.0; Python 3.9.6 |

The Windows host has an AMD Ryzen 5 3600, 6 physical/12 logical cores and
17,101,266,944 bytes of physical RAM. Post-run read-only probes identified the
test drive as NTFS on an ADATA SX8200PNP NVMe device, the active power plan as
AMD Ryzen High Performance, and BitLocker volume/protection/encryption states as
zero (fully decrypted/off/none). These are observations, not proof that every
setting or background workload stayed fixed during the run.

Debian was explicitly listed as WSL version 2. The workload ran in its Linux
home directory on ext4, not a Windows drive mount; reported guest `MemTotal`
was 8,094,516 KiB. Its VHD host-storage mapping and configured memory-limit
source were not established. macOS uses an Apple M1 Max with 32 GiB RAM and
10 physical/logical cores. Neither Windows/WSL nor this Mac is the specified
8-physical-core/32-GiB reference-Linux machine. Background load was not controlled.

With maintainer permission, Rust 1.99.1, rustfmt and Clippy were installed
per user on Windows and Debian; Python 3.14.7 was installed per user on Windows.
Rustup installer hashes were checked against the official distribution checksums.
Existing Visual Studio Build Tools 2019 were sufficient; no replacement was
installed. Debian's missing development/measurement packages were installed,
including `time`, `unzip` and `pkg-config`, with required apt dependency updates.
No distribution upgrade or security-setting change was performed.

## Checks actually executed

Each collector run completed these ten steps with exit code zero:

```sh
cargo fetch --locked
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --frozen
cargo test -p puyudb-crdt --example native_allocations --frozen
cargo test --release -p puyudb-crdt --test mixed_histories thousands_of_seeded_partition_duplicate_restart_schedules --frozen -- --ignored
cargo build --release -p puyudb-crdt --examples --frozen
# Standalone target/<host>/release/examples executables (.exe on Windows):
native_workloads w01
native_workloads w02
native_allocations
```

After locked provisioning, Cargo network access was disabled for the remaining
steps. Each platform passed 47 default integration tests; the one default-ignored
long test was then explicitly executed and passed, covering 2,048 seeded delivery
schedules. Allocator calibration passed separately. All 13 Python collector
contract tests also passed on native Windows and Debian, as well as locally.
All reports say `commands_passed` and `reference_gate: not_evaluated`.

### Windows failure and correction

The first Windows attempt, `windows-native-20261025T190026167263Z`, correctly
stopped at tests: four CLI snapshots failed because Clap derived `puyu.exe` from
the executable filename. Its source fingerprint was the earlier
`04ba1b131594228ba9a51562a83f61d66cfa9cec3480988d67d8382c06ec750c`.

The CLI now explicitly sets its public binary name to `puyu`. The existing
strict snapshots were not weakened or changed. The full successful runs above
use this fix, and the failed report is retained separately rather than relabeled.

## Native workload evidence

Each W01 and allocation run contains 64 measured samples for each of the
10 KiB/100 KiB inputs, after the existing warmups. This report does not infer
cross-engine rankings or compare instrumented and uninstrumented timings.
The [cost assessment](I02-cost-assessment.md) evaluates the WSL2 samples under
the explicitly approved platform scope. Its [raw W01](platform-wsl2-w01.csv) and
[allocation samples](platform-wsl2-allocations.csv) are also retained in the repository.

The complete W02 CSVs are retained here:

- [Windows](platform-windows-w02.csv).
- [Debian WSL2](platform-wsl2-w02.csv).
- [macOS](platform-macos-w02.csv).

Each checked a 1 MiB baseline, 100,000 native edit transactions, 100,001 retained
changes, five saved cuts after loading, and the same final text digest.
Cuts are at 0/25,000/50,000/75,000/100,000 transactions: four earlier versions
and the final state. All three reported 13,653,102 uncompressed and 291,896
compressed native snapshot bytes.
These are encoded buffer sizes, not allocated disk blocks or a frozen-format
compatibility promise. Native decoding is from an in-memory buffer, not a
filesystem-cold load.

| W02 process | Recorded resident-memory observation | Quality |
| --- | ---: | --- |
| Windows | 476,053,504 bytes peak working set | **Lower bound**, 2,218 accepted live OS-peak samples |
| Debian WSL2 | 438,824,960 bytes maximum RSS | GNU `time -v`, KiB converted to bytes |
| macOS | 920,322,048 bytes maximum RSS | Darwin `time -l`, native byte units |

Windows CPU user/kernel totals were 20,296,875,000 / 2,250,000,000 ns; resource
queries reported no errors. Unix kernel lifetime maxima and Windows sampled
working-set bounds are not interchangeable metrics or per-document heap sizes.
These whole-process measurements include original/reloaded documents and
temporaries. The collector/build/Cargo process is outside the measurement.

## Raw evidence retention

Complete reports, raw step stdout/stderr and Unix resource outputs remain
in ignored local directories. WSL2 W01/allocation and all three W02 CSVs have
byte-identical copies in the repository; other raw outputs stay here:

- `target/i02-remote/windows-native-20261025T190412724673Z/`
- `target/i02-remote/wsl2-20261025T190808885690Z/`
- `target/i02-baselines/macos-native-20261025T191247038915Z/`
- Failed first run: `target/i02-remote/windows-native-20261025T190026167263Z/`.

They were retrieved from the remote test directories; nothing was uploaded to
a public service. Raw reports contain local paths, so they are not copied into
public documentation wholesale. Preserve the complete result directories for
the eventual gate review; cloning the repository does not transfer ignored
artifacts. The repository W02 copies are byte-identical to the corresponding
raw stdout files.

| Profile | Raw file | SHA-256 |
| --- | --- | --- |
| Windows | `run.json` | `34aff5cd867aba6d02529d10f7b2b69aaaca795f941a9d885a4a2f77b13311bf` |
| Windows | `w01.stdout` | `957235d53743ad570e0ebb1dea01ec3b8ffd8b632d0efb58e7bfe5645d068fc3` |
| Windows | `w02.stdout` | `183ef549c7341c3bf8d1754c2b23dc5e16b0a6701547b60d13eb789ca881fc20` |
| Windows | `w01-allocations.stdout` | `ff085aadda7a090d1e453a2d30738113d1cdf671b868dea64c4dae5a50663093` |
| WSL2 | `run.json` | `d3b8e262e74e7bc80da6432d99f6a2ebcaff5d2c30f95ddab9b17b21778db63d` |
| WSL2 | `w01.stdout` | `cbdae5efde96199be1a004bd2073787542745b642972b9d09ce7561cda9ec491` |
| WSL2 | `w02.stdout` | `6bfa36498b7e9119766f7b476d45422c901777694e83bcc6109512fe36f839f1` |
| WSL2 | `w01-allocations.stdout` | `b9d0543111d396d14a7e52c73abb53906bfe178e76a154211afa100debb69839` |
| macOS | `run.json` | `d220ada080a6def85ca8e89158a5560f41ff97d05c14c2a981aa0a62f3dcaff8` |
| macOS | `w01.stdout` | `7b329edfff72bc44f4e52049f2f69595a4d82144844a86ee190d272b270a035e` |
| macOS | `w02.stdout` | `f168f9d6e9155dc0967e0db78699667af9de166a562ca30656f0f28341716023` |
| macOS | `w01-allocations.stdout` | `9f2b54a7e410e400302c9ab8570816a34efe80e91f02ca7f19db816419dcc8b8` |

## Stop/go

The Windows portability execution gap and previously missing WSL observations
are closed at this experimental seam. Native CI, filesystem/platform guarantees,
durable latency and safe production admission remain unverified.

The maintainer has accepted the scoped WSL2 amendment, so unavailable reference
hardware no longer blocks the current I02 platform criterion. I02 still requires
acceptance of the proposed native-semantic/counter/cost decision. The Windows
memory observation is a lower bound, not a demonstrated upper resource budget.
**Do not start I03 from these results or the platform amendment alone.**
