//! Consolidation writes: the maintenance pass's two ways to retire near-duplicate entries. Tier 1
//! ([`MemoryBlock::consolidate`]) replaces a same-level cluster with a freshly synthesized entry,
//! preserving each distinct source teller as an attestation; tier 2 ([`MemoryBlock::consolidate_into`])
//! folds narrower near-duplicates into an already-live wider entry, absorbing each as an attestation
//! rather than dropping it. Both preserve every source's teller and audience so nothing leaks or is lost.

use crate::{
    event::{ConversationRef, EventPayload, ProducedBy, Teller, Visibility},
    ids::{EntryId, MemoryId},
    memory::memory_block::{MemoryBlock, MemoryError, attest::posture_width, effects::LiveEntry},
    time::Timestamp,
};

impl MemoryBlock {
    /// Tier 1 consolidation: replace `sources` with a freshly synthesized entry on `id`'s `same_as`
    /// class. Every source must be a live entry of the class sharing one visibility level — the
    /// maintenance pass groups its clusters by level before synthesizing, so a cluster is always
    /// same-level — and the replacement inherits that level verbatim. Its teller is the sources' shared
    /// teller, or [`Teller::Agent`] for a cross-teller merge, which is permitted only at
    /// [`Visibility::Public`] or [`Visibility::Attributed`] — the two all-audience levels, where every
    /// source surfaces to everyone, so a synthesized replacement leaks nothing.
    ///
    /// Buffers the replacement's `MemoryContentAppended`, then one [`EventPayload::EntryAttested`] per
    /// distinct source teller-class (the replacement's own founding teller skipped — it is already the
    /// append's attestation, so a uniform-teller merge emits none), then the
    /// [`EventPayload::EntriesConsolidated`] that tombstones the sources, all as one transaction. The
    /// attestations carry each source's `told_in` and earliest `asserted_at`, so a cross-teller merge
    /// preserves who the accounts came from rather than losing them to the agent-synthesized text. Their
    /// posture is the group's visibility, equal to the replacement's founding posture, so the audience-
    /// widening invariant holds by construction. Runs the foreign-confidence supersede guard on each
    /// source like [`MemoryBlock::supersede`]: a maintenance pass drives it under `Authority::Agent`,
    /// which the guard clears, and preserving each source's teller and level keeps the fact visible to
    /// exactly its original audience.
    pub fn consolidate(
        &mut self,
        id: MemoryId,
        sources: &[EntryId],
        replacement_text: String,
        produced_by: Option<ProducedBy>,
    ) -> Result<EntryId, MemoryError> {
        self.transaction(|block| {
            let id = block.class_write_target(id)?;
            block.guard_self(id)?;
            block.guard_operator(id)?;
            let live = block.live_class_entries(id)?;
            let mut visibility: Option<Visibility> = None;
            let mut teller: Option<Teller> = None;
            let mut uniform_teller = true;
            for source in sources {
                let Some(entry) = live.iter().find(|entry| entry.entry_id == *source) else {
                    return Err(MemoryError::UnknownEntry(source.0.to_string()));
                };
                block.guard_foreign_confidence_supersede(entry)?;
                match &visibility {
                    None => visibility = Some(entry.visibility.clone()),
                    Some(level) if *level != entry.visibility => {
                        return Err(MemoryError::ConsolidationInvariant(
                            "a cluster's sources must share one visibility level",
                        ));
                    }
                    Some(_) => {}
                }
                match &teller {
                    None => teller = Some(entry.told_by.clone()),
                    Some(shared) if *shared != entry.told_by => uniform_teller = false,
                    Some(_) => {}
                }
            }
            let Some(visibility) = visibility else {
                return Err(MemoryError::ConsolidationInvariant(
                    "a consolidation needs at least one source",
                ));
            };
            // A cross-teller merge collapses the founding attribution to the agent (the synthesized text
            // is nobody's verbatim account), which is only sound at an all-audience level — public or
            // attributed — where every source already surfaces to everyone. Below that, the teller is the
            // audience-bearing payload and the tier-1 grouping keeps such entries per teller, so a
            // cross-teller merge never reaches here.
            if !uniform_teller && !matches!(visibility, Visibility::Public | Visibility::Attributed)
            {
                return Err(MemoryError::ConsolidationInvariant(
                    "a cross-teller merge is only permitted at an all-audience visibility level",
                ));
            }
            let told_by = if uniform_teller {
                teller.expect("a teller is recorded whenever a source is seen")
            } else {
                Teller::Agent
            };
            let replacement = block.push_content(
                id,
                replacement_text,
                told_by.clone(),
                visibility.clone(),
                None,
            )?;
            // Preserve each distinct source teller as an attestation on the synthesized replacement, so
            // a cross-teller merge does not lose who the accounts came from. The replacement's own
            // founding teller is skipped (it stands as the append's own attestation), so a uniform-teller
            // merge emits none. The posture is the group's visibility — equal to the replacement's
            // founding posture — so no attestation is wider than the entry it stands on.
            for group in block.distinct_source_tellers(sources, &live, &told_by)? {
                block.buffer.push(EventPayload::EntryAttested {
                    memory: id,
                    entry: replacement,
                    teller: group.teller,
                    told_in: group.told_in,
                    asserted_at: group.asserted_at,
                    posture: visibility.clone(),
                    phrasing: None,
                    source_entry: Some(group.source_entry),
                    produced_by: produced_by.clone(),
                });
            }
            block.buffer.push(EventPayload::entries_consolidated(
                id,
                sources.to_vec(),
                replacement,
                produced_by,
            ));
            Ok(replacement)
        })
    }

    /// Tier 2 consolidation: retire `sources` into an already-live `replacement` entry, absorbing each
    /// source's fact as an attestation on the replacement rather than merely dropping it. The maintenance
    /// pass uses this to fold a narrower near-duplicate (a private confidence, or an attributed account)
    /// whose fact is already attested by a wider entry (the `replacement`), so no `MemoryContentAppended`
    /// is written — the replacement already exists — and the retired text never enters a synthesis.
    ///
    /// For each retired source, one [`EventPayload::EntryAttested`] is buffered on the replacement,
    /// carrying the source's teller, `told_in`, `asserted_at`, exact text (as `phrasing`), and posture,
    /// *before* the [`EventPayload::EntriesConsolidated`] that tombstones the sources. The attestation's
    /// posture is the source's own visibility, narrower than or equal to the all-audience replacement, so
    /// the audience-widening invariant holds by construction — a hidden private attestation on a public
    /// entry, or an attribution-bearing one for a folded attributed source. When the source's teller-class
    /// already attests the replacement at that same posture, the attestation is skipped to keep the log
    /// lean (the fold would be an idempotent no-op); a different posture still emits, upserting last-
    /// writer-wins. Every `source` and the `replacement` must be a live entry of `id`'s `same_as` class.
    /// Runs the foreign-confidence supersede guard on each source: this is deliberately the cross-teller,
    /// cross-posture case the guard exists for, so it is permitted only under `Authority::Agent` (which
    /// the guard clears), and only because the pass has verified the fact is already attested at a wider
    /// level whose audience is a superset of the source's.
    pub fn consolidate_into(
        &mut self,
        id: MemoryId,
        sources: &[EntryId],
        replacement: EntryId,
        produced_by: Option<ProducedBy>,
    ) -> Result<(), MemoryError> {
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        if sources.is_empty() {
            return Err(MemoryError::ConsolidationInvariant(
                "a consolidation needs at least one source",
            ));
        }
        let live = self.live_class_entries(id)?;
        let Some(target) = live.iter().find(|entry| entry.entry_id == replacement) else {
            return Err(MemoryError::UnknownEntry(replacement.0.to_string()));
        };
        // Plan every source's absorbing attestation before touching the buffer, so a mid-loop error
        // (a teller-class comparison locks the graph) leaves nothing half-written. `running` folds the
        // replacement's committed attestations with the ones planned here, so a second source of the
        // same teller-class and posture dedups against an earlier one in the same batch.
        let mut running: Vec<(Teller, Visibility)> = target.attestations.clone();
        let mut planned: Vec<EventPayload> = Vec::new();
        for source in sources {
            let Some(entry) = live.iter().find(|entry| entry.entry_id == *source) else {
                return Err(MemoryError::UnknownEntry(source.0.to_string()));
            };
            self.guard_foreign_confidence_supersede(entry)?;
            let mut already_attested = false;
            for (teller, posture) in &running {
                if *posture == entry.visibility && self.same_teller_class(teller, &entry.told_by)? {
                    already_attested = true;
                    break;
                }
            }
            if already_attested {
                continue;
            }
            // The planner only ever absorbs a narrower-or-attribution-preserving source into an
            // all-audience target, so the carried attestation cannot widen past the replacement's
            // founding posture — the tripwire guards the invariant against a future caller bug.
            debug_assert!(
                posture_width(&entry.visibility) <= posture_width(&target.visibility),
                "an absorption must never carry an attestation wider than its replacement's founding posture"
            );
            running.push((entry.told_by.clone(), entry.visibility.clone()));
            planned.push(EventPayload::EntryAttested {
                memory: id,
                entry: replacement,
                teller: entry.told_by.clone(),
                told_in: entry.told_in.clone(),
                asserted_at: entry.asserted_at,
                posture: entry.visibility.clone(),
                phrasing: Some(entry.text.clone()),
                source_entry: Some(entry.entry_id),
                produced_by: produced_by.clone(),
            });
        }
        self.touched.insert(id);
        for event in planned {
            self.buffer.push(event);
        }
        self.buffer.push(EventPayload::entries_consolidated(
            id,
            sources.to_vec(),
            replacement,
            produced_by,
        ));
        Ok(())
    }

    /// The distinct source teller-classes of a tier-1 consolidation, each reduced to the
    /// earliest-asserted source that carries it — the attestations [`MemoryBlock::consolidate`] leaves on
    /// the synthesized replacement so a cross-teller merge preserves who its accounts came from. The
    /// `founding` teller (the replacement's own attribution) is dropped, since it already stands as the
    /// append's founding attestation, so a uniform-teller merge yields nothing. Sources are grouped by
    /// teller *class* — a merged identity of a teller is the same teller — keeping the earliest
    /// `asserted_at`, and that source's `told_in` and id, as the group's representative.
    fn distinct_source_tellers(
        &self,
        sources: &[EntryId],
        live: &[LiveEntry],
        founding: &Teller,
    ) -> Result<Vec<SourceTeller>, MemoryError> {
        let mut groups: Vec<SourceTeller> = Vec::new();
        for source in sources {
            let entry = live
                .iter()
                .find(|entry| entry.entry_id == *source)
                .expect("each source was validated as a live entry above");
            if self.same_teller_class(&entry.told_by, founding)? {
                continue;
            }
            let mut placed = false;
            for group in &mut groups {
                if self.same_teller_class(&group.teller, &entry.told_by)? {
                    // Keep the earliest-asserted source as the class's representative, so the
                    // attestation carries the account's first assertion, not an arbitrary one.
                    if entry.asserted_at < group.asserted_at {
                        group.teller = entry.told_by.clone();
                        group.told_in = entry.told_in.clone();
                        group.asserted_at = entry.asserted_at;
                        group.source_entry = entry.entry_id;
                    }
                    placed = true;
                    break;
                }
            }
            if !placed {
                groups.push(SourceTeller {
                    teller: entry.told_by.clone(),
                    told_in: entry.told_in.clone(),
                    asserted_at: entry.asserted_at,
                    source_entry: entry.entry_id,
                });
            }
        }
        Ok(groups)
    }
}

/// One distinct source teller-class of a tier-1 consolidation, reduced to its earliest-asserted source:
/// the teller, that source's `told_in` and `asserted_at`, and its entry id — the provenance
/// [`MemoryBlock::consolidate`] stamps onto the attestation it leaves on the synthesized replacement.
struct SourceTeller {
    teller: Teller,
    told_in: Option<ConversationRef>,
    asserted_at: Timestamp,
    source_entry: EntryId,
}
