# Architecture glossary

These terms guide future implementation; they are not available APIs or frozen
wire/storage schemas.

| Term | Meaning and boundary |
| --- | --- |
| Puyu / PuyuDB | Umbrella brand / database and synchronization engine |
| Space | Unit of sharing, authorization, replication, and atomic local history publication; no cross-space transaction promise |
| Repository binding | Association with a Git repository; identity is not a remote URL |
| Branch view | Named history frontier, independent of disk checkout state |
| Checkout | Machine-local materialization, not authoritative history |
| Batch | Immutable atomic PuyuDB history publication, distinct from a native CRDT change and a Git object |
| Version | A normalized frontier and its complete ancestor closure, not "whatever is latest" |
| Intent base | The captured version against which an edit observes positions and metadata; not implicitly replaced by current branch heads |
| Native change | A text-engine change with its own native identity and dependency scope |
| Git object ID | Git's typed object identity, not a batch or native-change ID |
| File identity / content epoch | Stable file identity across path changes / one retained content lineage |
| Stored / applied / hydrated / materialized | Staged bytes / validated closed history with required native data and opaque manifests / all reachable content resident and verified / reflected on disk |
| Local durable receipt | Requires durable publication and all newly introduced local content resident; not a filesystem or network receipt |
| Authenticated writer | Verified signing identity; not necessarily the asserted human/agent author |
| Authorship assertion | Attributed subject or observed source; not proof of authentication or permission |
| Authorization | Separately enforced permission, never inferred from authorship, actor IDs, or content hashes |

## Ownership rules

- Core will own history validation, versions, atomic publication, and queries.
- Native text integration will own native changes/seeds/views, not repository
  metadata conflicts or permissions.
- Local integration will coordinate Git/filesystem effects through core
  operations; these effects are not participants in a SQLite transaction.
- The CLI will parse, call reusable interfaces, and present results. It must not
  become the only implementation of a database operation.
- Embedding must remain possible offline without a daemon, account, editor,
  AI provider, or agent framework.

See the [ADR index](adr/README.md) for decision gates and
[implementation status](IMPLEMENTATION_STATUS.md) for actual behavior.
