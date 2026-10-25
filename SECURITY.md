# Security policy

## Current status

PuyuDB is early scaffolding. There are **no supported releases**, no working
database engine, and no supported persisted data format. It is **not ready for
production use** or for storing the only copy of valuable data. See
[implementation status](docs/IMPLEMENTATION_STATUS.md) and
[compatibility](docs/COMPATIBILITY.md).

There is no staffed security response team, guaranteed response time, or security
SLA. This policy does not promise a fix, release, or coordinated disclosure
timeline.

## Reporting a concern

Do not post exploit details, credentials, private source code, database contents,
or sensitive logs in a public issue, pull request, or discussion.

GitHub Private Vulnerability Reporting is enabled for this repository, confirmed
through GitHub's public API during I01 closure. Use
[a private vulnerability report](https://github.com/puyu-tech/puyudb/security/advisories/new)
to send vulnerability details privately. Enabling the channel does not establish
a staffed response service or a guaranteed disclosure timeline.

If that option is unavailable, do not publish sensitive information as a fallback.
You may open a non-sensitive request asking maintainers to provide a private
contact, without describing the exploit or affected private data. No alternative
private contact is currently confirmed.

Share only the minimum needed to investigate: the affected commit and environment,
a synthetic reproducer, expected and observed behavior, and the suspected impact.
Redact secrets and personal information. Coordinate any disclosure rather than
assuming a response or publication date.
