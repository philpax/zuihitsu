//! Content write operations: create, rename, append, supersede, revise, and set volatility.

use crate::{
    event::{ConversationRef, EventPayload, ProducedBy, Teller, Visibility},
    graph::{EntryView, GraphError, MemoryView},
    ids::{EntryId, MemoryId, MemoryName},
    memory::visibility::{subject_participant, visible_attestations},
    time::{TemporalRef, Timestamp},
};

use crate::memory::memory_block::{
    AppendOptions, AppendOutcome, Authority, Corroboration, EntrySelector, MemoryBlock,
    MemoryError, Retraction, effects::LiveEntry, reconcile_forced_visibility,
    suggest::most_similar,
};

impl MemoryBlock {
    /// Create a memory, optionally with a first content entry. The name must be free — a collision is
    /// a teachable error rejected before anything is buffered, so a duplicate `MemoryCreated` never
    /// reaches the log (where it would only fail at materialize, poisoning replay).
    pub fn create(
        &mut self,
        name: impl Into<MemoryName>,
        content: Option<&str>,
    ) -> Result<MemoryId, MemoryError> {
        self.create_with_opts(name, content, None)
    }

    /// Create a memory with optional first-entry overrides, mirroring `append`'s option table. This
    /// keeps `memory.create(name, content, opts)` from silently dropping `occurred_at`, a footgun that
    /// produced untimed reminders that never fired. The first entry is resolved before anything is
    /// buffered, so an unclassified write fails without leaving a half-created memory; the whole
    /// operation runs as a [`MemoryBlock::transaction`] so a later failure would roll the create back
    /// too.
    pub fn create_with_opts(
        &mut self,
        name: impl Into<MemoryName>,
        content: Option<&str>,
        opts: Option<AppendOptions>,
    ) -> Result<MemoryId, MemoryError> {
        self.transaction(|block| {
            let name = name.into();
            if block.resolve(name.as_str())?.is_some() {
                let similar = block.similar_names(&name)?;
                return Err(MemoryError::NameExists { name, similar });
            }
            let id = MemoryId::generate();
            // A first entry is told like any append: by the turn's teller, classified the same way (an
            // agent-authored first entry about a person must set its visibility). Resolve it before
            // buffering anything, so an unclassified write fails without leaving a half-created memory.
            let first_entry = match content {
                Some(text) => {
                    let mut opts = opts.unwrap_or_default();
                    let teller = entry_teller(&opts, &block.teller);
                    let forced = reconcile_forced_visibility(opts.visibility, opts.exclude.take())?;
                    // An unclassified inline seed about a person is refused regardless of teller:
                    // it would take the write-time default silently — PrivateToTeller for a
                    // participant-told fact — and the fact would vanish for every other audience,
                    // discovered only when someone else is refused it. Create-path-only: a bare
                    // `:append`'s private landing is the aside guard working as designed, and a
                    // `#confidential` room's blanket private firm-up is likewise deliberate. The
                    // operator's console writes may take the default, as with the link gate.
                    if forced.is_none()
                        && !block.confidential_context
                        && block.authority == Authority::Platform
                        && subject_participant(name.as_str(), id).is_some()
                    {
                        return Err(MemoryError::VisibilityRequiredOnCreate);
                    }
                    let unforced = forced.is_none();
                    let visibility =
                        block.resolve_visibility(Some(name.as_str()), id, &teller, forced)?;
                    Self::validate_occurred_at(opts.occurred_at.as_ref())?;
                    // A seed that took the unforced default and landed open is remembered, so a
                    // later exclude append to this memory in the same block fails teachably rather
                    // than leaving the open copy beside the guard (see `open_default_seeds`).
                    if unforced && matches!(visibility, Visibility::Public | Visibility::Attributed)
                    {
                        block.open_default_seeds.insert(id);
                    }
                    Some((
                        text.to_owned(),
                        teller,
                        visibility,
                        opts.occurred_at,
                        opts.volatility,
                    ))
                }
                None => None,
            };
            block.touched.insert(id);
            block.buffer.push(EventPayload::memory_created(id, name));
            if let Some((text, teller, visibility, occurred_at, volatility)) = first_entry {
                // A created memory's first entry may carry an occurrence and an inline volatility, just
                // like a standalone `mem:append("...", { occurred_at = ..., volatility = ... })`.
                let entry_id = block.push_content(id, text, teller, visibility, occurred_at)?;
                // The seed entry mirrors the `description` argument, not a real occurrence. Flag it so
                // the turn-end temporal extraction skips it: were it left in the feed, its untimed text
                // would be stamped with the conversation's "now" and that fabricated date would collide
                // with a later, correctly-dated append on the same memory. An explicitly-supplied
                // `occurred_at` still stands — the extraction only ever touches untimed entries.
                block
                    .buffer
                    .push(EventPayload::entry_description_mirrored(id, entry_id));
                if let Some(volatility) = volatility {
                    block.buffer.push(EventPayload::memory_volatility_set(
                        id,
                        volatility.into_volatility(),
                    ));
                }
            }
            Ok(id)
        })
    }

    /// Rename a memory's handle: the same node under a new agent-facing name (spec §Identity →
    /// Renaming). The ULID and every relational reference are untouched — only the `name` and its FTS
    /// row change — so the memory carries its whole history forward, which is what lets the agent follow
    /// a person who changes the name they go by (a transition above all) without splitting or
    /// misaddressing them. Guarded like the agent's other writes, not gated like a merge: `self` is
    /// operator-only, and the new name must be free — renaming onto a handle that already belongs to a
    /// *different* memory is a collision (a teachable error), never a silent merge of the two. Renaming
    /// a memory to the name it already holds is a no-op.
    pub fn rename(&mut self, id: MemoryId, new_name: &str) -> Result<(), MemoryError> {
        self.guard_self(id)?;
        // The no-op first, so renaming any memory — a platform stub included — to its own current
        // name stays the documented no-op rather than tripping the shape guard below with a message
        // about squatting a binding the memory already holds.
        let existing = self.resolve(new_name)?;
        if existing == Some(id) {
            return Ok(());
        }
        // The platform-qualified namespace (`person/<user>@<platform>`) is connector-owned in both
        // directions under platform authority: a first contact binds a platform identity to whatever
        // memory bears the qualified name, so renaming onto the shape would squat a future
        // participant's binding.
        if self.authority == Authority::Platform && platform_qualified(new_name) {
            return Err(MemoryError::RenameOntoPlatformHandle {
                name: MemoryName::new(new_name),
            });
        }
        if existing.is_some() {
            let name = MemoryName::new(new_name);
            let similar = self.similar_names(&name)?;
            return Err(MemoryError::NameExists { name, similar });
        }
        // A rename always reaches here from a live handle, so the old name resolves; a vanished memory
        // is a defensive no-op (the materializer's update would touch no rows either).
        let Some(old_name) = self.resolve_name(id)? else {
            return Ok(());
        };
        // The other direction of the same ownership: a bound stub's name mirrors the platform's view
        // of the account and follows the platform — the connector renames it when the platform-side
        // name changes. The agent renames the person's bare profile instead.
        if self.authority == Authority::Platform && platform_qualified(old_name.as_str()) {
            return Err(MemoryError::RenameOfPlatformHandle { name: old_name });
        }
        self.touched.insert(id);
        self.buffer.push(EventPayload::memory_renamed(
            id,
            old_name,
            MemoryName::new(new_name),
        ));
        Ok(())
    }

    /// Append a content entry to `id`, running the dedup capture matrix first. As
    /// [`MemoryBlock::append`], but checks the candidate's embedding against the vector index for a
    /// near-identical live entry on the same identity class (the best such hit is `E`), and then:
    ///
    /// - **No hit**, or every hit skipped: an ordinary [`AppendOutcome::Appended`].
    /// - **`E` founded as another teller's confidence**, and the incoming teller differs: `E` is
    ///   invisible to this writer (an independent statement of a fact someone else confided must record
    ///   normally, never against a confidence its speaker was never told), so it is not the target and
    ///   the write appends.
    /// - **The incoming teller is `E`'s founding teller** (or a merged identity of theirs): the
    ///   [`MemoryError::DuplicateEntry`] teachable error — the writer already stands behind this fact.
    /// - **A different teller, `E` founded all-audience** (`Public`/`Attributed`): the write is recorded
    ///   as a corroboration — an [`EventPayload::EntryAttested`] adding this teller under the posture the
    ///   same append would have resolved (a private confirmation lands as a hidden attestation) — and
    ///   returns [`AppendOutcome::Corroborated`] with `E`'s id and a note. Idempotent: a teller already
    ///   attesting `E` at that posture is a no-op note; at a different posture it re-attests
    ///   (last-writer-wins).
    ///
    /// `opts.distinct_from` names an entry the scan skips, so a re-append the agent has decided is a
    /// genuinely different fact records anew rather than corroborating.
    ///
    /// The dedup check is best-effort: the vector index lags behind the log (the indexer runs on a
    /// timer), so entries appended in the current block or the last few seconds are not yet indexed.
    /// Same-block duplicates are missed — the consolidation pass catches these retroactively. A
    /// `None` embedding (graph-only instance, or an embed failure) skips the check.
    pub fn append_dedup(
        &mut self,
        id: MemoryId,
        text: &str,
        mut opts: AppendOptions,
        dedup_embedding: Option<&[f32]>,
    ) -> Result<AppendOutcome, MemoryError> {
        let distinct_from = match opts.distinct_from.take() {
            Some(selector) => Some(self.resolve_entry_ref(id, selector)?),
            None => None,
        };
        let told_by = entry_teller(&opts, &self.teller);
        match self.find_dedup_target(id, dedup_embedding, distinct_from, &told_by)? {
            Some((memory, entry)) => self.corroborate_append(memory, entry, text, opts),
            None => self.append(id, text, opts).map(AppendOutcome::Appended),
        }
    }

    /// Scan the vector index for the best live, same-class entry the candidate embedding duplicates and
    /// that this writer may capture against — the corroboration/duplicate target `E`, or `None` to
    /// append normally. A read: it locks the graph and vector index transiently and buffers nothing, so
    /// [`MemoryBlock::append_dedup`] can decide the capture against `&mut self` once the borrows here
    /// have released.
    fn find_dedup_target(
        &self,
        id: MemoryId,
        dedup_embedding: Option<&[f32]>,
        distinct_from: Option<EntryId>,
        told_by: &Teller,
    ) -> Result<Option<(MemoryView, EntryView)>, MemoryError> {
        let Some(embedding) = dedup_embedding else {
            return Ok(None);
        };
        let Some(retrieval) = &self.engine.retrieval else {
            return Ok(None);
        };
        let settings = crate::settings::Settings::from_store(self.engine.store.lock().as_ref())
            .unwrap_or_default();
        let threshold = settings.maintenance.dedup_similarity_threshold as f32;
        let hits = retrieval
            .vectors
            .lock()
            .search(embedding, 50)
            .map_err(|e| MemoryError::Graph(GraphError::Malformed(e.to_string())))?;
        let class_target = self.class_write_target(id)?;
        let target_class = {
            let graph = self.engine.graph.lock();
            graph.class_id(class_target)?.unwrap_or(class_target)
        };
        for hit in &hits {
            if hit.score < threshold {
                break;
            }
            let Some(crate::model::index::VectorKey::EntryContextual(entry_id)) =
                crate::model::index::VectorKey::parse(&hit.id)
            else {
                continue;
            };
            if Some(entry_id) == distinct_from {
                continue;
            }
            let candidate = {
                let graph = self.engine.graph.lock();
                graph.entry_by_id(entry_id)?
            };
            let Some((memory, entry)) = candidate else {
                continue;
            };
            let entry_class = {
                let graph = self.engine.graph.lock();
                graph.class_id(memory.id)?.unwrap_or(memory.id)
            };
            if entry.superseded_by.is_some() || entry_class != target_class {
                continue;
            }
            // Another teller's confidence is invisible to this check: an independent statement of a
            // fact someone else confided must append normally, not be captured against — or shown a
            // snippet of — a confidence its speaker was never told. Only an all-audience entry, or the
            // confiding teller's own (or a merged identity's) repeat, is a target.
            let visible_to_writer = match entry.visibility {
                Visibility::Public | Visibility::Attributed => true,
                Visibility::PrivateToTeller | Visibility::Exclude(_) => {
                    self.same_teller_class(&entry.told_by, told_by)?
                }
            };
            if visible_to_writer {
                return Ok(Some((memory, entry)));
            }
        }
        Ok(None)
    }

    /// The auto-attest arm of [`MemoryBlock::append_dedup`]: the write duplicates all-audience entry
    /// `E` (on `memory`) told by a *different* teller, so it is recorded as a corroboration rather than
    /// a second copy. The incoming teller re-recording their own founding fact is the
    /// [`MemoryError::DuplicateEntry`] teachable error instead. The confirmation's posture is resolved
    /// exactly as an append would resolve it, so a private confirmation lands as a hidden attestation;
    /// `E` is all-audience here, so that posture is narrower-or-equal by construction and the
    /// audience-widening invariant holds without an error path.
    fn corroborate_append(
        &mut self,
        memory: MemoryView,
        entry: EntryView,
        text: &str,
        mut opts: AppendOptions,
    ) -> Result<AppendOutcome, MemoryError> {
        let told_by = entry_teller(&opts, &self.teller);
        if self.same_teller_class(&entry.told_by, &told_by)? {
            return Err(MemoryError::DuplicateEntry {
                existing_entry_id: entry.entry_id,
                snippet: entry.text,
            });
        }
        let forced = reconcile_forced_visibility(opts.visibility, opts.exclude.take())?;
        let posture =
            self.resolve_visibility(Some(memory.name.as_str()), memory.id, &told_by, forced)?;
        debug_assert!(
            posture_width(&posture) <= posture_width(&entry.visibility),
            "auto-attest must never widen an all-audience entry",
        );
        let phrasing = (text != entry.text).then(|| text.to_owned());
        let existing: Vec<(Teller, Visibility)> = entry
            .attestations
            .iter()
            .map(|attestation| (attestation.teller.clone(), attestation.posture.clone()))
            .collect();
        let corroboration = self.record_attestation(AttestationWrite {
            memory: memory.id,
            entry: entry.entry_id,
            entry_text: &entry.text,
            founding_posture: &entry.visibility,
            existing: &existing,
            teller: told_by,
            posture,
            phrasing,
            note_style: NoteStyle::AutoAppend,
        })?;
        Ok(AppendOutcome::Corroborated(corroboration))
    }

    /// Attest an existing entry: stand behind its fact as a further teller (the current speaker, or
    /// `opts.told_by`/`by_agent`, mirroring append), rather than recording the fact anew. `entry` is
    /// resolved against `id`'s `same_as` class and must be a live entry of it. The attestation's posture
    /// resolves through the same machinery as an append (`opts.visibility`/`exclude`, or the write-time
    /// default). Teachable errors: an unknown entry ([`MemoryError::UnknownEntry`]); a posture wider than
    /// the entry's founding posture ([`MemoryError::AttestationWiderThanEntry`]) — the invariant's real
    /// check. An attestation already standing at the resolved posture is a success no-op note, not an
    /// error. Read-your-writes: the idempotence check folds this block's pending attestations, so a
    /// second attest of the same entry sees the first; folding pending attestations into the *entry
    /// reads* is deliberately deferred.
    pub fn attest(
        &mut self,
        id: MemoryId,
        entry: impl Into<EntrySelector>,
        mut opts: AppendOptions,
    ) -> Result<Corroboration, MemoryError> {
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        let entry = self.resolve_entry_ref(id, entry.into())?;
        let live = self.live_class_entries(id)?;
        let Some(target) = live.iter().find(|candidate| candidate.entry_id == entry) else {
            return Err(MemoryError::UnknownEntry(entry.0.to_string()));
        };
        let founding_posture = target.visibility.clone();
        let found = self.attest_target(entry)?;
        let told_by = entry_teller(&opts, &self.teller);
        let name = self.resolve_name(found.memory)?;
        let forced = reconcile_forced_visibility(opts.visibility, opts.exclude.take())?;
        let posture = self.resolve_visibility(
            name.as_ref().map(MemoryName::as_str),
            found.memory,
            &told_by,
            forced,
        )?;
        self.record_attestation(AttestationWrite {
            memory: found.memory,
            entry,
            entry_text: &found.text,
            founding_posture: &founding_posture,
            existing: &found.attestations,
            teller: told_by,
            posture,
            phrasing: None,
            note_style: NoteStyle::Explicit,
        })
    }

    /// The memory, text, and committed live attestations of `entry` — the target facts
    /// [`MemoryBlock::attest`] needs beyond its liveness check. Reads the committed graph, then this
    /// block's pending appends (an entry created this block has no committed row yet, and no committed
    /// attestations). An entry found nowhere is a teachable [`MemoryError::UnknownEntry`].
    fn attest_target(&self, entry: EntryId) -> Result<AttestTarget, MemoryError> {
        let committed = { self.engine.graph.lock().entry_by_id(entry)? };
        if let Some((memory, view)) = committed {
            let attestations = view
                .attestations
                .iter()
                .map(|attestation| (attestation.teller.clone(), attestation.posture.clone()))
                .collect();
            return Ok(AttestTarget {
                memory: memory.id,
                text: view.text,
                attestations,
            });
        }
        for event in &self.buffer {
            if let EventPayload::MemoryContentAppended {
                id, entry_id, text, ..
            } = event
                && *entry_id == entry
            {
                return Ok(AttestTarget {
                    memory: *id,
                    text: text.clone(),
                    attestations: Vec::new(),
                });
            }
        }
        Err(MemoryError::UnknownEntry(entry.0.to_string()))
    }

    /// Buffer one teller's attestation of an entry (or recognize an identical one already held), the
    /// shared engine behind the auto-attest and explicit-attest paths. Enforces the audience-widening
    /// invariant here — the resolved posture may not be wider than the entry's founding posture — so
    /// both paths are covered by the one check; the auto-attest path cannot trip it (its target is
    /// always all-audience), and the explicit `mem:attest` is where it bites. Idempotence folds the
    /// committed and pending attestations: a teller already attesting at this posture is a no-op, a
    /// different posture re-attests (last-writer-wins at the fold), and a new teller is added.
    fn record_attestation(
        &mut self,
        write: AttestationWrite<'_>,
    ) -> Result<Corroboration, MemoryError> {
        if posture_width(&write.posture) > posture_width(write.founding_posture) {
            return Err(MemoryError::AttestationWiderThanEntry);
        }
        let mut existing: Vec<(Teller, Visibility)> = write.existing.to_vec();
        existing.extend(self.pending_attestations(write.entry));
        let mut held_posture = None;
        for (teller, posture) in &existing {
            if self.same_teller_class(teller, &write.teller)? {
                held_posture = Some(posture.clone());
                break;
            }
        }
        let ulid = write.entry.0;
        let snippet = attestation_snippet(write.entry_text);
        let label = posture_label(&write.posture);
        if held_posture.as_ref() == Some(&write.posture) {
            let note = format!(
                "entry {ulid} — \"{snippet}\" — is already attested at {label} visibility; nothing recorded."
            );
            return Ok(Corroboration {
                entry: write.entry,
                note,
            });
        }
        let re_attest = held_posture.is_some();
        let attesters = existing.len() + 1;
        self.touched.insert(write.memory);
        self.buffer.push(EventPayload::EntryAttested {
            memory: write.memory,
            entry: write.entry,
            teller: write.teller,
            told_in: self.told_in.clone(),
            asserted_at: self.engine.clock.now(),
            posture: write.posture.clone(),
            phrasing: write.phrasing,
            source_entry: None,
            produced_by: None,
        });
        let note = if re_attest {
            format!(
                "updated your attestation of entry {ulid} — \"{snippet}\" — to {label} visibility."
            )
        } else {
            match write.note_style {
                NoteStyle::AutoAppend => format!(
                    "recorded as corroboration of entry {ulid} — \"{snippet}\"; now attested by \
                     {attesters} tellers. If what you were told is genuinely different, re-append \
                     with distinct_from = \"{ulid}\" and include the distinguishing detail."
                ),
                NoteStyle::Explicit => {
                    format!("attested entry {ulid} — \"{snippet}\" — at {label} visibility.")
                }
            }
        };
        Ok(Corroboration {
            entry: write.entry,
            note,
        })
    }

    /// Buffer a content entry for `id` — the agent's primary write, recording a fact, an
    /// observation, or a relayed claim. `opts.told_by` overrides the speaker (a relayed claim's
    /// source); `opts.by_agent` attributes it to the agent; with neither, it is the current speaker.
    /// `opts.visibility` forces the visibility; otherwise the write-time default applies (a
    /// `#confidential` room, or an aside about an absent third party, defaults private to the teller).
    pub fn append(
        &mut self,
        id: MemoryId,
        text: &str,
        mut opts: AppendOptions,
    ) -> Result<EntryId, MemoryError> {
        // A class-level fact told through a platform-agnostic handle lands on the class primary, not on
        // whichever member the clean name resolves to — so the guards, the visibility default, and the
        // entry itself all key on the redirect target (see [`MemoryBlock::class_write_target`]).
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        let told_by = entry_teller(&opts, &self.teller);
        let name = self.resolve_name(id)?;
        let forced = reconcile_forced_visibility(opts.visibility, opts.exclude.take())?;
        let visibility =
            self.resolve_visibility(name.as_ref().map(MemoryName::as_str), id, &told_by, forced)?;
        // An exclude landing beside this block's own unguarded seed is the one-plain-copy leak: the
        // seed took the unforced default and sits open, so the guard is undone the moment it
        // commits. Caught here, at the point of failure, so the agent reissues the block with the
        // seed classified too (or created bare) — no pre-teaching needed.
        if matches!(visibility, Visibility::Exclude(_)) && self.open_default_seeds.contains(&id) {
            return Err(MemoryError::UnguardedSeedBesideExclude);
        }
        // Reject a recurrence the scheduler cannot interpret before it is buffered, rather than
        // committing a Recurring entry that silently never fires. Surfaced as a teachable error so the
        // agent reissues with a supported rule.
        Self::validate_occurred_at(opts.occurred_at.as_ref())?;
        let entry_id =
            self.push_content(id, text.to_owned(), told_by, visibility, opts.occurred_at)?;
        // An inline volatility classification: set the memory's volatility alongside the append, so the
        // agent can mark a fast-changing fact in one call rather than a separate `set_volatility`.
        if let Some(volatility) = opts.volatility {
            self.buffer.push(EventPayload::memory_volatility_set(
                id,
                volatility.into_volatility(),
            ));
        }
        Ok(entry_id)
    }

    /// Supersede `old` with `new` on `id` — the agent corrected or retracted a fact, recording which
    /// entry replaces it (spec §Visibility → superseded entries are not live). Both must be live
    /// entries of `id`'s `same_as` class (a live read, so the lock layer holds the class). Buffers a
    /// `MemorySuperseded`; the superseded entry then drops from every live surface while remaining in
    /// history. Like an append, it is a write to `id`, so platform authority may not supersede a
    /// `self` entry; nor may a platform turn supersede another participant's confidence
    /// ([`MemoryBlock::guard_foreign_confidence_supersede`]).
    pub fn supersede(
        &mut self,
        id: MemoryId,
        old: impl Into<EntrySelector>,
        new: impl Into<EntrySelector>,
    ) -> Result<(), MemoryError> {
        // Recorded against the class primary when told through a platform-agnostic handle, matching where
        // `append` lands a class-level fact — the supersession's effect keys on the entry ids, which
        // `live_class_entries` gathers across the whole class either way, so the redirect only attributes
        // the event to the primary.
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        // Resolve a by-id-or-prefix reference against the class before validating it is live, so an
        // agent can address the entry by the id its read rendered rather than by holding the handle.
        let old = self.resolve_entry_ref(id, old.into())?;
        let new = self.resolve_entry_ref(id, new.into())?;
        let live = self.live_class_entries(id)?;
        let Some(old_entry) = live.iter().find(|entry| entry.entry_id == old) else {
            return Err(MemoryError::UnknownEntry(old.0.to_string()));
        };
        if !live.iter().any(|entry| entry.entry_id == new) {
            return Err(MemoryError::UnknownEntry(new.0.to_string()));
        }
        self.guard_foreign_confidence_supersede(old_entry)?;
        self.touched.insert(id);
        self.buffer
            .push(EventPayload::memory_superseded(id, old, new));
        Ok(())
    }

    /// Retract `entry` on `id`, recording `reason` — the agent withdraws a fact rather than replacing
    /// it in place (spec §Visibility → superseded entries are not live). Under a conversation turn
    /// (platform authority) this is **per-attester**: a fact is a set of tellers' accounts, so a
    /// speaker withdraws only its own attestation and the fact stands as long as another teller still
    /// attests it. Three shapes follow:
    ///
    /// - The speaker attests the entry **and other tellers do too** → an [`EventPayload::AttestationRetracted`]
    ///   for the speaker's account alone; the entry stays live on the rest, and the returned
    ///   [`Retraction::Withdrawn`] carries a note naming the remaining *visible* attesters.
    /// - The speaker is the entry's **sole teller** → a whole-entry `EntryRetracted`, as before.
    /// - The speaker **attests nothing** on it → a whole-entry retraction, still governed by
    ///   [`MemoryBlock::guard_foreign_confidence_supersede`]: permitted for a public/attributed fact,
    ///   refused for another participant's confidence.
    ///
    /// A maintenance pass (agent authority) and the console (operator authority) always retract the
    /// whole entry regardless of who else attests it — their reach is deliberate and must not be
    /// silently narrowed to a single account. `entry` must be a live entry of `id`'s `same_as` class,
    /// and `reason` must be non-empty (an unexplained retraction is unauditable). Guarded like
    /// `supersede`: platform authority may not retract a `self` entry. A model-driven caller (the
    /// link-cleanup maintenance pass) passes its `produced_by`; a mechanical or agent-authored
    /// retraction passes `None`. The last-attestation case routes through `EntryRetracted` here, so
    /// the read-your-writes fold ([`MemoryBlock::pending_superseded`]) sees the tombstone without
    /// having to reason over pending attestation-retractions.
    pub fn retract(
        &mut self,
        id: MemoryId,
        entry: impl Into<EntrySelector>,
        reason: &str,
        produced_by: Option<ProducedBy>,
    ) -> Result<Retraction, MemoryError> {
        // Recorded against the class primary when told through a platform-agnostic handle, matching where
        // `supersede` lands the tombstone — `live_class_entries` gathers the whole class either way, so
        // the redirect only attributes the event to the primary.
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(MemoryError::RetractionReasonRequired);
        }
        // Resolve a by-id-or-prefix reference against the class, like `supersede`, before the live check.
        let entry = self.resolve_entry_ref(id, entry.into())?;
        let live = self.live_class_entries(id)?;
        let Some(target) = live.iter().find(|e| e.entry_id == entry) else {
            return Err(MemoryError::UnknownEntry(entry.0.to_string()));
        };
        // A conversation turn retracts per-attester; a maintenance pass or the console always retracts
        // the whole entry.
        if self.authority == Authority::Platform {
            let speaker = self.teller.clone();
            let mut speaker_accounts: Vec<Teller> = Vec::new();
            let mut others_attest = false;
            for (teller, _) in &target.attestations {
                if self.same_teller_class(teller, &speaker)? {
                    speaker_accounts.push(teller.clone());
                } else {
                    others_attest = true;
                }
            }
            if !speaker_accounts.is_empty() && others_attest {
                // The speaker stands among several tellers: withdraw only its account. Name the
                // remaining visible attesters before buffering, so the note tells the agent the fact
                // still stands and by whom — for the present audience, never leaking a hidden attester.
                let remaining = self.remaining_visible_attesters(entry, &speaker)?;
                self.touched.insert(id);
                for account in speaker_accounts {
                    self.buffer.push(EventPayload::attestation_retracted(
                        id,
                        entry,
                        account,
                        reason,
                        produced_by.clone(),
                    ));
                }
                return Ok(Retraction::Withdrawn {
                    note: withdrawal_note(entry, &remaining),
                });
            }
        }
        // Whole-entry retraction: the speaker is the sole teller, a non-attester on a public/attributed
        // fact, or any agent/operator write. The foreign-confidence guard still governs a platform
        // non-attester's reach over a confidence.
        self.guard_foreign_confidence_supersede(target)?;
        self.touched.insert(id);
        self.buffer.push(EventPayload::entry_retracted(
            id,
            entry,
            reason,
            produced_by,
        ));
        Ok(Retraction::Entry)
    }

    /// Withdraw one named teller's attestation from `entry` under operator authority — the console's
    /// per-attester counterpart to [`MemoryBlock::retract`]. Buffers an
    /// [`EventPayload::AttestationRetracted`] for every live attestation of `teller`'s class on the
    /// entry; if that leaves no live attestation, the fold tombstones the entry exactly as a
    /// whole-entry retraction does (see the `AttestationRetracted` apply). `entry` must be a live entry
    /// of `id`'s class, `reason` non-empty, and `teller` must actually attest the entry
    /// ([`MemoryError::UnknownAttestation`] otherwise). Operator-only in practice — it emits under
    /// whatever authority the block carries, and the console builds an operator block.
    pub fn retract_attestation(
        &mut self,
        id: MemoryId,
        entry: impl Into<EntrySelector>,
        teller: Teller,
        reason: &str,
        produced_by: Option<ProducedBy>,
    ) -> Result<(), MemoryError> {
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        self.guard_operator(id)?;
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(MemoryError::RetractionReasonRequired);
        }
        let entry = self.resolve_entry_ref(id, entry.into())?;
        let live = self.live_class_entries(id)?;
        let Some(target) = live.iter().find(|e| e.entry_id == entry) else {
            return Err(MemoryError::UnknownEntry(entry.0.to_string()));
        };
        // Withdraw every live attestation of the named teller's class — a merged identity's account is
        // the same teller's, keyed on the exact stored teller value so the fold's `(entry, teller)`
        // row is matched.
        let mut accounts: Vec<Teller> = Vec::new();
        for (held, _) in &target.attestations {
            if self.same_teller_class(held, &teller)? {
                accounts.push(held.clone());
            }
        }
        if accounts.is_empty() {
            return Err(MemoryError::UnknownAttestation);
        }
        self.touched.insert(id);
        for account in accounts {
            self.buffer.push(EventPayload::attestation_retracted(
                id,
                entry,
                account,
                reason,
                produced_by.clone(),
            ));
        }
        Ok(())
    }

    /// The readable labels of the tellers still standing behind `entry`, other than `speaker`, that are
    /// visible to the present audience — the names a withdrawal note may safely surface. Reads the
    /// committed attestation set through the chip-rule predicate ([`visible_attestations`]), so a
    /// hidden attester (a confidence whose teller is absent, or blocked by the subject rule) is never
    /// named, and drops the withdrawing speaker's own class. A pending same-block attestation is not
    /// reflected (matching the deferred read-your-writes for attestation reads); the committed set is
    /// the note's basis, which suffices since the fact being withdrawn was already committed.
    fn remaining_visible_attesters(
        &self,
        entry: EntryId,
        speaker: &Teller,
    ) -> Result<Vec<String>, MemoryError> {
        let committed = { self.engine.graph.lock().entry_by_id(entry)? };
        let Some((memory, view)) = committed else {
            return Ok(Vec::new());
        };
        let visible_tellers: Vec<Teller> = {
            let graph = self.engine.graph.lock();
            let class_of = |mid| graph.class_id(mid).map(|class| class.unwrap_or(mid));
            visible_attestations(&view, &memory, &self.present_set, &class_of)?
                .into_iter()
                .map(|attestation| attestation.teller.clone())
                .collect()
        };
        let mut labels = Vec::new();
        for teller in visible_tellers {
            if !self.same_teller_class(&teller, speaker)? {
                labels.push(self.teller_label(&teller));
            }
        }
        Ok(labels)
    }

    /// Revise a fact in one call: append `text` as a new entry on `id`, then supersede `old` with it.
    /// This is the find-and-supersede flow without the append-then-supersede two-step — and it cannot
    /// half-apply: the append and supersede run as a [`MemoryBlock::transaction`], so if the supersede
    /// fails (e.g. `old` is not a live entry), the append's buffered event is rolled back and the error
    /// propagates, leaving no orphaned new entry beside the stale value. Returns the new entry.
    pub fn revise(
        &mut self,
        id: MemoryId,
        old: EntryId,
        text: &str,
        mut opts: AppendOptions,
    ) -> Result<EntryId, MemoryError> {
        // Carry the superseded entry's occurrence onto the replacement when the caller names none.
        // Revise supersedes `old`, and the representative-date projections read only live entries, so a
        // dateless replacement would erase a dated fact's date everywhere at once — its render, its
        // search hit, and every link's `[when …]`. An explicit occurred_at still wins.
        if opts.occurred_at.is_none() {
            opts.occurred_at = self.entry_occurred_at(id, old)?;
        }
        self.transaction(|block| {
            let new = block.append(id, text, opts)?;
            block.supersede(id, old, new)?;
            Ok(new)
        })
    }

    /// The occurrence recorded on `old`, read from this block's pending appends first, then the
    /// committed graph — so [`MemoryBlock::revise`] can carry it onto the replacement entry when the
    /// caller supplies none.
    fn entry_occurred_at(
        &self,
        id: MemoryId,
        old: EntryId,
    ) -> Result<Option<TemporalRef>, MemoryError> {
        if let Some(entry) = self.entry_ref_by_id(old) {
            return Ok(entry.occurred_at);
        }
        Ok(self
            .engine
            .graph
            .lock()
            .class_history(id)?
            .into_iter()
            .find(|entry| entry.entry_id == old)
            .and_then(|entry| entry.occurred_at))
    }

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

    /// Set a memory's volatility — how fast its facts go out of date. `high` for fast-changing status
    /// (a current location, a mood), `low` for durable facts, `medium` the default. Volatility steepens
    /// the recency decay in search and, for `high`, lets an aged entry read as stale so the agent hedges
    /// rather than asserting it as current (spec §Recency and volatility).
    pub fn set_volatility(&mut self, id: MemoryId, level: &str) -> Result<(), MemoryError> {
        // Volatility classifies how fast a memory's facts age, so a class-level classification told
        // through a platform-agnostic handle lands on the primary — the same node `append` funnels the
        // class's facts to, keeping the inline `append` volatility opt and a standalone call consistent.
        let id = self.class_write_target(id)?;
        self.guard_self(id)?;
        let volatility = level
            .parse()
            .map_err(|()| MemoryError::UnknownVolatility(level.to_owned()))?;
        self.touched.insert(id);
        self.buffer
            .push(EventPayload::memory_volatility_set(id, volatility));
        Ok(())
    }

    /// The near-matching existing handles in `attempted`'s namespace, closest first — the suggestions
    /// a name collision surfaces so the agent picks a distinguishing name. Scoped to the namespace,
    /// since a near-duplicate is a handle of the same kind; a handle in no known namespace (only the
    /// reserved `self`) has none. Reads the committed graph, like the block's other lookups.
    ///
    /// The candidate fetch is pushed down to an indexed first-character slice rather than pulling
    /// the whole namespace: both relevance gates in [`most_similar`] require the candidate to share
    /// at least the attempted subject's first character (the stem gate needs a shared leading run,
    /// and the typo gate explicitly requires one shared leading character), so a candidate not
    /// sharing it can never be suggested, and fetching only that slice is provably sufficient. The
    /// gates compare ASCII-case-insensitively, so both case variants' ranges are fetched (they are
    /// disjoint, so nothing repeats, and [`most_similar`]'s total ordering makes the concatenation
    /// order irrelevant). A subject with no first character cannot be sliced, so it falls back to
    /// ranking the whole namespace, preserving the unsliced behavior.
    pub(super) fn similar_names(
        &self,
        attempted: &MemoryName,
    ) -> Result<Vec<MemoryName>, GraphError> {
        let Ok(namespaced) = attempted.namespaced() else {
            return Ok(Vec::new());
        };
        let prefix = namespaced.namespace.prefix();
        let names = match namespaced.subject.chars().next() {
            Some(first) => {
                let lower = first.to_ascii_lowercase();
                let upper = first.to_ascii_uppercase();
                let graph = self.engine.graph.lock();
                let mut names = graph.memory_names_with_prefix(&format!("{prefix}{lower}"))?;
                if upper != lower {
                    names.extend(graph.memory_names_with_prefix(&format!("{prefix}{upper}"))?);
                }
                names
            }
            None => self
                .engine
                .graph
                .lock()
                .memories_in_namespace(prefix)?
                .into_iter()
                .map(|memory| memory.name)
                .collect(),
        };
        let candidates = names
            .into_iter()
            .map(|name| {
                let subject = name
                    .as_str()
                    .strip_prefix(prefix)
                    .unwrap_or(name.as_str())
                    .to_owned();
                (subject, name)
            })
            .collect();
        Ok(most_similar(&namespaced.subject, candidates))
    }
}

/// The target facts an explicit [`MemoryBlock::attest`] resolves beyond its liveness check: the entry's
/// own memory, its text (for the corroboration note), and the committed live attestations already
/// standing behind it (for the idempotence check).
struct AttestTarget {
    memory: MemoryId,
    text: String,
    attestations: Vec<(Teller, Visibility)>,
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

/// The inputs to [`MemoryBlock::record_attestation`], bundled so the auto-attest and explicit-attest
/// paths hand it one cohesive request rather than a long positional list. `founding_posture` and
/// `existing` describe the target entry (its own posture and the attestations already standing behind
/// it); `teller`/`posture`/`phrasing` are the incoming endorsement; `note_style` picks the wording of
/// the agent-facing note.
struct AttestationWrite<'a> {
    memory: MemoryId,
    entry: EntryId,
    entry_text: &'a str,
    founding_posture: &'a Visibility,
    existing: &'a [(Teller, Visibility)],
    teller: Teller,
    posture: Visibility,
    phrasing: Option<String>,
    note_style: NoteStyle,
}

/// Which register a corroboration note is written in: the auto-attest path (an append folded into an
/// existing entry) points the agent at `distinct_from` for the case it meant a different fact, while
/// the explicit `mem:attest` path just confirms the endorsement it deliberately made.
#[derive(Clone, Copy)]
enum NoteStyle {
    AutoAppend,
    Explicit,
}

/// The audience breadth of a visibility posture, for the audience-widening invariant: an attestation
/// may sit at or below its entry's founding posture, never above it. `Public` and `Attributed` both
/// surface to the whole audience, so they share the widest tier; `PrivateToTeller` reaches only the
/// teller, and an `Exclude` is narrower still (withheld from the named parties on top). Comparing these
/// ranks, a `Public` attestation on an `Attributed`-founded entry is level (allowed), while a
/// `PrivateToTeller` attestation on an `Exclude`-founded entry would widen it (rejected).
fn posture_width(visibility: &Visibility) -> u8 {
    match visibility {
        Visibility::Public | Visibility::Attributed => 2,
        Visibility::PrivateToTeller => 1,
        Visibility::Exclude(_) => 0,
    }
}

/// The agent-facing label for an attestation's posture in a corroboration note — the same three-way
/// register a read uses: `public`, `attributed`, or `private` for a confidence.
fn posture_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Attributed => "attributed",
        Visibility::PrivateToTeller | Visibility::Exclude(_) => "private",
    }
}

/// The note a per-attester withdrawal hands back: the speaker's account is gone but the fact stands,
/// attested by the remaining tellers visible to the present audience. Names them when any are visible;
/// when the survivors are all hidden from this audience, it says the fact still stands without naming
/// anyone, so a hidden attester's existence is never leaked through the note.
fn withdrawal_note(entry: EntryId, remaining: &[String]) -> String {
    let ulid = entry.0;
    if remaining.is_empty() {
        return format!(
            "withdrew your account of entry {ulid}; the fact still stands, attested by others."
        );
    }
    let who = remaining.join(", ");
    format!("withdrew your account of entry {ulid} — the fact still stands, attested by {who}.")
}

/// A one-line snippet of an entry's text for a corroboration note, clipped so the note stays compact.
fn attestation_snippet(text: &str) -> String {
    const MAX: usize = 60;
    let text = text.trim();
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let clipped: String = text.chars().take(MAX).collect();
    format!("{clipped}…")
}

/// Whether `name` is a platform-qualified participant handle (`person/<user>@<platform>`) — the
/// connector-owned shape the rename guards key on. Parse failures read as unqualified, so a
/// malformed name falls through to the ordinary checks rather than being mistaken for a stub.
fn platform_qualified(name: &str) -> bool {
    MemoryName::new(name)
        .namespaced()
        .is_ok_and(|name| name.is_platform_qualified())
}

/// The teller a content entry is stamped with: an explicit `told_by` (a relayed claim attributed to
/// its source) wins over everything, then `by_agent` (the agent's own observation), and otherwise the
/// current speaker. Shared by `append` and `create_with_opts` so a created memory's first entry and a
/// later append attribute identically.
fn entry_teller(opts: &AppendOptions, speaker: &Teller) -> Teller {
    opts.told_by.clone().unwrap_or_else(|| {
        if opts.by_agent {
            Teller::Agent
        } else {
            speaker.clone()
        }
    })
}
