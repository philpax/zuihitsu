//! Resolution and read-your-writes helpers: visibility resolution, pending-state overlays,
//! entry projection, and name resolution.

use std::collections::BTreeSet;

use crate::{
    decay,
    event::{EventPayload, Teller, Visibility},
    graph::{EntryView, Graph, GraphError},
    ids::{EntryId, MemoryId, MemoryName},
    memory::visibility::{default_visibility_named, subject_participant, visible},
};

use super::{EntryRef, MemoryBlock, MemoryError, VisibilityChoice, WITHHELD_STUB};

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
        explicit: Option<VisibilityChoice>,
    ) -> Result<Visibility, MemoryError> {
        if let Some(choice) = explicit {
            return Ok(match choice {
                VisibilityChoice::Public => Visibility::Public,
                VisibilityChoice::Attributed => Visibility::Attributed,
                VisibilityChoice::Private => Visibility::PrivateToTeller,
            });
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

    /// The entries this block has superseded but not yet committed — applied to the live reads so a
    /// correction's effect is visible within the block (read-your-writes).
    pub(super) fn pending_superseded(&self) -> BTreeSet<EntryId> {
        self.buffer
            .iter()
            .filter_map(|event| match event {
                EventPayload::MemorySuperseded { entry, .. } => Some(*entry),
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
                    disputed: false,
                    occurred_at: occurred_at.clone(),
                    withheld: false,
                    stale: false,
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
            occurred_at: view.occurred_at,
            withheld,
            stale,
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
    ) -> Result<Vec<(EntryView, bool, bool)>, MemoryError> {
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
                let withheld = match (audience, &memory) {
                    (true, Some(memory)) => {
                        let mut probe = entry.clone();
                        probe.superseded_by = None;
                        !visible(&probe, memory, &self.present_set, &class_of)?
                    }
                    _ => false,
                };
                Ok((entry, withheld, stale))
            })
            .collect()
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
