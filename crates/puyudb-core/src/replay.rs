// SPDX-License-Identifier: Apache-2.0

//! Historical reconstruction driver (I08 P2; blueprint §5.8 cost rows, D1/D5).
//!
//! Phase 1 has no physical checkpoints (P3): reconstruction is always full
//! replay over the version's own `closure(H)`, so a read can never observe a
//! checkpoint carrying post-cut changes (§6.6). Every read carries an
//! explicit budget; exceeding it is a typed cancellation, never a partial
//! "latest" fallback (D5).

use rusqlite::Connection;

use crate::history::ReadBudget;
use crate::identity::{EpochId, SpaceId, VersionId};
use crate::publication::Error;
use crate::version;

/// Text of `epoch` at `version`: resolve the VersionId (a missing object is
/// `VersionUnavailable` → `content_unavailable`, never an approximate latest
/// view, D1), then replay the version's closure with `budget`.
///
/// # Errors
/// Returns [`Error::MissingVersion`] for an unknown VersionId,
/// [`Error::ReadCancelled`] when `budget` is exceeded, and the native read's
/// errors otherwise.
pub fn text_at_version(
    conn: &Connection,
    space: &SpaceId,
    version: &VersionId,
    epoch: &EpochId,
    budget: Option<&ReadBudget>,
) -> Result<String, Error> {
    let frontier = version::resolve(conn, space, version)?;
    crate::native::text_at_budgeted(conn, &frontier.heads, epoch, budget)
}
