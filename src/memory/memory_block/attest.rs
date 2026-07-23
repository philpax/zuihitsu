//! Attestation writes: the dedup capture matrix behind `append`, the explicit `mem:attest`, and the
//! shared engine that buffers one teller's endorsement of an entry while enforcing the audience-widening
//! invariant. An append checks its candidate against the vector index for a near-identical live entry and
//! either records normally, refuses a self-duplicate, or folds into a corroboration; `mem:attest` lets
//! the agent stand behind an existing fact deliberately. Both paths converge on `record_attestation`.

use crate::{
    event::{EventPayload, Teller, Visibility},
    graph::{EntryView, GraphError, MemoryView},
    ids::{EntryId, MemoryId, MemoryName},
};

use crate::memory::memory_block::{
    AppendOptions, AppendOutcome, Corroboration, EntrySelector, MemoryBlock, MemoryError,
    reconcile_forced_visibility, writes::entry_teller,
};

impl MemoryBlock {
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
        let scan = self.find_dedup_target(id, dedup_embedding, distinct_from, &told_by)?;
        let advisory = scan.cross_class.as_ref().map(|(memory, entry)| {
            format!(
                "note: a near-identical fact already lives on {} [{}]: \"{}\". If this is one \
                 fact about several subjects, record it once where it most belongs — the shared \
                 topic or event — and link the cast, rather than re-phrasing it per subject.",
                memory.name.as_str(),
                entry.entry_id.0,
                attestation_snippet(&entry.text),
            )
        });
        match scan.capture {
            Some((memory, entry)) => self.corroborate_append(memory, entry, text, opts),
            None => Ok(AppendOutcome::Appended {
                entry: self.append(id, text, opts)?,
                advisory,
            }),
        }
    }
}

/// One dedup scan's outcome: the capture target when the write duplicates a same-class entry, and
/// the best all-audience cross-class near-duplicate for the advisory note when one exists.
#[derive(Default)]
struct DedupScan {
    capture: Option<(MemoryView, EntryView)>,
    cross_class: Option<(MemoryView, EntryView)>,
}

impl MemoryBlock {
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
    ) -> Result<DedupScan, MemoryError> {
        let Some(embedding) = dedup_embedding else {
            return Ok(DedupScan::default());
        };
        let Some(retrieval) = &self.engine.retrieval else {
            return Ok(DedupScan::default());
        };
        let settings = crate::settings::Settings::from_store(self.engine.store.lock().as_ref())
            .unwrap_or_default();
        // Two bars: a capture needs the strict dedup threshold, while the cross-subject advisory
        // fires from the looser consolidation band — it is non-blocking, and a subject-prefixed
        // embedding puts the same fact about two subjects below the dedup bar by construction.
        let capture_threshold = settings.maintenance.dedup_similarity_threshold as f32;
        let advisory_threshold =
            (settings.maintenance.consolidation_similarity_threshold as f32).min(capture_threshold);
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
        let mut cross_class: Option<(MemoryView, EntryView)> = None;
        for hit in &hits {
            if hit.score < advisory_threshold {
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
            if entry.superseded_by.is_some() {
                continue;
            }
            if entry_class != target_class {
                // A different subject's near-identical fact is never a capture (a different subject
                // is a different fact by policy), but the best all-audience one is worth an advisory:
                // the same fact re-phrased once per participant is the duplication the shared topic
                // and links exist to prevent. A cross-class confidence stays wholly invisible — no
                // snippet, no existence — exactly as it is invisible to the capture itself.
                if cross_class.is_none()
                    && matches!(
                        entry.visibility,
                        Visibility::Public | Visibility::Attributed
                    )
                {
                    cross_class = Some((memory, entry));
                }
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
            if visible_to_writer && hit.score >= capture_threshold {
                return Ok(DedupScan {
                    capture: Some((memory, entry)),
                    cross_class,
                });
            }
        }
        Ok(DedupScan {
            capture: None,
            cross_class,
        })
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
}

/// The target facts an explicit [`MemoryBlock::attest`] resolves beyond its liveness check: the entry's
/// own memory, its text (for the corroboration note), and the committed live attestations already
/// standing behind it (for the idempotence check).
struct AttestTarget {
    memory: MemoryId,
    text: String,
    attestations: Vec<(Teller, Visibility)>,
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
pub(super) fn posture_width(visibility: &Visibility) -> u8 {
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
