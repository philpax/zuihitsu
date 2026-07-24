//! The embedding batch: coalescing events into one operation per vector, embedding the texts, and
//! applying the resolved operations to the index.

use std::collections::BTreeMap;

use crate::{
    event::{Event, EventPayload},
    ids::{MemoryId, MemoryName, Seq},
    model::{
        embed::Embedder,
        index::{IndexError, VectorKey},
    },
    vector::{VectorError, VectorId, VectorIndex, VectorRecord},
};

/// Resolves a memory id to its current name, for the contextual embedding prefix. The live
/// indexer resolves names from the graph before calling [`embed_batch`], so the slow embed
/// holds no graph lock. `None` in tests or when the caller doesn't need contextual embeddings.
pub type NameResolver<'a> = &'a (dyn Fn(MemoryId) -> Option<MemoryName> + Sync);

/// Embed the content recorded in `events` into a [`Batch`] of pending index changes — **without
/// touching the vector index**. Coalesces to one operation per vector (last event wins), so a
/// description regenerated several times embeds once; entries are immutable, so each embeds once.
/// Async because it calls the embedder. The caller applies the result with [`apply_batch`] under the
/// index lock — separating the slow embedding from the brief index write is what lets a search proceed
/// without waiting behind a batch's embedding (spec §Concurrency, §Storage → vector store).
///
/// `name_resolver` resolves a memory id to its current name so the contextual embedding
/// (`"{handle}: {text}"`) can be produced alongside the raw-text embedding. Pass `None` to produce
/// only `Entry` vectors (backward-compatible with the pre-contextual behavior).
pub async fn embed_batch(
    embedder: &dyn Embedder,
    events: &[Event],
    name_resolver: Option<NameResolver<'_>>,
) -> Result<Batch, IndexError> {
    let mut ops: BTreeMap<VectorId, Pending> = BTreeMap::new();
    for event in events {
        match &event.payload {
            EventPayload::MemoryContentAppended {
                id, entry_id, text, ..
            } => {
                // The raw-text embedding serves search.
                ops.insert(
                    VectorKey::Entry(*entry_id).to_vector_id(),
                    Pending::Embed(text.clone()),
                );
                // The contextual embedding serves the dedup check and consolidation pass. The
                // handle prefix normalizes name-bearing and name-less entries so the same fact
                // scores similarly regardless of how it was phrased.
                if let Some(resolver) = name_resolver
                    && let Some(name) = resolver(*id)
                {
                    ops.insert(
                        VectorKey::EntryContextual(*entry_id).to_vector_id(),
                        Pending::Embed(crate::model::embed::contextual_text(name.as_str(), text)),
                    );
                }
            }
            EventPayload::MemoryDescriptionRegenerated { id, new_text, .. } => {
                ops.insert(
                    VectorKey::Description(*id).to_vector_id(),
                    Pending::Embed(new_text.clone()),
                );
            }
            EventPayload::MemoryDeleted { id } => {
                ops.insert(VectorKey::Description(*id).to_vector_id(), Pending::Remove);
            }
            // A superseded entry is no longer live, so both its raw and contextual vectors are
            // dropped from the index.
            EventPayload::MemorySuperseded { entry, .. } => {
                ops.insert(VectorKey::Entry(*entry).to_vector_id(), Pending::Remove);
                ops.insert(
                    VectorKey::EntryContextual(*entry).to_vector_id(),
                    Pending::Remove,
                );
            }
            // A retracted entry is tombstoned, so both vectors are dropped.
            EventPayload::EntryRetracted { entry, .. } => {
                ops.insert(VectorKey::Entry(*entry).to_vector_id(), Pending::Remove);
                ops.insert(
                    VectorKey::EntryContextual(*entry).to_vector_id(),
                    Pending::Remove,
                );
            }
            // Each consolidated source entry is tombstoned by the replacement; drop both vectors.
            EventPayload::EntriesConsolidated { sources, .. } => {
                for source in sources {
                    ops.insert(VectorKey::Entry(*source).to_vector_id(), Pending::Remove);
                    ops.insert(
                        VectorKey::EntryContextual(*source).to_vector_id(),
                        Pending::Remove,
                    );
                }
            }
            _ => {}
        }
    }

    // Skip blank texts: an empty (or whitespace-only) entry or description carries no semantic
    // content to embed, and a real embedding endpoint rejects an empty input outright ("the decoder
    // prompt cannot be empty"). Dropping it here leaves no vector — correct, since there is nothing
    // to retrieve — rather than failing the whole batch.
    let to_embed: Vec<(VectorId, String)> = ops
        .iter()
        .filter_map(|(key, op)| match op {
            Pending::Embed(text) if !text.trim().is_empty() => Some((key.clone(), text.clone())),
            Pending::Embed(_) | Pending::Remove => None,
        })
        .collect();

    let mut resolved = Vec::with_capacity(ops.len());
    if !to_embed.is_empty() {
        let texts: Vec<String> = to_embed.iter().map(|(_, text)| text.clone()).collect();
        let embeddings = embedder.embed(&texts).await?;
        let model_id = embedder.model_id();
        for ((id, _), embedding) in to_embed.into_iter().zip(embeddings) {
            resolved.push(ResolvedOp::Upsert(VectorRecord {
                id,
                embedding,
                model_id: model_id.into(),
            }));
        }
    }
    for (key, op) in &ops {
        if matches!(op, Pending::Remove) {
            resolved.push(ResolvedOp::Remove(key.clone()));
        }
    }

    Ok(Batch {
        ops: resolved,
        last_seq: events.last().map(|event| event.seq),
    })
}

/// Apply an embedded [`Batch`] to the vector index and advance its cursor — **synchronous and brief**,
/// so it can run under the index lock without blocking a concurrent search for long. The cursor is
/// advanced last, after the vectors are written, so a crash re-processes the batch rather than skipping
/// it (an idempotent re-embed).
pub fn apply_batch(vectors: &mut dyn VectorIndex, batch: Batch) -> Result<(), VectorError> {
    for op in batch.ops {
        match op {
            ResolvedOp::Upsert(record) => vectors.upsert(record)?,
            ResolvedOp::Remove(id) => vectors.remove(&id)?,
        }
    }
    if let Some(seq) = batch.last_seq {
        vectors.set_cursor(seq)?;
    }
    Ok(())
}

/// A batch of index changes with their embeddings already computed (by [`embed_batch`]) — ready for a
/// brief, lock-held [`apply_batch`]. Carries the highest `Seq` it covers, so applying it advances the
/// index cursor.
pub struct Batch {
    pub ops: Vec<ResolvedOp>,
    pub last_seq: Option<Seq>,
}

/// One vector's change before embedding: (re)embed to this text, or drop it.
enum Pending {
    Embed(String),
    Remove,
}

/// One vector's change after embedding: the record to write, or the id to drop.
pub enum ResolvedOp {
    Upsert(VectorRecord),
    Remove(VectorId),
}
