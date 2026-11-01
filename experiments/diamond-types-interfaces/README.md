# DT native-interface maintainability spike

This is a disposable experiment, not an engine selection, production fork,
new persistent format or dependency promotion. The main workspace still uses
the original pinned Git dependency.

## Question and stop conditions

Can two small native interfaces address the observed DT-fit gaps?

1. Resolve remote-ID text anchors using native identity/deleted-item ordering.
2. Inspect a complete new-operation update through the existing native decoder,
   without relying on byte-equal re-encoding or losing known-prefix input.

Reuse the existing native merger and decoder. Do not add a second merger,
independent tombstone CRDT, replacement binary parser, actor-order override or
new native wire version. A patch that needs those changes is a stop/reassess
result, not permission to grow a separate engine.

Passing bounded tests establishes a prototype, not general conformance, panic/
resource safety, upstream acceptance or a promise of inexpensive future rebases.

## Source and reproducibility

Source pin: `89ae3a0ab8d941a2885e6ec89d094bc9ce9d2922`, manifest 2.0.0,
default LZ4/storage features disabled. This is not a released stable 2.0 package.

The runner finds the exact cache source through frozen Cargo metadata, reads the
pinned Git object with `git archive`, and extracts it into a fresh, non-Git tree:

```text
target/dt-interfaces/<UTC timestamp>-<mode>/
  upstream/             disposable source generated from the pinned object
  harness/              separate Cargo workspace with a native path dependency
  *.stdout, *.stderr    complete command outputs
  manifest.json         inputs, dependency selections, commands and hashes
```

Cargo's cache checkout is never edited. No upstream source is permanently
vendored. Only the reviewable patches, independently written test drivers and
runner are retained here. Patches target ISC upstream code; the harness/tests
are original Apache-2.0 code. This is not redistribution/legal clearance; the
previously recorded upstream notice/dependency obligations remain.
Four historical native byte vectors are quoted from the pinned upstream encoding
tests with attribution for compatibility checking; no upstream test implementation
is copied. New contributions, upstream submission terms and complete distribution
notices still require review before any publication.

The generated harness is seeded from the main lockfile. Offline resolution must
use registry package versions/sources already present in the main graph. DT and
`rle` become local path packages only inside that harness. The runner checks that
main `Cargo.toml`, `Cargo.lock` and `rust-toolchain.toml` are unchanged.

## Run

Provision the pinned Rust toolchain and dependency cache as for the main project,
then run from the repository root:

```sh
python3 experiments/diamond-types-interfaces/run.py --mode baseline
python3 experiments/diamond-types-interfaces/run.py --mode anchors
python3 experiments/diamond-types-interfaces/run.py --mode strict
python3 experiments/diamond-types-interfaces/run.py --mode combined
```

Use `py -3.14` instead of `python3` on the authorized native Windows test machine.
Nonempty compiler/target overrides are refused rather than silently changing the
experiment. Outputs always use a fresh directory; reruns do not replace evidence.

Each run checks source-extraction contracts and compares 15 native text/encoded
byte observations before/after the selected patches. These cover empty/Unicode/
ASCII seeds plus six concurrent insertion/deletion delivery orders and reloads.
The mode-specific Rust tests run in debug and release, followed by harness Clippy
and formatting. `baseline` selects no extension test driver; its positive evidence
is the executed compatibility example and runner checks, not extension tests.

The byte observations are identical-default-behavior controls, not a complete
binary-format compatibility suite or performance measurement. The strict
new-operation interface is not the duplicate/replay interface: already known
operation spans are rejected, while existing native replay behavior stays intact.
PuyuDB outer identity, metadata policy, authentication and publication remain
separate from the generic native seam.

## Native default-feature regression

An independent review caught a lifetime defect in the first borrowed metadata
report: enabling native LZ4 made a returned start-text reference escape a local
decompression buffer. The corrected report owns metadata values, copied only
in strict mode; the ordinary decoder does not allocate those report values.

The optional check compiles real native `default,lz4,storage` features and executes
a compressed start-content/update/snapshot control in debug and release.
Strict mode still refuses compressed input. This is a separate compatibility
graph, not a feature change to the main or default-off spike graph.

Download the exact published archives into ignored `target/`, then run:

```sh
mkdir -p target/dt-interface-feature-archives
curl --fail --location https://static.crates.io/crates/lz4_flex/lz4_flex-0.14.0.crate --output target/dt-interface-feature-archives/lz4_flex-0.14.0.crate
curl --fail --location https://static.crates.io/crates/twox-hash/twox-hash-2.1.2.crate --output target/dt-interface-feature-archives/twox-hash-2.1.2.crate
python3 experiments/diamond-types-interfaces/run.py --mode combined --native-defaults-archives target/dt-interface-feature-archives
```

The runner verifies archive SHA-256 against the official registry checksums before
extraction, and uses local path copies of those exact sources without editing
Cargo's registry or Git source caches. The native pin already declares optional
`lz4_flex ^0.14.0`; its frame feature uses `twox-hash ^2.0.0`. These two temporary
path packages are additional to the core experiment's native/harness path packages.
No new registry version is permitted and main dependency inputs remain unchanged.
The package manifests declare MIT and no build script; distribution/advisory
qualification for promoting them is not supplied by this compatibility check.

This checks the actual 0.14.0 implementation, not the earlier diagnostic stand-in.
It is not the full upstream feature/MSRV/platform matrix or decoder hardening.

Native upstream defaults include `storage`, whose existing source uses Unix-only
file APIs and does not compile on native Windows. The PuyuDB text experiment
already disables that unrelated storage module. On Windows, test the real LZ4
text path explicitly without storage:

```sh
py -3.14 experiments/diamond-types-interfaces/run.py --mode combined --native-defaults-archives target/dt-interface-feature-archives --native-feature-profile lz4
```

The default-feature Windows failure is retained as an upstream compatibility
limitation, not hidden by this narrower check or “fixed” with a storage rewrite.
The manifest records the exact selected feature set.
