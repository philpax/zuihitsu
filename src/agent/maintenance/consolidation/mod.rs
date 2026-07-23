//! The consolidation pass: clusters semantically-overlapping live entries and folds redundant ones
//! away, running off the hot path under [`Authority::Agent`] through the ordinary
//! [`MemoryBlock`](crate::memory::memory_block::MemoryBlock) write path — so every write clears the
//! same guards a turn does, rather than appending raw events that bypass them.
//!
//! Two tiers run per identity class with content changes since the cursor, each committed as its own
//! block:
//!
//! **Tier 1 — within-level synthesis.** Live entries are grouped by visibility posture (see
//! [`clustering::tier1_groups`]): public and attributed entries each merge across tellers (both surface
//! to everyone, so synthesizing two relayed accounts leaks nothing), while private and exclude entries
//! group per teller (and per exact exclude set), since below the all-audience tier the teller is the
//! audience-bearing payload, not mere provenance. Within a group, near-duplicates are clustered at
//! `consolidation_similarity_threshold` and the model synthesizes one richer replacement. The
//! replacement inherits the group's visibility verbatim, and its teller is the group's teller when
//! uniform or [`Teller::Agent`] for a cross-teller merge (permitted at the public or attributed level).
//! Each distinct source teller survives as an [`EntryAttested`](crate::event::EventPayload::EntryAttested) on the replacement, so a
//! cross-teller merge preserves who the accounts came from rather than collapsing them into the agent.
//! Because synthesis never crosses a level, a private confidence's text is never merged into a copy
//! visible to a wider audience.
//!
//! **Tier 2 — cross-level dedup, never synthesis.** After tier 1 commits, a narrower (or equally-wide
//! but attributed) live entry whose fact is already attested by a wider entry — at the stricter
//! `dedup_similarity_threshold`, not the looser consolidation bar — is retired into that entry via
//! `EntriesConsolidated`, the existing wider entry as the replacement. No new entry is written and the
//! retired text enters no prompt; instead the write path leaves an [`EntryAttested`](crate::event::EventPayload::EntryAttested) on
//! the replacement carrying the retired source's teller, posture, and exact phrasing, so the fact is
//! absorbed rather than merely dropped. A fact already attested at least as widely is redundant in its
//! narrower copy, and the stricter bar is where "same fact" is credible enough to act on. This is the
//! one place consolidation crosses tellers and postures, exactly the case the foreign-confidence
//! supersede guard exists for; the write clears that guard under [`Authority::Agent`] precisely because
//! the replacement's audience is a verified superset of the retired entry's.
//!
//! Source entries are tombstoned (stamped `superseded_by` = the replacement), dropping them from live
//! surfaces while preserving them in history, and each `EntriesConsolidated` carries the full
//! many-to-one relationship.

use std::sync::Arc;

use crate::{
    InstanceError,
    agent::templates,
    engine::Engine,
    event::{EventSource, ProducedBy, PromptTemplateName, Teller},
    graph::EntryView,
    ids::{EntryId, MemoryId, Seq, TurnId},
    memory::memory_block::{Authority, MemoryBlock},
    model::ModelClient,
    settings::{CaptureLevel, Settings},
};

use crate::agent::{
    maintenance::dedupe_by_class,
    turn::{Recording, collect_written_memories},
};

mod clustering;
mod synthesis;

#[cfg(test)]
mod tests;

use clustering::{cluster_within, embed_class_entries, tier1_groups, tier2_absorptions};
use synthesis::synthesize_cluster;

/// The maximum number of entries per class the pass considers. A safety valve, not a tuning
/// parameter — clustering is O(n²) but trivially fast for n ≤ 100.
const MAX_ENTRIES_PER_CLASS: usize = 100;

/// Run one consolidation sweep. Returns `(new_cursor, memories_considered)`.
pub async fn catch_up(
    engine: &Arc<Engine>,
    model: &dyn ModelClient,
    cursor: Seq,
) -> Result<(Seq, usize), InstanceError> {
    let head = engine.store.lock().head()?;
    if head <= cursor {
        return Ok((cursor, 0));
    }

    let Some(template) = templates::latest_template(
        engine.store.lock().as_ref(),
        PromptTemplateName::EntryConsolidation,
    )?
    else {
        return Ok((head, 0));
    };

    let written = collect_written_memories(engine.store.lock().as_ref(), cursor)?;
    let written = dedupe_by_class(engine, written)?;
    if written.is_empty() {
        return Ok((head, 0));
    }

    // Without retrieval there is nothing to embed, cluster, or dedup — the sweep advances its cursor.
    if engine.retrieval.is_none() {
        return Ok((head, written.len()));
    }

    let recording = Recording::new(None, TurnId::generate(), CaptureLevel::Off);
    let settings = Settings::from_store(engine.store.lock().as_ref()).unwrap_or_default();
    let produced_by = ProducedBy {
        model_id: model.model_id().into(),
        template_name: PromptTemplateName::EntryConsolidation,
        template_version: template.version,
    };
    let sweep = Sweep {
        engine,
        model,
        recording: &recording,
        template_body: &template.body,
        produced_by: &produced_by,
        consolidation_threshold: settings.maintenance.consolidation_similarity_threshold,
        dedup_threshold: settings.maintenance.dedup_similarity_threshold,
        max_entry_chars: settings.memory.max_entry_chars.max(1) as usize,
    };

    sweep.tier1(&written).await?;
    sweep.tier2(&written).await?;

    Ok((head, written.len()))
}

/// The shared inputs one sweep threads through both tiers: the engine and model seam, the recording
/// sink and synthesis template, the consolidation model's provenance, the two similarity thresholds,
/// and the entry-length limit a synthesized replacement must fit.
struct Sweep<'a> {
    engine: &'a Arc<Engine>,
    model: &'a dyn ModelClient,
    recording: &'a Recording,
    template_body: &'a str,
    produced_by: &'a ProducedBy,
    consolidation_threshold: f64,
    dedup_threshold: f64,
    max_entry_chars: usize,
}

impl Sweep<'_> {
    /// Tier 1: synthesize a richer replacement for each within-level cluster, committed as one block.
    async fn tier1(&self, written: &[MemoryId]) -> Result<(), InstanceError> {
        let mut block = self.new_block()?;
        for &id in written {
            let entries: Vec<EntryView> = {
                let graph = self.engine.graph.lock();
                graph.class_entries(id)?
            };
            if entries.len() < 2 || entries.len() > MAX_ENTRIES_PER_CLASS {
                continue;
            }
            let embeddings = embed_class_entries(self.engine, id, &entries).await?;
            if embeddings.len() != entries.len() {
                continue;
            }
            let existing_links = {
                let graph = self.engine.graph.lock();
                graph.class_links(id)?
            };

            for group in tier1_groups(&entries) {
                if group.len() < 2 {
                    continue;
                }
                for cluster in cluster_within(&embeddings, &group, self.consolidation_threshold) {
                    if cluster.len() < 2 {
                        continue;
                    }
                    let cluster_entries: Vec<EntryView> =
                        cluster.iter().map(|&i| entries[i].clone()).collect();
                    self.synthesize_and_consolidate(
                        &mut block,
                        id,
                        &cluster_entries,
                        &existing_links,
                    )
                    .await?;
                }
            }
        }
        self.commit(block)
    }

    /// Synthesize one cluster and buffer its consolidation, logging (rather than failing the sweep) a
    /// model decline, a synthesis error, or a rejected write.
    async fn synthesize_and_consolidate(
        &self,
        block: &mut MemoryBlock,
        id: MemoryId,
        cluster: &[EntryView],
        existing_links: &[crate::graph::ClassLinkView],
    ) -> Result<(), InstanceError> {
        match synthesize_cluster(
            self.engine,
            self.model,
            self.recording,
            self.template_body,
            id,
            cluster,
            existing_links,
        )
        .await
        {
            Ok(Some(text)) => {
                let sources: Vec<EntryId> = cluster.iter().map(|entry| entry.entry_id).collect();
                if let Err(error) =
                    block.consolidate(id, &sources, text, Some(self.produced_by.clone()))
                {
                    tracing::warn!(
                        memory = ?id,
                        %error,
                        "consolidation: tier 1 write rejected; skipping cluster"
                    );
                }
            }
            Ok(None) => tracing::debug!(
                memory = ?id,
                cluster_size = cluster.len(),
                "consolidation: model returned no synthesis for a cluster; skipping"
            ),
            Err(error) => tracing::warn!(
                memory = ?id,
                %error,
                "consolidation: synthesis failed for a cluster; skipping"
            ),
        }
        Ok(())
    }

    /// Tier 2: retire more-private near-duplicates into their more-public counterparts, committed as
    /// one block. Structural only — no model call. Runs after tier 1 has committed, so it sees the
    /// synthesized replacements and dedups against them too.
    async fn tier2(&self, written: &[MemoryId]) -> Result<(), InstanceError> {
        let mut block = self.new_block()?;
        for &id in written {
            let entries: Vec<EntryView> = {
                let graph = self.engine.graph.lock();
                graph.class_entries(id)?
            };
            if entries.len() < 2 || entries.len() > MAX_ENTRIES_PER_CLASS {
                continue;
            }
            let embeddings = embed_class_entries(self.engine, id, &entries).await?;
            if embeddings.len() != entries.len() {
                continue;
            }
            for (replacement, sources) in
                tier2_absorptions(&entries, &embeddings, self.dedup_threshold)
            {
                if let Err(error) = block.consolidate_into(
                    id,
                    &sources,
                    replacement,
                    Some(self.produced_by.clone()),
                ) {
                    tracing::warn!(
                        memory = ?id,
                        %error,
                        "consolidation: tier 2 dedup rejected; skipping"
                    );
                }
            }
        }
        self.commit(block)
    }

    /// Open a maintenance block: `Teller::Agent`, `Authority::Agent`, no conversation or turn (the pass
    /// runs off the hot path), and no present set (writes do not consult it).
    fn new_block(&self) -> Result<MemoryBlock, InstanceError> {
        Ok(MemoryBlock::new(
            self.engine.clone(),
            Teller::Agent,
            Authority::Agent,
            None,
            None,
            Vec::new(),
            self.max_entry_chars,
        )?)
    }

    /// Commit a block's buffered events under [`EventSource::Orchestration`] and reproject, or do
    /// nothing when the block wrote nothing.
    fn commit(&self, block: MemoryBlock) -> Result<(), InstanceError> {
        let events = block.into_effects().events;
        if events.is_empty() {
            return Ok(());
        }
        let now = self.engine.clock.now();
        self.engine
            .store
            .lock()
            .append(now, EventSource::Orchestration, events)?;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        Ok(())
    }
}
