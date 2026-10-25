# I02 native Windows and Debian WSL2 collection

This is an optional **developer measurement helper**, not a PuyuDB command,
installer, data export, release, or migration. Python is not required by the
database/CLI build or runtime.

The maintainer has Windows x86-64 and Debian configured as WSL **version 2**.
Collect them separately. A WSL2 result is Linux-VM evidence, not native reference
hardware or a WSL filesystem-support guarantee. The maintainer has
[approved provisional WSL2 evidence for I02](../adr/01-wsl2-validation-scope.md),
without approving the native-profile decision. The [2026-10-25 results](I02-platform-results.md) record the first
executed Windows/WSL runs; the instructions below reproduce the workflow.

## 1. Transfer the current uncommitted source

Do not rely on a GitHub clone to transfer this work: the validation changes in
this thread are local and have not been pushed. A local checkpoint commit does
not make the current research snapshot available on GitHub.

From the current thread worktree on the source machine:

```sh
python3 -m unittest discover -s scripts -p 'test_i02*.py'
python3 scripts/i02_baseline.py pack
```

The helper prints a new `target/i02-source-<UTC timestamp>.zip` path. Copy that
specific archive to your Windows machine manually. No commit/push or remote
upload is performed by the helper.

The archive contains a constrained inventory of Rust manifests/source/test
fixtures, collector scripts, root license/readme files and this guide. It excludes
Git history, build output, `.env` files, Cargo caches and the private architecture
input. Source symlinks are refused. The included `I02_SOURCE_MANIFEST.json` records
file hashes and a source fingerprint, which collection checks before execution
and after all steps. Hashes detect accidental changes; they are not signatures
or proof that arbitrary source is trustworthy. Review the selected source before
transferring it.
Project `.cargo/config` and `.cargo/config.toml` are currently refused pending
explicit review rather than silently omitting build-affecting configuration.
Cargo credential files are never included by the inventory.

## 2. Native Windows x86-64

Prerequisites:

- 64-bit Windows with the native MSVC Rust toolchain; Visual Studio C++ Build
  Tools and Windows SDK/linker available.
- Current 64-bit CPython, accessible through `py -3`.
- Rustup, with the repository's exact toolchain and components:

```powershell
rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy
```

Extract the archive into a **new empty directory**, not over another checkout:

```powershell
Expand-Archive -LiteralPath "$HOME\Downloads\i02-source-<timestamp>.zip" -DestinationPath "C:\work\puyudb-i02"
Set-Location "C:\work\puyudb-i02"
py -3 scripts\i02_baseline.py collect --profile windows-native
```

Replace the timestamp/path with the actual archive. Run this in native
PowerShell, not inside WSL. Compiler-host checks reject Linux tools or a
non-x86-64/non-MSVC host for this profile.
Compiler-selection overrides and wrappers (`RUSTC`, `CARGO_BUILD_RUSTC`, and
the corresponding wrapper variables) must be unset for this narrowly scoped
baseline. The collector pins the probed native rustc executable and disables
inherited wrappers; it does not silently benchmark an unchecked compiler.

## 3. Debian WSL2

Confirm the distribution mode from native Windows:

```powershell
wsl --list --verbose
```

Use the Debian row's `VERSION` column. `wsl --version` alone is not evidence of
the selected distribution mode.

Inside Debian, install missing development prerequisites if needed:

```sh
sudo apt update
sudo apt install build-essential git curl python3 time util-linux unzip
```

An existing **Linux rustup installation** is required. If it is missing, follow
the [official rustup installation instructions](https://rustup.rs/) inside Debian;
the Debian packages above do not install rustup.
Use Linux cargo/rustc, not Windows `cargo.exe` through interoperability. Then
install the repository toolchain with the same rustup command shown above.

Extract into a new directory in the Linux filesystem:

```sh
mkdir -p "$HOME/work/puyudb-i02"
unzip -n /mnt/c/Users/<windows-user>/Downloads/i02-source-<timestamp>.zip -d "$HOME/work/puyudb-i02"
cd "$HOME/work/puyudb-i02"
python3 scripts/i02_baseline.py collect --profile wsl2
```

Adjust the archive's Windows drive/path as required. Reading the archive from a
Windows mount is fine; **building/measuring the source there is not this profile**.
The helper checks both common drive-mount paths and `findmnt` filesystem type;
unknown WSL storage is rejected rather than assumed to be ext4.

Kernel/environment markers are detection evidence, not an infallible WSL version
oracle: custom kernels and disabled interoperability exist. The `wsl2` option
records the operator's confirmed distribution mode, and does not relabel WSL as
native reference Linux.

## 4. What runs and what is saved

The helper runs sequentially:

1. Locked dependency provisioning, then offline Cargo operations.
2. Formatting, all-target/all-feature Clippy, workspace tests.
3. Allocator calibration and the explicitly selected long native release gate.
4. Release example builds, followed by standalone W01, W02 and allocation runs.

Expect several minutes for first compilation and the long tests; W02 previously
peaked near 1 GiB for the whole macOS process. Leave ample free memory (at least
4 GiB is advisable including compilation), close competing workloads, and record
the actual Windows-host RAM and WSL guest limits. These are measurement
preconditions, not new product resource guarantees.

Output goes to a fresh `target/i02-baselines/<profile>-<UTC timestamp>/`:

- `run.json`: exact source/lockfile fingerprints, compiler host and versions,
  available environment facts, commands, exit codes, durations and resources.
- Per-step `.stdout` / `.stderr`: complete raw outputs. The workload `.stdout`
  files are CSV, written as bytes to avoid PowerShell text-redirection encoding.
- Unix `.resources.txt`: raw GNU time output where applicable.

Failed commands produce a nonzero collector exit and `status: failed`; partial
outputs remain available. A successful command that ran zero selected tests or
produced incomplete benchmark CSV is rejected. Source changes during collection
also invalidate the result. Success is only `status: commands_passed`, with
`reference_gate: not_evaluated`—it never approves I02 or changes an ADR.

Compare `source_fingerprint` across Windows/WSL reports. If it differs, the runs
did not measure the same input tree and must not be silently combined.

## 5. Resource-method boundaries

- **Debian/Linux:** `/usr/bin/time -v` measures the standalone Rust executable.
  Kernel maximum RSS is converted from KiB to bytes; raw output is retained.
- **macOS:** `/usr/bin/time -l` is supported for local collector verification;
  Darwin reports maximum RSS in bytes.
- **Windows:** a retained query-only process handle supplies final process CPU
  times. Live OS peak-working-set samples are reported strictly as a **lower
  bound**, not an exact final peak. No private Python subprocess handles,
  administrator privilege or post-exit-memory guarantee is assumed.
- Failed/unavailable resource queries remain `null`/`unavailable`, never invented
  zero measurements. Workload success and resource completeness are separate.

Windows working set and Linux RSS are platform-specific quantities, not heap
allocation counts or directly interchangeable accounting contracts. CPU totals
exclude descendants; current standalone workloads do not launch children.

Fill the `operator_context_required` fields separately: physical cores, host RAM,
WSL guest memory limit, SSD model, encryption, power mode and background load.
Do not invent unknown fields or treat a shared/virtualized machine as the
controlled reference class. Review logs before sharing; compiler diagnostics can
contain local paths. Rust flag values are not automatically copied into the
report: only presence/digests are recorded, so provide separately reviewed,
sanitized flag details if any are set. Global Cargo configuration is not exported
and must also be accounted for during review. No device serials or whole-environment
dump is collected.

Return the two complete result directories (or archives made from just those
directories), not a Cargo `target` tree or your `.git` directory. No automatic
upload or publication exists.

## 6. Common four-candidate comparison

Add `--comparison` to collection to execute the common trusted native workload
for Automerge, Yrs, Loro and the pinned no-LZ4 diamond-types Git version:

```sh
python3 scripts/i02_baseline.py collect --profile wsl2 --comparison
```

Use `py -3` and `--profile windows-native` for native Windows, or
`--profile macos-native` on macOS. Transfer a freshly packed input tree first:
the original platform-run archive predates this runner.

The common native cost profile is distinct from the original Automerge-specific
metadata/inspection experiment. It uses fixed candidate-native identities,
history-preserving settings, and no human edit messages. It does not implement
a PuyuDB batch ledger, authenticated admission, or a production adapter.

After the seven setup/check/build steps, the collector runs the explicit release
history fixture, then 32 candidate processes: three fresh seed probes and three
W01 runs per engine, one separate instrumented allocation run, and one W02 run.
Process order rotates for repeated probes/W01. Native cases have a recorded
300-second timeout. A candidate failure does not suppress the other cases:
outputs and validation failures are retained, and collection exits nonzero
after recording them all. Setup/source failures still stop immediately.

W02 uses the same 1 MiB/100,000-batch operation schedule across all candidates
and checks 11 saved cuts, including the short paste/delete/restore intervals.
Each query starts from a fresh native load. Coordinate translation, native
mutation/commit, complete wrapper calls, native encoding, latest-state load
and historical reconstruction have separate timing fields. These are not
production admission, durable IO or cross-engine atomic-transaction guarantees.

Comparison revision 2 also times UTF-8 materialization separately after setup,
load, and every W01 edit. Native edit API returns are not a promise of eager
physical materialization: DT can defer rope work to a later edit/read. W02's
native-call sums therefore are not complete edit-to-render costs; its complete
history interval includes cut reads. Seed text is read before W02 edits, and
every historical query starts from a fresh load plus a separately timed latest
text read. No artificial per-edit DT flush or full 1 MiB render is imposed in W02.

Native full-history artifacts and seed artifacts are retained beside raw CSVs.
Their codecs/compression and native history models differ; exported byte counts
are not complete PuyuDB authoritative-storage costs. Original per-update bytes
and application transaction boundaries are separate, not implied by old-text reads.

A completed workflow says `comparison_recorded`, not engine selected. Seed
repeatability is reported separately, including nonrepeatable bytes; no
`reference_gate` or ADR approval is inferred. The comparison report must keep
unexecuted/missing gates distinct from observed profile failures.

## Verification status

The original collector's 13 contract tests and original collection workflow
passed on native Windows x86-64, Debian WSL2 and macOS ARM64. Windows ctypes/CIM produced
actual CPU and lower-bound working-set observations; see the linked results.
These runs do not establish a controlled reference-Linux result, an exact Windows
memory upper bound, general Windows/WSL filesystem support or I02 approval.
The expanded comparison's separate evidence and current test counts belong to
its run reports, not retrospectively to the original three platform runs.

Primary references:

- [Microsoft process times](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes)
- [Microsoft process memory info](https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-getprocessmemoryinfo)
- [Process memory counter layout](https://learn.microsoft.com/en-us/windows/win32/api/psapi/ns-psapi-process_memory_counters)
- [Query rights and OpenProcess](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openprocess)
- [WSL commands](https://learn.microsoft.com/en-us/windows/wsl/basic-commands)
- [WSL filesystems](https://learn.microsoft.com/en-us/windows/wsl/filesystems)
- [WSL configuration/custom kernels](https://learn.microsoft.com/en-us/windows/wsl/wsl-config)
