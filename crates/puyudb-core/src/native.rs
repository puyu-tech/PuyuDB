// SPDX-License-Identifier: Apache-2.0

//! Native text admission and reads (I05 P6–P8).
//!
//! Normative source: `docs/spec/canonical-profile-v1.md` §6.3 (native
//! artifact), §12 (dt-text-v1 profile, scope rules §12.5, seed §12.4) and
//! `PUYUDB-BLUEPRINT.md` §5.4–§5.5. The `native_changes` table maps
//! `(epoch, artifact)` to its producer: `NULL` for the epoch-owned seed, a
//! unique batch id otherwise (§6.3). All native bytes go through the
//! `puyudb-crdt` binding; this module never touches a native type directly.
//!
//! Admission checks per §12.5: the artifact's epoch must be a declared text
//! epoch at the batch's parent cut, every submitted op must belong to the
//! enclosing writer's derived actor name, op sequences must continue that
//! actor's known frontier without gaps, every native parent must lie within
//! the batch's parent cut, and root-object creation is only allowed in the
//! verified seed. Violations are `invalid_native_scope`.

use std::collections::HashSet;

use rusqlite::Connection;

use puyudb_crdt::dt_text::{Error as CrdtError, TextDoc};

use crate::canon::{self, Item};
use crate::identity::{ArtifactId, BatchId, EpochId};
use crate::profile::{self, BatchBody, EpochKind, NativeArtifact, Record};
use crate::projection::{self, Field};
use crate::publication::Error;

/// Native tables added on top of the I04/I05-phase-1 schema.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS native_changes (
  epoch BLOB NOT NULL,
  artifact_id BLOB NOT NULL,
  producer BLOB,
  PRIMARY KEY(epoch, artifact_id),
  FOREIGN KEY(artifact_id) REFERENCES artifacts(id),
  FOREIGN KEY(producer) REFERENCES batches(batch_id)
);
CREATE INDEX IF NOT EXISTS native_changes_epoch ON native_changes(epoch);
";

fn native_scope(detail: impl Into<String>) -> Error {
    Error::InvalidNativeScope(detail.into())
}

fn crdt_error(error: CrdtError) -> Error {
    match error.code() {
        "invalid_native_scope" => native_scope(error.to_string()),
        _ => Error::Rejected(error.to_string()),
    }
}

/// The verified, resident raw payload of one artifact, re-hash-checked.
pub(crate) fn artifact_payload(conn: &Connection, id: &ArtifactId) -> Result<Vec<u8>, Error> {
    let manifest_blob: Vec<u8> = conn
        .query_row(
            "SELECT manifest FROM artifacts WHERE id = ?1",
            [id.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .map_err(|_| Error::MissingDependency)?;
    let item = canon::decode(&manifest_blob).map_err(|err| Error::Rejected(err.to_string()))?;
    let manifest =
        profile::ArtifactManifest::decode(&item).map_err(|err| Error::Rejected(err.to_string()))?;
    let mut statement = conn.prepare(
        "SELECT ac.chunk_id, c.bytes FROM artifact_chunks ac JOIN chunks c ON c.id = ac.chunk_id
         WHERE ac.artifact_id = ?1 ORDER BY ac.ordinal",
    )?;
    let rows = statement.query_map([id.as_bytes().as_slice()], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut payload: Vec<u8> = Vec::new();
    let mut count = 0usize;
    for row in rows {
        let (chunk_blob, bytes) = row?;
        let chunk_id = crate::identity::ChunkId::from_bytes(&chunk_blob)
            .map_err(|err| Error::Rejected(err.to_string()))?;
        if profile::chunk_id(&bytes) != chunk_id {
            return Err(Error::Rejected(
                "stored chunk does not hash to its id".into(),
            ));
        }
        if count >= manifest.chunks.len() || manifest.chunks[count] != chunk_id {
            return Err(Error::Rejected(
                "artifact chunk list does not match the manifest".into(),
            ));
        }
        if bytes.len() > profile::CHUNK_RAW_BYTES && count + 1 != manifest.chunks.len() {
            return Err(Error::Rejected(
                "non-final chunk exceeds the raw cap".into(),
            ));
        }
        payload.extend_from_slice(&bytes);
        count += 1;
    }
    if count != manifest.chunks.len()
        || payload.len() as u64 != manifest.raw_length
        || profile::raw_digest(&manifest.kind, &payload) != manifest.raw_digest
    {
        return Err(Error::Rejected(
            "artifact payload does not match its manifest".into(),
        ));
    }
    Ok(payload)
}

/// Decode a validated native artifact payload item.
fn decode_native(item: &Item) -> Result<NativeArtifact, Error> {
    profile::validate_record(
        Record::NativeArtifact,
        &canon::encode(item).map_err(|err| Error::Rejected(err.to_string()))?,
    )
    .map_err(|err| Error::Rejected(err.to_string()))?;
    let record = match item {
        Item::Array(items) if items.len() == 4 => items,
        _ => {
            return Err(Error::Rejected(
                "native artifact has the wrong width".into(),
            ));
        }
    };
    let epoch_bytes = match &record[1] {
        Item::Bytes(bytes) if bytes.len() == 32 => bytes,
        _ => return Err(Error::Rejected("native epoch is not bytes32".into())),
    };
    let profile_text = match &record[2] {
        Item::Text(text) => text.clone(),
        _ => return Err(Error::Rejected("native profile is not text".into())),
    };
    let native = match &record[3] {
        Item::Bytes(bytes) => bytes.clone(),
        _ => return Err(Error::Rejected("native bytes is not a byte string".into())),
    };
    Ok(NativeArtifact {
        epoch: EpochId::from_bytes(epoch_bytes).map_err(|err| Error::Rejected(err.to_string()))?,
        profile: profile_text,
        native,
    })
}

/// The visible closure of `seeds` as a set (seeds included).
pub(crate) fn closure_of(conn: &Connection, seeds: &[BatchId]) -> Result<HashSet<BatchId>, Error> {
    let map = projection::ancestor_map_pub(conn, seeds)?;
    let mut out: HashSet<BatchId> = seeds.iter().copied().collect();
    for set in map.values() {
        out.extend(set.iter().copied());
    }
    Ok(out)
}

/// The declared text-epoch descriptor for `epoch` in the parent closure, if any.
fn text_epoch_at(
    conn: &Connection,
    parents: &[BatchId],
    epoch: &EpochId,
) -> Result<Option<(EpochId, profile::EpochDescriptor)>, Error> {
    let writes = projection::load_writes_pub(conn, parents)?;
    for write in writes {
        if write.field == Field::Epoch.tag() {
            let descriptor = projection::decode_epoch_value_pub(&write.value)?;
            let id = descriptor
                .id()
                .map_err(|err| Error::Rejected(err.to_string()))?;
            if &id == epoch {
                if descriptor.kind != EpochKind::Text {
                    return Ok(None);
                }
                return Ok(Some((id, descriptor)));
            }
        }
    }
    Ok(None)
}

/// Build the native document for `epoch` at a closure: seed first, then every
/// admitted change in the closure applied in causal (frontier) order. The
/// result is arrival-order independent because DT converges.
fn build_log(
    conn: &Connection,
    epoch: &EpochId,
    closure: &HashSet<BatchId>,
) -> Result<TextDoc, Error> {
    let mut statement =
        conn.prepare("SELECT artifact_id, producer FROM native_changes WHERE epoch = ?1")?;
    let rows = statement.query_map([epoch.as_bytes().as_slice()], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
    })?;
    let mut seed: Option<Vec<u8>> = None;
    let mut pending: Vec<Vec<u8>> = Vec::new();
    for row in rows {
        let (artifact, producer) = row?;
        let artifact =
            ArtifactId::from_bytes(&artifact).map_err(|err| Error::Rejected(err.to_string()))?;
        let keep = match &producer {
            None => true,
            Some(batch) => closure.contains(
                &BatchId::from_bytes(batch).map_err(|err| Error::Rejected(err.to_string()))?,
            ),
        };
        if !keep {
            continue;
        }
        let payload = artifact_payload(conn, &artifact)?;
        // Unwrap the canonical NativeArtifact envelope; the DT bytes are the
        // `native` field.
        profile::validate_record(Record::NativeArtifact, &payload)
            .map_err(|err| Error::Rejected(err.to_string()))?;
        let item = canon::decode(&payload).map_err(|err| Error::Rejected(err.to_string()))?;
        let native = decode_native(&item)?;
        match producer {
            None => seed = Some(native.native),
            Some(_) => pending.push(native.native),
        }
    }
    let seed = seed.ok_or(Error::MissingDependency)?;
    let mut base = TextDoc::load(&seed).map_err(crdt_error)?;
    // Kahn-style: apply any change whose ops are all new; repeat until done.
    while !pending.is_empty() {
        let mut progress = false;
        let mut remaining = Vec::with_capacity(pending.len());
        for payload in pending.drain(..) {
            match base.strict_append(&payload) {
                Ok((staged, _)) => {
                    base = staged;
                    progress = true;
                }
                Err(CrdtError::KnownOperation) => remaining.push(payload),
                Err(other) => return Err(crdt_error(other)),
            }
        }
        pending = remaining;
        if !progress {
            return Err(Error::Rejected(
                "native change set does not converge within the cut".into(),
            ));
        }
    }
    Ok(base)
}

/// Validate every native artifact of a batch against the parent cut.
///
/// Returns the `(epoch, artifact)` pairs to record after the batch row exists.
/// Runs inside the publication transaction, after the parents are admitted.
pub(crate) fn check_native(
    conn: &Connection,
    body: &BatchBody,
    native_payloads: &[(ArtifactId, Vec<u8>)],
) -> Result<Vec<(EpochId, ArtifactId)>, Error> {
    if native_payloads.is_empty() {
        return Ok(Vec::new());
    }
    let closure = closure_of(conn, &body.parents)?;
    let mut seen_epochs: HashSet<EpochId> = HashSet::new();
    let mut out = Vec::with_capacity(native_payloads.len());
    for (artifact_id, payload) in native_payloads {
        if payload.len() as u64 > profile::MAX_ARTIFACT_RAW_BYTES {
            return Err(Error::Rejected(
                "native payload exceeds the profile cap".into(),
            ));
        }
        profile::validate_record(Record::NativeArtifact, payload)
            .map_err(|err| Error::Rejected(err.to_string()))?;
        let item = canon::decode(payload).map_err(|err| Error::Rejected(err.to_string()))?;
        let native = decode_native(&item)?;
        let epoch = native.epoch;
        if native.profile != profile::TEXT_PROFILE {
            return Err(native_scope("native profile is not dt-text-v1"));
        }
        // One native artifact per (epoch, batch): all of the batch's edits to
        // one epoch are grouped in it (§6.3).
        if !seen_epochs.insert(epoch) {
            return Err(native_scope(
                "batch carries more than one native artifact for one epoch",
            ));
        }
        // The epoch must be a declared text epoch at the parent cut.
        if text_epoch_at(conn, &body.parents, &epoch)?.is_none() {
            return Err(native_scope(
                "native artifact names an epoch not declared as text in the parent cut",
            ));
        }
        // The epoch-owned seed must already be registered.
        let seed: Option<Vec<u8>> = conn
            .query_row(
                "SELECT artifact_id FROM native_changes WHERE epoch = ?1 AND producer IS NULL",
                [epoch.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .ok();
        if seed.is_none() {
            return Err(Error::MissingDependency);
        }
        let base = build_log(conn, &epoch, &closure)?;
        let base_len = base.op_len();
        let (staged, report) = base
            .strict_append(&native.native)
            .map_err(|err| match err {
                CrdtError::KnownOperation => native_scope(
                    "native artifact contains operations already known in the parent cut",
                ),
                CrdtError::BaseVersionUnknown => {
                    native_scope("native base version falls outside the batch parent cut")
                }
                other => crdt_error(other),
            })?;
        if report.submitted.0 == report.submitted.1 {
            return Err(native_scope("native artifact carries no new operations"));
        }
        // Every declared agent name must be the seed or this writer's actor.
        let seed_name = profile::seed_actor_name(&epoch);
        let writer_name =
            profile::writer_actor_name(&body.public_key, &body.branch, &body.writer_epoch, &epoch);
        for name in &report.agent_names {
            if name != &seed_name && name != &writer_name {
                return Err(native_scope(
                    "native artifact declares an agent outside the seed/writer namespaces",
                ));
            }
        }
        // Every submitted op belongs to the writer actor, with contiguous
        // sequence continuation from the base frontier (§12.5).
        let spans = staged.submitted_spans(report.submitted);
        let mut writer_spans: Vec<&puyudb_crdt::dt_text::SubmittedSpan> = spans
            .iter()
            .filter(|span| span.agent == writer_name)
            .collect();
        if writer_spans.len() != spans.len() {
            return Err(native_scope(
                "native artifact carries operations of an actor other than the enclosing writer",
            ));
        }
        writer_spans.sort_by_key(|span| span.seq_start);
        let mut expected = base
            .remote_frontier()
            .into_iter()
            .find(|(name, _)| name == &writer_name)
            .map(|(_, next)| next)
            .unwrap_or(0);
        for span in &writer_spans {
            if span.seq_start != expected {
                return Err(native_scope(
                    "native actor sequence is not continuous with the parent cut",
                ));
            }
            expected = span.seq_end;
        }
        // Op kinds are insert/delete only by type; assert the range is real.
        if staged.op_kinds(report.submitted).is_empty() {
            return Err(native_scope("native artifact carries no operations"));
        }
        // Every native parent must lie within the batch's parent cut (the
        // base log), or chain inside this artifact's own submitted span.
        for entry in staged.submitted_entries(report.submitted) {
            for parent in &entry.parents {
                let in_base = *parent < base_len && base.version_contains(*parent);
                let in_artifact = *parent >= report.submitted.0 && *parent < report.submitted.1;
                if !in_base && !in_artifact {
                    return Err(native_scope(
                        "native parent falls outside the batch parent cut",
                    ));
                }
            }
        }
        for lv in &report.start_frontier_lvs {
            if *lv >= base_len || !base.version_contains(*lv) {
                return Err(native_scope(
                    "native start frontier falls outside the batch parent cut",
                ));
            }
        }
        out.push((epoch, *artifact_id));
    }
    Ok(out)
}

/// Record validated native changes after the batch row exists (same commit).
pub(crate) fn record_native(
    conn: &Connection,
    batch: &BatchId,
    pairs: &[(EpochId, ArtifactId)],
) -> Result<(), Error> {
    for (epoch, artifact) in pairs {
        conn.execute(
            "INSERT INTO native_changes(epoch, artifact_id, producer) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                epoch.as_bytes().as_slice(),
                artifact.as_bytes().as_slice(),
                batch.as_bytes().as_slice(),
            ],
        )
        .map_err(|_| {
            native_scope("native artifact already has a different producer for this epoch")
        })?;
    }
    Ok(())
}

/// Register the epoch-owned seed artifact for a text epoch (idempotent by
/// `(epoch, artifact)`). Validates the seed per §12.4: empty baseline ⇒
/// zero-length native payload; otherwise the seed's only agent is the seed
/// actor and its text equals the baseline content.
pub(crate) fn register_seed(
    conn: &Connection,
    epoch: &EpochId,
    manifest: &profile::ArtifactManifest,
    chunks: &[crate::identity::ChunkId],
) -> Result<(), Error> {
    if manifest.kind != "native-change-v1" {
        return Err(Error::Rejected(
            "seed artifact kind must be native-change-v1".into(),
        ));
    }
    let encoded = manifest
        .encode()
        .map_err(|err| Error::Rejected(err.to_string()))?;
    profile::validate_record(Record::ArtifactManifest, &encoded)
        .map_err(|err| Error::Rejected(err.to_string()))?;
    let id = manifest
        .id()
        .map_err(|err| Error::Rejected(err.to_string()))?;
    if manifest.chunks.len() != chunks.len()
        || manifest
            .chunks
            .iter()
            .zip(chunks)
            .any(|(expected, given)| expected != given)
    {
        return Err(Error::Rejected(
            "chunk list does not match the artifact manifest".into(),
        ));
    }
    // Hydration: every chunk staged, hashing to its id, streaming to the raw
    // digest and length.
    let mut payload: Vec<u8> = Vec::new();
    for chunk in chunks {
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT bytes FROM chunks WHERE id = ?1",
                [chunk.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .map_err(|_| Error::MissingChunk)?;
        if profile::chunk_id(&bytes) != *chunk {
            return Err(Error::Rejected(
                "stored chunk does not hash to its id".into(),
            ));
        }
        payload.extend_from_slice(&bytes);
    }
    if payload.len() as u64 != manifest.raw_length
        || profile::raw_digest(&manifest.kind, &payload) != manifest.raw_digest
    {
        return Err(Error::Rejected(
            "artifact payload does not match its manifest".into(),
        ));
    }
    profile::validate_record(Record::NativeArtifact, &payload)
        .map_err(|err| Error::Rejected(err.to_string()))?;
    let item = canon::decode(&payload).map_err(|err| Error::Rejected(err.to_string()))?;
    let native = decode_native(&item)?;
    if &native.epoch != epoch || native.profile != profile::TEXT_PROFILE {
        return Err(native_scope(
            "seed artifact does not bind the epoch and the dt-text-v1 profile",
        ));
    }
    // The epoch must be declared as a text epoch somewhere in visible history.
    let descriptor = find_text_epoch_anywhere(conn, epoch)?
        .ok_or_else(|| native_scope("epoch is not declared as a text epoch"))?;
    let baseline = artifact_payload(conn, &descriptor.baseline)?;
    if baseline.is_empty() {
        if !native.native.is_empty() {
            return Err(native_scope(
                "an empty baseline requires a zero-length seed payload",
            ));
        }
    } else {
        let doc = TextDoc::load(&native.native).map_err(crdt_error)?;
        let seed_name = profile::seed_actor_name(epoch);
        let spans = doc.submitted_spans((0, doc.op_len()));
        if spans.is_empty() || !spans.iter().all(|span| span.agent == seed_name) {
            return Err(native_scope(
                "seed may only contain operations of the seed actor",
            ));
        }
        if doc.text().as_bytes() != baseline.as_slice() {
            return Err(Error::Rejected(
                "seed text does not match the epoch baseline (hash_mismatch)".into(),
            ));
        }
    }
    // Exactly one seed per epoch.
    let existing: Option<Vec<u8>> = conn
        .query_row(
            "SELECT artifact_id FROM native_changes WHERE epoch = ?1 AND producer IS NULL",
            [epoch.as_bytes().as_slice()],
            |row| row.get(0),
        )
        .ok();
    if let Some(existing) = existing {
        if existing.as_slice() == id.as_bytes().as_slice() {
            return Ok(());
        }
        return Err(Error::Rejected(
            "epoch already has a different seed artifact".into(),
        ));
    }
    conn.execute(
        "INSERT OR IGNORE INTO artifacts(id, kind, raw_length, raw_digest, manifest)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            id.as_bytes().as_slice(),
            manifest.kind.as_str(),
            manifest.raw_length as i64,
            manifest.raw_digest.as_slice(),
            encoded,
        ],
    )?;
    for (ordinal, chunk) in manifest.chunks.iter().enumerate() {
        conn.execute(
            "INSERT OR IGNORE INTO artifact_chunks(artifact_id, ordinal, chunk_id) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                id.as_bytes().as_slice(),
                ordinal as i64,
                chunk.as_bytes().as_slice(),
            ],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO content_availability(artifact_id, state) VALUES (?1, 'resident')",
        rusqlite::params![id.as_bytes().as_slice()],
    )?;
    conn.execute(
        "INSERT INTO native_changes(epoch, artifact_id, producer) VALUES (?1, ?2, NULL)",
        rusqlite::params![epoch.as_bytes().as_slice(), id.as_bytes().as_slice()],
    )?;
    Ok(())
}

fn find_text_epoch_anywhere(
    conn: &Connection,
    epoch: &EpochId,
) -> Result<Option<profile::EpochDescriptor>, Error> {
    let mut statement = conn.prepare(
        "SELECT fw.value FROM field_writes fw
         JOIN batches b ON b.batch_id = fw.batch_id AND b.quarantined = 0
         WHERE fw.field = ?1",
    )?;
    let rows = statement.query_map([Field::Epoch.tag() as i64], |row| row.get::<_, Vec<u8>>(0))?;
    for row in rows {
        let value = row?;
        let descriptor = projection::decode_epoch_value_pub(&value)?;
        let id = descriptor
            .id()
            .map_err(|err| Error::Rejected(err.to_string()))?;
        if &id == epoch {
            return Ok(Some(descriptor));
        }
    }
    Ok(None)
}

/// Current text of `epoch` at a cut (P8): seed plus all native changes in the
/// cut's closure. A historical read at a version is the same call with the
/// version's heads as the cut.
pub fn text_at(conn: &Connection, cut: &[BatchId], epoch: &EpochId) -> Result<String, Error> {
    projection::check_cut_pub(conn, cut)?;
    let closure = closure_of(conn, cut)?;
    let doc = build_log(conn, epoch, &closure)?;
    Ok(doc.text())
}

/// Historical text at a local native version is a cut-bounded read; the
/// public position unit is the UTF-8 byte offset (§5.5), converted with the
/// binding's boundary-checked helpers.
pub fn text_at_version(
    conn: &Connection,
    cut: &[BatchId],
    epoch: &EpochId,
    version: &[usize],
) -> Result<String, Error> {
    projection::check_cut_pub(conn, cut)?;
    let closure = closure_of(conn, cut)?;
    let doc = build_log(conn, epoch, &closure)?;
    Ok(doc.text_at(version))
}
