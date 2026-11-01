---
name: Possible data loss
about: Report missing, corrupted, or unexpectedly overwritten state safely.
title: ""
labels: ""
assignees: ""
---

Stop writes to affected data where practical. Preserve original files and logs,
and make a separate backup without overwriting earlier backups. Do not run
destructive repair, delete state, or overwrite the affected files to reproduce.
Recovery is not guaranteed.

Do not upload private source code, database files, histories, credentials, or
sensitive logs. Use synthetic inputs and redacted evidence. If a security issue
may be involved, follow the [security policy](https://github.com/puyu-tech/puyudb/security/policy)
instead of posting exploit details.

## Version and environment

Commit or version, operating system, filesystem/storage, and relevant configuration:

## What appears lost or changed?

Describe the expected state, observed state, and how each was verified without
sharing private data.

## Events before the problem

List operations in order, including crashes, power loss, concurrent access, or
storage errors if known. State what is unknown rather than guessing.

## Preservation and safe reproduction

What backups or untouched copies exist? What actions have already been taken?
If possible, provide a reproduction using disposable synthetic data, never the
only copy of affected data.
