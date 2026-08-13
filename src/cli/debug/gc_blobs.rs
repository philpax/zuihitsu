//! The `gc-blobs` command: delete stored blobs the event log no longer refers to.
//!
//! Blobs are as permanent as the log, because the log is append-only: an attachment recorded on a
//! turn is referred to forever. A blob becomes garbage only two ways — a `revert` truncates the tail
//! that named it, or an upload was never followed by the message that would have carried it. Both are
//! operator-scale events, so reclaiming the space is an offline operator action rather than a
//! background sweep.
//!
//! The mark scan is deliberately over-inclusive: it walks each payload's JSON for any string shaped
//! like a content address, rather than knowing which payload fields hold one. A payload that gains a
//! blob reference later is therefore covered without this command being taught about it, and the
//! worst an unrelated 64-hex string (a `request_digest`, say) can do is retain a blob that could have
//! gone. Over-retention wastes disk; over-deletion loses data the log still points at.

use zuihitsu::{
    config::EnvConfig,
    ids::{BlobHash, Seq},
    store::{BlobStore, SqliteStore, Store},
};

use std::collections::BTreeSet;

use crate::cli::error::CliError;

/// Delete every stored blob the log no longer refers to. Without `--yes`, it reports what it would
/// collect and changes nothing.
///
/// The log is opened read-write purely for its single-writer lock: the command only reads it, but a
/// running agent may upload a blob moments before recording the message that names it, and sweeping
/// through that window would delete a live attachment. Failing to take the lock is the guard.
pub(crate) fn gc_blobs(config: &EnvConfig, yes: bool) -> Result<(), CliError> {
    let log_path = config.storage.event_log();
    let store = SqliteStore::open(&log_path).map_err(|source| {
        CliError::GcBlobs(format!(
            "could not open the event log at {} (is the agent running?): {source}",
            log_path.display()
        ))
    })?;

    let blobs_path = config.storage.blobs();
    if !blobs_path.exists() {
        tracing::info!(
            "no blob store at {} — nothing to collect",
            blobs_path.display()
        );
        return Ok(());
    }
    let blobs = BlobStore::open(&blobs_path).map_err(|source| {
        CliError::GcBlobs(format!(
            "could not open the blob store at {}: {source}",
            blobs_path.display()
        ))
    })?;

    let referenced = referenced_hashes(&store)?;
    let stored = blobs.hashes().map_err(|source| {
        CliError::GcBlobs(format!("could not list the stored blobs: {source}"))
    })?;
    let unreferenced: Vec<BlobHash> = stored
        .iter()
        .filter(|hash| !referenced.contains(*hash))
        .cloned()
        .collect();

    if unreferenced.is_empty() {
        tracing::info!(
            "{} blob(s) stored, all still referenced by the log — nothing to collect",
            stored.len()
        );
        return Ok(());
    }

    let reclaimed: u64 = unreferenced
        .iter()
        .filter_map(|hash| blobs.head(hash).ok().flatten())
        .map(|meta| meta.byte_len)
        .sum();

    if !yes {
        for hash in &unreferenced {
            tracing::info!("would collect {hash}");
        }
        tracing::warn!(
            "re-run with --yes to delete {} unreferenced blob(s) ({reclaimed} bytes) of {} stored",
            unreferenced.len(),
            stored.len()
        );
        return Ok(());
    }

    for hash in &unreferenced {
        blobs.remove(hash).map_err(|source| {
            CliError::GcBlobs(format!("could not remove blob {hash}: {source}"))
        })?;
        tracing::info!("collected {hash}");
    }
    tracing::info!(
        "collected {} unreferenced blob(s) ({reclaimed} bytes); {} still referenced",
        unreferenced.len(),
        stored.len() - unreferenced.len()
    );
    Ok(())
}

/// Every content address the log still mentions, from any payload field.
fn referenced_hashes(store: &dyn Store) -> Result<BTreeSet<BlobHash>, CliError> {
    let events = store
        .read_from(Seq(0))
        .map_err(|source| CliError::GcBlobs(format!("could not read the event log: {source}")))?;
    let mut referenced = BTreeSet::new();
    for event in events {
        let payload = serde_json::to_value(&event.payload).map_err(|source| {
            CliError::GcBlobs(format!(
                "could not re-serialise the payload at seq {}: {source}",
                event.seq.0
            ))
        })?;
        collect_hashes(&payload, &mut referenced);
    }
    Ok(referenced)
}

/// Walk `value` for strings that parse as a content address, adding each to `found`.
fn collect_hashes(value: &serde_json::Value, found: &mut BTreeSet<BlobHash>) {
    match value {
        serde_json::Value::String(text) => {
            if let Ok(hash) = text.parse::<BlobHash>() {
                found.insert(hash);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_hashes(item, found);
            }
        }
        serde_json::Value::Object(fields) => {
            for field in fields.values() {
                collect_hashes(field, found);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::collect_hashes;
    use std::collections::BTreeSet;
    use zuihitsu::ids::BlobHash;

    #[test]
    fn a_hash_is_found_however_deeply_it_is_nested() {
        let hash = BlobHash::of(b"the bytes");
        let payload = serde_json::json!({
            "type": "ConversationTurn",
            "text": "have a look",
            "attachments": [{ "name": "shot.png", "blob": hash.as_str() }],
        });
        let mut found = BTreeSet::new();
        collect_hashes(&payload, &mut found);
        assert_eq!(found, BTreeSet::from([hash]));
    }

    #[test]
    fn a_payload_naming_no_address_marks_nothing() {
        let payload = serde_json::json!({ "text": "not a hash", "seq": 12, "ok": true });
        let mut found = BTreeSet::new();
        collect_hashes(&payload, &mut found);
        assert!(found.is_empty());
    }

    #[test]
    fn an_unrelated_digest_is_marked_rather_than_risking_a_live_blob() {
        // A `request_digest` is a 64-hex string too. Marking it retains nothing (no blob has that
        // address) and costs nothing, where teaching the scan to skip it would risk the opposite
        // mistake on a payload field this command has not been told about.
        let digest = "a".repeat(64);
        let payload = serde_json::json!({ "type": "ModelCalled", "request_digest": digest });
        let mut found = BTreeSet::new();
        collect_hashes(&payload, &mut found);
        assert_eq!(found.len(), 1);
    }
}
