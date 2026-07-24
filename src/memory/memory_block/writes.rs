//! Content write operations: create, rename, append, supersede, revise, and set volatility.

use crate::{
    event::{EventPayload, Teller, Visibility},
    graph::GraphError,
    ids::{EntryId, MemoryId, MemoryName},
    memory::{
        memory_block::{
            AppendOptions, Authority, EntrySelector, MemoryBlock, MemoryError,
            reconcile_forced_visibility, suggest::most_similar,
        },
        visibility::subject_participant,
    },
    time::TemporalRef,
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
pub(super) fn entry_teller(opts: &AppendOptions, speaker: &Teller) -> Teller {
    opts.told_by.clone().unwrap_or_else(|| {
        if opts.by_agent {
            Teller::Agent
        } else {
            speaker.clone()
        }
    })
}
