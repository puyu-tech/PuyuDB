// SPDX-License-Identifier: Apache-2.0

//! VersionId resolution and frontier validation against the batch DAG
//! (I08 P1; spec §5, blueprint §5.3/§6.3).
//!
//! Only a normalized frontier is hashed, stored, or compared. Normalization
//! (dedup + ancestor dominance over the DAG + sort) is producer-side
//! construction and lives here; decoding ([`Frontier::decode`]) validates and
//! never repairs received bytes (D2).

use std::collections::HashSet;

use rusqlite::Connection;

use crate::identity::{BatchId, SpaceId, VersionId};
use crate::profile::{self, Frontier};
use crate::projection::{self, Error as ProjectionError};
use crate::publication::Error;

fn encoding(detail: impl Into<String>) -> Error {
    Error::Projection(ProjectionError::Encoding(detail.into()))
}

fn profile_err(err: profile::Error) -> Error {
    Error::Projection(ProjectionError::Profile(err))
}

/// Heads in `heads` that are ancestors of another head in the list, provable
/// from the visible batch DAG (spec §5 dominance step — computed over the
/// DAG, never over arrival order or lamport values).
fn dominated_heads(conn: &Connection, heads: &[BatchId]) -> Result<HashSet<BatchId>, Error> {
    let mut out = HashSet::new();
    for chunk in heads.chunks(projection::SEED_CHUNK) {
        let values = projection::values_list(chunk.len());
        let sql = format!(
            "WITH RECURSIVE h(x) AS (
               SELECT column1 FROM (VALUES {values}) v
               JOIN batches b ON b.batch_id = v.column1 AND b.quarantined = 0
             ),
             reach(root, node) AS (
               SELECT h.x, h.x FROM h
               UNION
               SELECT r.root, bp.parent FROM reach r
               JOIN batch_parents bp ON bp.child = r.node
               JOIN batches b ON b.batch_id = bp.parent AND b.quarantined = 0
             )
             SELECT h2.x FROM h h2
             WHERE EXISTS (SELECT 1 FROM reach r WHERE r.node = h2.x AND r.root != h2.x)"
        );
        let mut statement = conn.prepare(&sql)?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(projection::seed_params(chunk)),
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        for row in rows {
            let bytes = row?;
            out.insert(BatchId::from_bytes(&bytes).map_err(|err| encoding(err.to_string()))?);
        }
    }
    Ok(out)
}

/// Producer-side normalization (spec §5 steps 1–3): dedup, drop every head
/// that is an ancestor of another head, sort ascending by raw bytes.
///
/// # Errors
/// Returns `invalid_encoding` when the head list exceeds the frontier cap.
pub fn normalize(conn: &Connection, space: &SpaceId, heads: &[BatchId]) -> Result<Frontier, Error> {
    if heads.len() > profile::MAX_FRONTIER_HEADS {
        return Err(encoding("frontier heads exceed the profile cap"));
    }
    let mut unique = heads.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let dominated = dominated_heads(conn, &unique)?;
    unique.retain(|head| !dominated.contains(head));
    Ok(Frontier {
        space: *space,
        heads: unique,
    })
}

/// Consumer-side validation (D2, spec §5): the decoded head list is already
/// sorted unique ([`Frontier::decode`]); here a head that is provably an
/// ancestor of another head in the local DAG is rejected as redundant —
/// `invalid_encoding`, never silently dropped, because silently normalizing
/// would let two replicas disagree on a version's identity.
///
/// # Errors
/// Returns `invalid_encoding` for a redundant head and SQLite errors.
pub fn validate_dominance(conn: &Connection, frontier: &Frontier) -> Result<(), Error> {
    let dominated = dominated_heads(conn, &frontier.heads)?;
    if let Some(head) = dominated.iter().next() {
        return Err(encoding(format!(
            "frontier head {head} is an ancestor of another head"
        )));
    }
    Ok(())
}

/// Visibility gate for hashing a raw cut: within the cap and naming only
/// visible publications. A cut with an unknown or quarantined head would
/// mint a version that can never resolve, so it is rejected instead (D1).
fn check_visible(conn: &Connection, cut: &[BatchId]) -> Result<(), Error> {
    if cut.len() > profile::MAX_FRONTIER_HEADS {
        return Err(encoding("cut exceeds the frontier cap"));
    }
    for chunk in cut.chunks(projection::SEED_CHUNK) {
        let values = projection::values_list(chunk.len());
        let sql = format!(
            "SELECT COUNT(*) FROM batches b
             WHERE b.quarantined = 0 AND b.batch_id IN (SELECT column1 FROM (VALUES {values}))"
        );
        let visible: i64 = conn.query_row(
            &sql,
            rusqlite::params_from_iter(projection::seed_params(chunk)),
            |row| row.get(0),
        )?;
        if visible != chunk.len() as i64 {
            return Err(encoding(
                "cut names a batch that is not a visible publication",
            ));
        }
    }
    Ok(())
}

/// VersionId of a raw head cut (producer side): visibility gate, normalize,
/// hash. The result depends only on the DAG, never on arrival order.
///
/// # Errors
/// Returns `invalid_encoding` for an over-cap or non-visible cut.
pub fn version_of_cut(
    conn: &Connection,
    space: &SpaceId,
    cut: &[BatchId],
) -> Result<VersionId, Error> {
    check_visible(conn, cut)?;
    let frontier = normalize(conn, space, cut)?;
    frontier.id().map_err(profile_err)
}

/// Store one immutable frontier object and return its VersionId. Idempotent
/// by VersionId; runs inside the caller's transaction.
pub(crate) fn record_frontier(conn: &Connection, frontier: &Frontier) -> Result<VersionId, Error> {
    let body = frontier.encode().map_err(profile_err)?;
    let id = frontier.id().map_err(profile_err)?;
    conn.execute(
        "INSERT OR IGNORE INTO frontiers(version_id, space, body) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            id.as_bytes().as_slice(),
            frontier.space.as_bytes().as_slice(),
            body.as_slice(),
        ],
    )?;
    Ok(id)
}

/// Resolve a VersionId to its canonical frontier (spec §5). Resolution
/// requires the space and the canonical frontier body; a missing object is
/// `VersionUnavailable` ([`Error::MissingVersion`] → `content_unavailable`),
/// never an approximate latest view (D1).
///
/// # Errors
/// Returns [`Error::MissingVersion`] for an unknown VersionId,
/// `invalid_encoding` when the stored head list is provably redundant, and
/// [`Error::Corrupt`] when the row does not hash to its key or names another
/// space.
pub fn resolve(conn: &Connection, space: &SpaceId, version: &VersionId) -> Result<Frontier, Error> {
    let row: Option<(Vec<u8>, Vec<u8>)> = conn
        .query_row(
            "SELECT space, body FROM frontiers WHERE version_id = ?1",
            [version.as_bytes().as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let Some((stored_space, body)) = row else {
        return Err(Error::MissingVersion);
    };
    let stored_space =
        SpaceId::from_bytes(&stored_space).map_err(|err| Error::Corrupt(err.to_string()))?;
    if &stored_space != space {
        return Err(Error::Corrupt(
            "frontier row names a different space".into(),
        ));
    }
    // The stored body is decoded, never repaired: a non-canonical head list
    // fails here, and the row key must be the hash of the body it stores.
    let frontier = Frontier::decode(&body).map_err(profile_err)?;
    let id = frontier.id().map_err(profile_err)?;
    if &id != version {
        return Err(Error::Corrupt(
            "frontier row key does not hash to its stored body".into(),
        ));
    }
    validate_dominance(conn, &frontier)?;
    // §5: a missing head is VersionUnavailable, never an approximate latest
    // view. Gate head visibility HERE, before any closure computation, so a
    // quarantined or unknown head surfaces as MissingVersion instead of a
    // silently smaller closure downstream.
    require_visible(conn, &frontier.heads)?;
    Ok(frontier)
}

/// Every head of a resolved frontier must be a visible publication; otherwise
/// the version is unavailable (MissingVersion → content_unavailable).
fn require_visible(conn: &Connection, heads: &[BatchId]) -> Result<(), Error> {
    for chunk in heads.chunks(projection::SEED_CHUNK) {
        if chunk.is_empty() {
            continue;
        }
        let values = projection::values_list(chunk.len());
        let sql = format!(
            "SELECT COUNT(*) FROM batches b
             WHERE b.quarantined = 0 AND b.batch_id IN (SELECT column1 FROM (VALUES {values}))"
        );
        let visible: i64 = conn.query_row(
            &sql,
            rusqlite::params_from_iter(projection::seed_params(chunk)),
            |row| row.get(0),
        )?;
        if visible != chunk.len() as i64 {
            return Err(Error::MissingVersion);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon;

    fn frontier_bytes(space: &SpaceId, heads: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        canon::write_array_header(&mut out, 3).unwrap();
        canon::write_uint(&mut out, profile::PROFILE_VERSION).unwrap();
        canon::write_bytes(&mut out, space.as_bytes()).unwrap();
        canon::write_array_header(&mut out, heads.len()).unwrap();
        for head in heads {
            canon::write_bytes(&mut out, head).unwrap();
        }
        out
    }

    #[test]
    fn frontier_decode_roundtrip() {
        let space = SpaceId::from_array([7; 16]);
        let heads = vec![[2u8; 32], [1u8; 32]];
        let mut sorted = heads.clone();
        sorted.sort_unstable();
        let frontier = Frontier {
            space,
            heads: sorted.iter().map(|h| BatchId::from_array(*h)).collect(),
        };
        let decoded = Frontier::decode(&frontier.encode().unwrap()).unwrap();
        assert_eq!(decoded, frontier);
        assert_eq!(decoded.id().unwrap(), frontier.id().unwrap());
    }

    #[test]
    fn frontier_decode_rejects_unnormalized_lists() {
        let space = SpaceId::from_array([7; 16]);
        // Unsorted heads.
        let unsorted = frontier_bytes(&space, &[&[2u8; 32], &[1u8; 32]]);
        assert!(Frontier::decode(&unsorted).is_err());
        // Duplicate heads.
        let dup = frontier_bytes(&space, &[&[1u8; 32], &[1u8; 32]]);
        assert!(Frontier::decode(&dup).is_err());
        // Trailing bytes and wrong widths are not canonical.
        let mut trailing = frontier_bytes(&space, &[&[1u8; 32]]);
        trailing.push(0x00);
        assert!(Frontier::decode(&trailing).is_err());
        let short = frontier_bytes(&space, &[&[1u8; 16]]);
        assert!(Frontier::decode(&short).is_err());
        // A different space's body is still well-formed but hashes differently.
        let other = Frontier::decode(&frontier_bytes(
            &SpaceId::from_array([8; 16]),
            &[&[1u8; 32]],
        ))
        .unwrap();
        assert_ne!(other.space, space);
    }

    #[test]
    fn version_id_is_hash_of_canonical_body() {
        let space = SpaceId::from_array([9; 16]);
        let heads = [BatchId::from_array([3u8; 32])];
        let bytes = frontier_bytes(&space, &[&[3u8; 32]]);
        assert_eq!(
            profile::version_id(&bytes),
            Frontier {
                space,
                heads: heads.to_vec()
            }
            .id()
            .unwrap()
        );
    }
}
