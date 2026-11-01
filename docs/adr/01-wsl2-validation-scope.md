# ADR-01 amendment: provisional WSL2 evidence for I02

**Status: Accepted for the I02 measurement-platform substitution only.**

Approval: the maintainer explicitly instructed on 2026-10-25 that no Linux PC
is currently available and the project must use WSL2 for now. This accepts
the bounded platform change below, not the proposed native semantic profile.

## Context

The measurement policy calls for independent Linux x86-64 reference hardware,
with native macOS/Windows portability and resource baselines. The available
machine supplies native Windows x86-64 and Debian WSL2. Both, and the existing
macOS ARM64 machine, have completed the [recorded native-engine runs](../validation/I02-platform-results.md).
WSL2 is a Linux VM with guest limits and host interference, not the specified
8-physical-core/32-GiB reference machine.

## Decision

Use the explicitly identified **Debian WSL2, x86-64, Linux-filesystem** results
as the provisional Linux-side W01/W02 evidence for **I02 native feasibility**.
Lack of a separate Linux PC no longer blocks that I02 evidence criterion.
Assess the observations and limitations explicitly; a command exit code alone
does not accept costs or close I02.

Do not relabel WSL2 as native reference Linux, or change a report's
`reference_gate: not_evaluated` into a successful measurement. This ADR records
the owner's scope decision separately from those immutable observations.

Independent native Linux measurements are **deferred, not declared passed**.
Revisit when hardware is available and in I36 before declaring native Linux
support or Linux production-performance budgets. Any later waiver requires
its own recorded decision.

## Unchanged requirements

- Native seed equality, history retention, Unicode, anchor and operation-scope
  correctness gates remain required.
- The [native-profile/counter decision](01-native-text-evidence.md) remains
  proposed. This platform amendment does not approve its semantics or start I03.
- No native or PuyuDB wire/persistent format is frozen.
- Windows sampled working-set values remain lower bounds, not upper budgets.
- No production decoder/resource-safety, durability, filesystem, CI or general
  WSL support guarantee follows from these measurements.
- Durable IO and publication acceptance belong to later storage integration;
  native-only compute is not durable commit latency.

## Consequences and alternatives

The project can evaluate the measured native candidate without waiting
indefinitely for unavailable hardware. Host/guest scheduling, available RAM,
mount/storage differences and uncontrolled background load limit transferability.
Single-run observations cannot establish controlled-machine confidence intervals
or regression thresholds.

Waiting for a native machine was considered and is no longer the chosen I02
path. Provisioning paid/remote infrastructure is not implied or authorized by
this decision. Pretending that WSL2 is bare metal is rejected.

The [cost assessment](../validation/I02-cost-assessment.md) states what the
current evidence supports. I02 still needs a recorded native-profile decision
before I03; this accepted amendment must not be read as acceptance of all ADR-01.
