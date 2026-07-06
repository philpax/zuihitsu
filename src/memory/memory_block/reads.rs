//! Content read operations: resolving names, listing entries and history, and class membership.

use std::collections::BTreeSet;

use crate::{
    event::{EventPayload, Volatility},
    graph::EntryView,
    ids::{EntryId, MemoryId, MemoryName},
};

use super::{EntryRef, MemoryBlock, MemoryDetails, MemoryError};

impl MemoryBlock {
    /// Resolve a name to a memory id, or `None`, for `memory.get` — touches the result so it enters
    /// the lock set.
    /// Resolve a name to a memory for `memory.get`, returning the id and whether it matched a *former*
    /// name (an alias of a renamed memory) rather than a current one. A current name always wins; only
    /// when none holds the name does an old name resolve, flagged so the agent answers under the
    /// current handle and recognizes the person rather than treating the old name as a stranger (spec
    /// §Identity → Renaming). The looked-up result is touched, like every read.
    pub fn get(&mut self, name: &str) -> Result<Option<(MemoryId, bool)>, MemoryError> {
        if let Some(id) = self.resolve(name)? {
            self.touched.insert(id);
            return Ok(Some((id, false)));
        }
        if let Some(id) = self
            .engine
            .graph
            .lock()
            .memory_id_for_former_name(MemoryName::new(name))?
        {
            self.touched.insert(id);
            return Ok(Some((id, true)));
        }
        Ok(None)
    }

    /// The memory's live content entries: its whole `same_as` class from the graph plus this block's
    /// pending appends, minus any superseded this block (read-your-writes). A traversing read, so it
    /// touches every class member, not just `id`. Each entry is addressable (by id) so the agent can
    /// hand one to [`MemoryBlock::supersede`].
    pub fn entries(&mut self, id: MemoryId) -> Result<Vec<EntryRef>, MemoryError> {
        // A supersession buffered this block (not yet committed) must hide its target from this live
        // read too, so the agent sees the effect of a correction it just made.
        let pending_superseded = self.pending_superseded();
        let (members, annotated, disputed) = {
            let graph = self.engine.graph.lock();
            let members = graph.class_members(id)?;
            let disputed = graph.disputed_entries(id)?;
            let live: Vec<EntryView> = graph
                .class_entries(id)?
                .into_iter()
                .filter(|entry| !pending_superseded.contains(&entry.entry_id))
                .collect();
            let annotated = self.annotate(&graph, id, live)?;
            (members, annotated, disputed)
        };
        let members = self.touch_class(id, members);
        let mut refs: Vec<EntryRef> = annotated
            .into_iter()
            .map(|(entry, withheld, stale)| self.entry_ref(entry, &disputed, withheld, stale))
            .collect();
        refs.extend(self.pending_entries(&members, &pending_superseded));
        Ok(refs)
    }

    /// The memory's entries including superseded ones, oldest first — the agent's `mem:history()` view
    /// (spec §Per-memory history), the read where history is the point and the live filter is bypassed.
    /// Like [`MemoryBlock::entries`], a class-traversing read over the graph plus this block's pending
    /// appends; pending supersessions are *not* applied, since history keeps the superseded entries.
    pub fn history(&mut self, id: MemoryId) -> Result<Vec<EntryRef>, MemoryError> {
        let (members, annotated, disputed) = {
            let graph = self.engine.graph.lock();
            let members = graph.class_members(id)?;
            let disputed = graph.disputed_entries(id)?;
            let annotated = self.annotate(&graph, id, graph.class_history(id)?)?;
            (members, annotated, disputed)
        };
        let members = self.touch_class(id, members);
        let mut refs: Vec<EntryRef> = annotated
            .into_iter()
            .map(|(entry, withheld, stale)| self.entry_ref(entry, &disputed, withheld, stale))
            .collect();
        refs.extend(self.pending_entries(&members, &BTreeSet::new()));
        Ok(refs)
    }

    /// The memory's whole record in one read — the data behind `mem:details`. Gathers the header
    /// (current name, description, and any former handles), the live entries and every link across the
    /// merged identity, the applied tags, and the volatility, so the agent reads the complete record at
    /// a glance and can honestly conclude absence after one look. A class-traversing read (through
    /// [`MemoryBlock::entries`] and [`MemoryBlock::links`]), so it touches the whole class. Committed-only
    /// for the links, matching the link readers; the entries carry this block's pending writes like a
    /// direct [`MemoryBlock::entries`] read. The name honors a pending create, so details on a
    /// just-created memory still reads its name; description, tags, and volatility come from the graph
    /// projection (a pending create has none of those synthesized yet).
    pub fn details(&mut self, id: MemoryId) -> Result<MemoryDetails, MemoryError> {
        let view = self.engine.graph.lock().memory_by_id(id)?;
        let name = self
            .resolve_name(id)?
            .map(|name| name.as_str().to_owned())
            .or_else(|| view.as_ref().map(|memory| memory.name.as_str().to_owned()))
            .unwrap_or_default();
        let (description, tags, volatility) = match view {
            Some(memory) => (memory.description, memory.tags, memory.volatility),
            None => (String::new(), Vec::new(), Volatility::default()),
        };
        let former_names = self.former_names(id)?;
        let entries = self.entries(id)?;
        let links = self.links(id)?;
        Ok(MemoryDetails {
            name,
            description,
            former_names,
            entries,
            links,
            tags,
            volatility,
        })
    }

    /// The ids of every live memory whose name begins with `prefix`, ordered by name — the read behind
    /// `memory.list`, the handle-discovery-by-stem lookup. A committed-only graph read (like the link
    /// readers and search): a memory created but not yet committed this block does not list. The prefix
    /// is matched literally, its LIKE metacharacters escaped in the graph, so a `%` in the stem does not
    /// wildcard. Returns every match uncapped; the Lua layer caps the list and notes the remainder.
    pub fn list_by_prefix(&self, prefix: &str) -> Result<Vec<MemoryId>, MemoryError> {
        Ok(self
            .engine
            .graph
            .lock()
            .memory_ids_with_name_prefix(prefix)?)
    }

    /// The live members of `id`'s `same_as` class (including `id`), for the Lua lock layer to acquire
    /// the whole class before a traversing read (spec §Concurrency → class-wide locking). A lock-free
    /// read returning an owned list: it touches nothing itself — the traversing read it precedes records
    /// the class into the touched set — and the graph guard is released before it returns.
    pub fn class_members(&self, id: MemoryId) -> Result<Vec<MemoryId>, MemoryError> {
        Ok(self.engine.graph.lock().class_members(id)?)
    }

    /// The handles a memory used to go by, most recent first — empty unless it has been renamed. Surfaced
    /// on a `memory.get` handle so the agent connects a renamed person's old-name content to the same
    /// person under their current handle (spec §Identity → Renaming).
    pub fn former_names(&self, id: MemoryId) -> Result<Vec<String>, MemoryError> {
        Ok(self
            .engine
            .graph
            .lock()
            .former_names(id)?
            .into_iter()
            .map(|name| name.as_str().to_owned())
            .collect())
    }

    /// The [`EntryRef`] for an entry just appended this block (found in the buffer) — so `mem:append`
    /// can hand back a handle that renders with the same visibility and teller a read would show.
    pub fn entry_ref_by_id(&self, entry_id: EntryId) -> Option<EntryRef> {
        self.buffer.iter().find_map(|event| match event {
            EventPayload::MemoryContentAppended {
                entry_id: appended,
                text,
                told_by,
                visibility,
                occurred_at,
                ..
            } if *appended == entry_id => Some(EntryRef {
                entry_id: *appended,
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
    }
}
