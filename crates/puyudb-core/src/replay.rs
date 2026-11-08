// SPDX-License-Identifier: Apache-2.0

//! Historical reconstruction driver (I08 P2/P3; blueprint §5.8 cost rows,
//! §6.6, D1/D3/D5).
//!
//! A read at version V first tries the hash-checked physical checkpoint of V
//! (a cached native view keyed by the VersionId itself — it can never carry
//! post-cut changes, §6.6). Missing or corrupt checkpoints are discarded and
//! the read falls back to full replay over the version's own `closure(H)`;
//! canonical history alone always answers (D3). Every read carries an
//! explicit budget; exceeding it is a typed cancellation, never a partial
//! "latest" fallback (D5).

use std::path::Path;

use rusqlite::Connection;

use crate::checkpoint;
use crate::history::ReadBudget;
use crate::identity::{EpochId, SpaceId, VersionId};
use crate::publication::Error;
use crate::version;

/// Text of `epoch` at `version`: resolve the VersionId (a missing object is
/// `VersionUnavailable` → `content_unavailable`, never an approximate latest
/// view, D1), then the checkpointed native view of that exact version if a
/// valid one is cached, else full replay of the version's closure under
/// `budget`.
///
/// # Errors
/// Returns [`Error::MissingVersion`] for an unknown VersionId,
/// [`Error::ReadCancelled`] when `budget` is exceeded, and the native read's
/// errors otherwise.
pub fn text_at_version(
    conn: &Connection,
    space_dir: &Path,
    space: &SpaceId,
    version: &VersionId,
    epoch: &EpochId,
    budget: Option<&ReadBudget>,
) -> Result<String, Error> {
    let frontier = version::resolve(conn, space, version)?;
    if let Some(cp) = checkpoint::read_checkpoint(space_dir, space, version)?
        && let Some((_, doc)) = cp.docs.iter().find(|(e, _)| e == epoch)
    {
        let text = puyudb_crdt::dt_text::TextDoc::load(doc)
            .map_err(|err| Error::Corrupt(format!("cached native view is undecodable: {err}")))?
            .text();
        if let Some(budget) = budget {
            budget.consume_output(text.len() as u64)?;
        }
        return Ok(text);
    }
    crate::native::text_at_budgeted(conn, &frontier.heads, epoch, budget)
}
