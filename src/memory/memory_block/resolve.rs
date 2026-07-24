//! Resolution and read-your-writes helpers: visibility resolution, pending-state overlays,
//! entry projection, and name resolution.

use std::collections::BTreeSet;

use ulid::Ulid;

use crate::{
    decay,
    event::{EventPayload, Teller, Visibility},
    graph::{EntryView, Graph, GraphError},
    ids::{EntryId, MemoryId, MemoryName},
    memory::{
        memory_block::{
            Authority, EntryRef, EntrySelector, ForcedVisibility, MIN_ENTRY_PREFIX, MemoryBlock,
            MemoryError, VisibilityChoice, WITHHELD_STUB,
        },
        visibility::{
            default_link_visibility, default_visibility_named, subject_participant, visible,
            visible_attestations,
        },
    },
};

/// A live-or-history entry with the read-time annotations [`MemoryBlock::annotate`] computes: whether
/// its content is `withheld` from the present audience, whether it is `stale`, and the readable labels
/// of the attesters the audience may see. Projected into an [`EntryRef`] by [`MemoryBlock::entry_ref`].
pub(super) struct AnnotatedEntry {
    pub(super) entry: EntryView,
    pub(super) withheld: bool,
    pub(super) stale: bool,
    /// The visible attesting tellers (agent excluded), resolved to labels by
    /// [`MemoryBlock::entry_ref`] off the graph lock the annotating read holds.
    pub(super) attesters: Vec<Teller>,
}

impl MemoryBlock {
    /// The visibility a content entry is written at, or a teachable failure. An explicit choice is
    /// honored verbatim. With none: a `#confidential` room firms everything private; otherwise an
    /// agent-authored entry about a *person* (a subject-bearing memory) has no protective default —
    /// the participant-aside mechanism keys on a participant teller, not the agent, so silently
    /// defaulting to public is how a re-recorded confidence leaks — and must be classified. Any other
    /// write (a participant teller, or a non-subject memory like `self`/[`Namespace::Topic`]) takes the
    /// namespace/subject default.
    pub(super) fn resolve_visibility(
        &self,
        name: Option<&str>,
        id: MemoryId,
        told_by: &Teller,
        forced: Option<ForcedVisibility>,
    ) -> Result<Visibility, MemoryError> {
        if let Some(forced) = forced {
            return Ok(forced_to_visibility(forced));
        }
        if self.confidential_context {
            return Ok(Visibility::PrivateToTeller);
        }
        let about_a_person = name.is_some_and(|name| subject_participant(name, id).is_some());
        if matches!(told_by, Teller::Agent) && about_a_person {
            return Err(MemoryError::VisibilityRequired);
        }
        Ok(match name {
            Some(name) => default_visibility_named(name, id, told_by),
            None => Visibility::Public,
        })
    }

    /// The visibility a link is written at, or a teachable failure. An explicit choice is honored
    /// verbatim. With none: a `#confidential` room firms everything private; otherwise an agent-authored
    /// link about a *person* (a subject-bearing target) has no protective default — the same
    /// teachable-error gate as content. Any other write (a participant teller, or a non-person target)
    /// takes the link default, which distinguishes direct beliefs (`PrivateToTeller`) from relayed facts
    /// (`Attributed`).
    pub(super) fn resolve_link_visibility(
        &self,
        from: MemoryId,
        from_name: Option<&str>,
        to: MemoryId,
        to_name: Option<&str>,
        told_by: &Teller,
        forced: Option<ForcedVisibility>,
    ) -> Result<Visibility, MemoryError> {
        if let Some(forced) = forced {
            return Ok(forced_to_visibility(forced));
        }
        if self.confidential_context {
            return Ok(Visibility::PrivateToTeller);
        }
        // The teachable-error gate fires only for platform-authority (agent) writes about a person —
        // the operator is explicitly asserting from the console and may take the default.
        let about_a_person = to_name.is_some_and(|name| subject_participant(name, to).is_some());
        if self.authority == Authority::Platform
            && matches!(told_by, Teller::Agent)
            && about_a_person
        {
            return Err(MemoryError::VisibilityRequired);
        }
        match (from_name, to_name) {
            (Some(from_name), Some(to_name)) => Ok(default_link_visibility(
                from, from_name, to, to_name, told_by,
            )),
            _ => Ok(Visibility::Public),
        }
    }

    /// Record `id` and its `same_as` class as touched (a traversing read locks the whole class), and
    /// return the class as a set for membership tests against the pending buffer.
    pub(super) fn touch_class(
        &mut self,
        id: MemoryId,
        members: Vec<MemoryId>,
    ) -> BTreeSet<MemoryId> {
        self.touched.insert(id);
        let mut set = BTreeSet::new();
        for member in members {
            self.touched.insert(member);
            set.insert(member);
        }
        set.insert(id);
        set
    }

    /// The entries this block has superseded, retracted, or consolidated away but not yet committed —
    /// applied to the live reads so a correction's effect is visible within the block
    /// (read-your-writes). A retraction tombstones its entry exactly as a supersession does, and a
    /// consolidation tombstones each of its source entries, so all of them drop from the live read here.
    pub(super) fn pending_superseded(&self) -> BTreeSet<EntryId> {
        let mut superseded = BTreeSet::new();
        for event in &self.buffer {
            match event {
                EventPayload::MemorySuperseded { entry, .. }
                | EventPayload::EntryRetracted { entry, .. } => {
                    superseded.insert(*entry);
                }
                EventPayload::EntriesConsolidated { sources, .. } => {
                    superseded.extend(sources.iter().copied());
                }
                _ => {}
            }
        }
        superseded
    }

    /// This block's pending attestations of `entry` — the `(teller, posture)` pairs buffered but not yet
    /// committed — folded into the attestation write path so a second capture of the same entry within
    /// the block sees the first (read-your-writes). Deliberately minimal: it reads only pending
    /// [`EventPayload::EntryAttested`] events, since attestation retraction is not a block operation
    /// here, and it is consulted only by the write path's idempotence check, not by the entry reads (a
    /// pending attestation widening an entry's visibility mid-block is not reflected in `mem:entries`).
    pub(super) fn pending_attestations(&self, entry: EntryId) -> Vec<(Teller, Visibility)> {
        self.buffer
            .iter()
            .filter_map(|event| match event {
                EventPayload::EntryAttested {
                    entry: attested,
                    teller,
                    posture,
                    ..
                } if *attested == entry => Some((teller.clone(), posture.clone())),
                _ => None,
            })
            .collect()
    }

    /// This block's pending content appends to any member of `members`, as entry refs, skipping any in
    /// `exclude` — the read-your-writes tail of a live or history entry read.
    pub(super) fn pending_entries(
        &self,
        members: &BTreeSet<MemoryId>,
        exclude: &BTreeSet<EntryId>,
    ) -> Vec<EntryRef> {
        self.buffer
            .iter()
            .filter_map(|event| match event {
                EventPayload::MemoryContentAppended {
                    id,
                    entry_id,
                    text,
                    told_by,
                    visibility,
                    occurred_at,
                    ..
                } if members.contains(id) && !exclude.contains(entry_id) => Some(EntryRef {
                    entry_id: *entry_id,
                    text: text.clone(),
                    visibility: visibility.clone(),
                    teller: self.teller_label(told_by),
                    // A pending append carries only its founding attestation, so the read falls back
                    // to the lone teller until it commits and a fuller set materializes.
                    attesters: Vec::new(),
                    disputed: false,
                    occurred_at: occurred_at.clone(),
                    withheld: false,
                    stale: false,
                    retracted_reason: None,
                }),
                _ => None,
            })
            .collect()
    }

    /// Project an [`EntryView`] into an [`EntryRef`], resolving its teller to a readable label,
    /// marking it disputed when it is in the memory's set of unresolved-arbitration competing entries,
    /// and — when `withheld` — replacing its content with a stub so the confidence is not handed to a
    /// read whose present audience is not cleared to see it (see [`EntryRef::withheld`]).
    pub(super) fn entry_ref(
        &self,
        view: EntryView,
        disputed: &BTreeSet<EntryId>,
        withheld: bool,
        stale: bool,
        attesters: Vec<Teller>,
    ) -> EntryRef {
        EntryRef {
            disputed: disputed.contains(&view.entry_id),
            entry_id: view.entry_id,
            text: if withheld {
                WITHHELD_STUB.to_owned()
            } else {
                view.text
            },
            visibility: view.visibility,
            teller: self.teller_label(&view.told_by),
            // Resolved here, outside the graph lock the annotating read holds.
            attesters: attesters
                .iter()
                .map(|teller| self.teller_label(teller))
                .collect(),
            occurred_at: view.occurred_at,
            withheld,
            stale,
            retracted_reason: view.retracted_reason,
        }
    }

    /// Annotate each entry of a direct read with whether it is `withheld` and whether it is `stale`.
    ///
    /// *Withheld* applies the same [`visible`] predicate search does (resolving identity over the
    /// `same_as` class). Two deliberate carve-outs keep the agent's reach over its own memory intact:
    /// with no one present — a solo flush or maintenance pass — nothing is withheld; and the audience
    /// check ignores supersession (probing with `superseded_by` cleared), so `history` still shows a
    /// superseded entry yet still withholds one that was a confidence not for who is present.
    ///
    /// *Stale* is independent of who is present — it is a fact about the entry's age on a `High`
    /// volatility memory (spec §Recency and volatility) — so it is computed for every read, audience or
    /// not. It rides only an *unreplaced* entry, though: the marker reads "no newer entry", so a
    /// superseded entry (surfaced only by history, its successor beside it) must not carry it.
    pub(super) fn annotate(
        &self,
        graph: &Graph,
        id: MemoryId,
        entries: Vec<EntryView>,
    ) -> Result<Vec<AnnotatedEntry>, MemoryError> {
        let now = self.now();
        let memory = graph.memory_by_id(id)?;
        let volatility = memory
            .as_ref()
            .map(|memory| memory.volatility)
            .unwrap_or_default();
        let audience = !self.present_set.is_empty();
        let class_of = |mid| graph.class_id(mid).map(|class| class.unwrap_or(mid));
        entries
            .into_iter()
            .map(|entry| {
                let effective = entry.occurred_sort.unwrap_or(entry.asserted_at);
                // Only an unreplaced entry carries the marker: a superseded one (surfaced only by
                // history) has its newer version right there in the same list, so marking it "no newer
                // entry" would lie. A live read never reaches here with a superseded entry.
                let stale =
                    entry.superseded_by.is_none() && decay::is_stale(volatility, effective, now);
                // Probe with supersession cleared, matching the withheld check: history still surfaces
                // a superseded entry, and its attesters are its provenance.
                let mut probe = entry.clone();
                probe.superseded_by = None;
                let withheld = match (audience, &memory) {
                    (true, Some(memory)) => !visible(&probe, memory, &self.present_set, &class_of)?,
                    _ => false,
                };
                // With an audience present, keep only the attesters the audience may see, so a hidden
                // attestation leaves no residue; with no one present (a solo flush or maintenance
                // pass), the agent sees its whole record, matching the withheld carve-out. The agent
                // is skipped — the synthesizer of a consolidation replacement is not a source. The
                // tellers are resolved to labels in [`MemoryBlock::entry_ref`], off the graph lock this
                // read holds.
                let attester_tellers: Vec<Teller> = match (audience, &memory) {
                    (true, Some(memory)) => {
                        visible_attestations(&probe, memory, &self.present_set, &class_of)?
                            .into_iter()
                            .map(|attestation| attestation.teller.clone())
                            .collect()
                    }
                    _ => entry
                        .attestations
                        .iter()
                        .map(|attestation| attestation.teller.clone())
                        .collect(),
                };
                let attesters = attester_tellers
                    .into_iter()
                    .filter(|teller| !matches!(teller, Teller::Agent))
                    .collect();
                Ok(AnnotatedEntry {
                    entry,
                    withheld,
                    stale,
                    attesters,
                })
            })
            .collect()
    }

    /// Resolve an entry argument to the concrete [`EntryId`] a `supersede`/`retract` acts on. An entry
    /// handle already carries the full id, so [`EntrySelector::Id`] passes straight through; a string
    /// the agent typed ([`EntrySelector::Ref`]) is a full id or a unique prefix of one, resolved here
    /// against the memory's `same_as` class.
    ///
    /// Positional addressing ("the second entry") is deliberately *not* a form: an entry's position in
    /// a read shifts as entries land and supersede, so a stale ordinal would correct the wrong entry.
    /// Only a stable id — or a prefix of one — names an entry unambiguously, which is why every rendered
    /// entry line leads with its id.
    ///
    /// A full id resolves to itself (the live/historical check is the caller's, so a full id names an
    /// entry even once superseded, for `history`-driven work). A prefix is matched case-insensitively
    /// (the id renders uppercase) over the whole class's live-and-historical entries, and must be at
    /// least [`MIN_ENTRY_PREFIX`] characters: a unique match resolves, more than one is
    /// [`MemoryError::AmbiguousEntryPrefix`] listing the candidates, and none — or a too-short prefix —
    /// is [`MemoryError::UnknownEntry`].
    pub(super) fn resolve_entry_ref(
        &self,
        id: MemoryId,
        selector: EntrySelector,
    ) -> Result<EntryId, MemoryError> {
        let raw = match selector {
            EntrySelector::Id(entry) => return Ok(entry),
            EntrySelector::Ref(raw) => raw,
        };
        let raw = raw.trim();
        // A full id parses straight through; only a partial reference falls to prefix matching.
        if let Ok(ulid) = Ulid::from_string(raw) {
            return Ok(EntryId(ulid));
        }
        let needle = raw.to_ascii_uppercase();
        if needle.len() < MIN_ENTRY_PREFIX {
            return Err(MemoryError::UnknownEntry(raw.to_owned()));
        }
        let matches: Vec<(EntryId, String)> = self
            .class_entry_texts(id)?
            .into_iter()
            .filter(|(entry, _)| entry.0.to_string().starts_with(&needle))
            .collect();
        match matches.len() {
            0 => Err(MemoryError::UnknownEntry(raw.to_owned())),
            1 => Ok(matches.into_iter().next().expect("one match").0),
            _ => Err(MemoryError::AmbiguousEntryPrefix {
                prefix: raw.to_owned(),
                candidates: matches,
            }),
        }
    }

    /// Every entry of `id`'s `same_as` class as `(id, text)` — the committed live-and-historical
    /// entries plus this block's pending appends — the universe an entry-id prefix resolves against.
    /// A plain read: it touches nothing (the write it precedes records the class into the touched set).
    fn class_entry_texts(&self, id: MemoryId) -> Result<Vec<(EntryId, String)>, MemoryError> {
        let (members, mut entries) = {
            let graph = self.engine.graph.lock();
            let members: BTreeSet<MemoryId> =
                graph.class_members(id)?.into_iter().chain([id]).collect();
            let entries: Vec<(EntryId, String)> = graph
                .class_history(id)?
                .into_iter()
                .map(|entry| (entry.entry_id, entry.text))
                .collect();
            (members, entries)
        };
        for event in &self.buffer {
            if let EventPayload::MemoryContentAppended {
                id: entry_memory,
                entry_id,
                text,
                ..
            } = event
                && members.contains(entry_memory)
            {
                entries.push((*entry_id, text.clone()));
            }
        }
        Ok(entries)
    }

    /// Resolve a name to a memory id, consulting this block's pending creates/renames before the
    /// graph (read-your-writes).
    pub(super) fn resolve(&self, name: &str) -> Result<Option<MemoryId>, GraphError> {
        for event in &self.buffer {
            match event {
                EventPayload::MemoryCreated { id, name: created } if created.as_str() == name => {
                    return Ok(Some(*id));
                }
                EventPayload::MemoryRenamed { id, new_name, .. } if new_name.as_str() == name => {
                    return Ok(Some(*id));
                }
                _ => {}
            }
        }
        Ok(self
            .engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))?
            .map(|memory| memory.id))
    }

    /// The memory a class-level content write on `id` lands on. A handle that names any member of a
    /// merged `same_as` class addresses the whole class, so a class-level fact belongs on the class's
    /// primary stub — the id reads already resolve through — not on whichever member the caller happened
    /// to hold. This widens such a write to the primary; every other case returns `id` unchanged, so the
    /// write stays exactly where it was aimed:
    ///
    /// - A **platform-qualified** stub handle (`person/dave@discord`) redirects exactly like a bare one.
    ///   Stub handles are the default operands the agent holds — the present set, the brief, and search
    ///   all hand it the platform stub keyed by an opaque platform id — and a fact about a person belongs
    ///   on the person, not funnelled onto a single platform binding. A genuinely binding-scoped write is
    ///   the connector's own, recorded under connector provenance on a different path and exempted next,
    ///   never an agent write through here.
    /// - A **connector-authored** block (its events commit under
    ///   [`EventSource::PlatformConnector`](crate::event::EventSource::PlatformConnector)) is
    ///   never redirected: the connector maintains a participant's platform attributes (username, display
    ///   name) on the exact stub, holding the entry ids to supersede and retract, so its writes stay on
    ///   the stub they addressed. Keyed on the block's provenance, not the handle's shape.
    /// - A memory **created this block** is not yet committed, so it has no class: `class_id` finds
    ///   nothing and the write stays on the fresh stub (its class forms only once the create commits).
    /// - The **no-op** case where `id` already is its class's primary (an unmerged memory is its own
    ///   class) needs no redirect.
    /// - The **operator anchor** (`person/operator`) and `self` are never redirect targets. The anchor
    ///   holds no content by design (see [`MemoryBlock::guard_operator`]) and is the earliest-ULID
    ///   primary of the operator's class, so redirecting the operator's real `person/<name>` profile to
    ///   it would funnel every operator fact onto the anchor the guard forbids; `self` is never
    ///   class-merged. In both cases the write stays on the addressed profile.
    /// - A **soft-deleted** primary (a designated stub later deleted) is not a live write target, so the
    ///   write stays on the addressed member.
    ///
    /// It reads the committed `class_id` — the earliest-ULID designated member, or the earliest member
    /// overall when none is designated (the pass that designates primaries is separate machinery) — so
    /// the target is a deterministic function of the log, and it is the redirect target the write guards
    /// apply to, not the addressed handle.
    pub(super) fn class_write_target(&self, id: MemoryId) -> Result<MemoryId, MemoryError> {
        if self.connector_authored {
            return Ok(id);
        }
        let graph = self.engine.graph.lock();
        let Some(primary) = graph.class_id(id)? else {
            return Ok(id);
        };
        if primary == id
            || Some(primary) == self.operator_id
            || Some(primary) == self.self_id
            || graph.memory_by_id(primary)?.is_none()
        {
            return Ok(id);
        }
        Ok(primary)
    }

    /// A readable field of a memory by id — `name` or `description` — backing the handle metatable's
    /// lazy `handle.name` / `handle.description` accessors, so a memory handle minted from only an id (a
    /// calendar or link result) still reads its name. `name` honors this block's pending creates;
    /// `description` is graph-only (a just-created memory has none synthesized yet). An unknown field is
    /// `None`, so the metatable falls through to its methods.
    pub(crate) fn handle_field(
        &self,
        id: MemoryId,
        field: &str,
    ) -> Result<Option<String>, MemoryError> {
        match field {
            "name" => Ok(self.resolve_name(id)?.map(|name| name.as_str().to_owned())),
            "description" => Ok(self
                .engine
                .graph
                .lock()
                .memory_by_id(id)?
                .map(|memory| memory.description)),
            _ => Ok(None),
        }
    }

    /// Resolve a memory's name, honoring a pending `MemoryCreated` not yet projected — so a handle to a
    /// memory created this block reads its name (and the teller label for an entry attributed within the
    /// same block).
    pub(super) fn resolve_name(&self, id: MemoryId) -> Result<Option<MemoryName>, GraphError> {
        let pending = self.buffer.iter().find_map(|event| match event {
            EventPayload::MemoryCreated { id: created, name } if *created == id => {
                Some(name.clone())
            }
            _ => None,
        });
        match pending {
            Some(name) => Ok(Some(name)),
            None => Ok(self
                .engine
                .graph
                .lock()
                .memory_by_id(id)?
                .map(|memory| memory.name)),
        }
    }
}

/// Map a reconciled [`ForcedVisibility`] to the concrete [`Visibility`] a write is recorded at: an
/// explicit posture verbatim, and an `exclude` list into a [`Visibility::Exclude`] over the named
/// memory ids. The shared tail of both the content and link visibility resolvers.
fn forced_to_visibility(forced: ForcedVisibility) -> Visibility {
    match forced {
        ForcedVisibility::Choice(VisibilityChoice::Public) => Visibility::Public,
        ForcedVisibility::Choice(VisibilityChoice::Attributed) => Visibility::Attributed,
        ForcedVisibility::Choice(VisibilityChoice::Private) => Visibility::PrivateToTeller,
        ForcedVisibility::Exclude(ids) => Visibility::Exclude(ids),
    }
}
