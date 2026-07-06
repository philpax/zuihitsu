//! The block-scoped seam ([`BlockApi`]), the per-block lock set, and the free helpers shared between
//! the lifecycle (`mod.rs`) and the Lua-table builders (`tables.rs`): lock acquisition and release,
//! handle minting, error routing, the `memory.search` runner, and value rendering.

use std::{collections::HashMap, sync::Arc};

use mlua::{Lua, LuaSerdeExt, Table, Value};
use parking_lot::Mutex;
use serde::Deserialize;
use tokio::sync::OwnedMutexGuard;
use ulid::Ulid;

use crate::{
    engine::{Engine, MemoryLocks},
    event::{TerminalCause, Visibility},
    graph::{GraphError, RelationView},
    ids::{EntryId, MemoryId},
    memory::{
        memory_block::{
            AppendOptions, EntryRef, LinkDirection, LinkRef, MemoryBlock, MemoryDetails,
            MemoryError,
        },
        search::{SalientRelation, SearchQuery, search},
    },
    settings::Settings,
    time::{MILLIS_PER_DAY, TemporalRef, civil_date_to_millis, format_occurrence},
    vocabulary::TagName,
};

use super::error::{
    ConcatError, HandleAssignmentError, HandleError, HandleKind, MemorySearchError,
    TemporalArgError,
};

/// The block-scoped handles every memory-API closure captures: the transaction (`block`), the
/// infrastructure-error slot (`infra`), the per-block lock set (`lock_set`), and the server-wide lock
/// registry (`manager`). Bundled so the install helpers pass one seam rather than four parallel
/// arguments, and the `'static` async closures clone one value. `Clone` clones the inner `Arc`s.
#[derive(Clone)]
pub(super) struct BlockApi {
    pub(super) block: Arc<Mutex<MemoryBlock>>,
    pub(super) infra: Arc<Mutex<Option<GraphError>>>,
    pub(super) lock_set: Arc<Mutex<LockSet>>,
    pub(super) manager: Arc<MemoryLocks>,
    /// What the block wrote with `print(...)`, accumulated across the script and folded into the
    /// block's agent-visible result. Without this, `print` output is lost, so an agent that inspects a
    /// query by printing it (rather than returning it) sees nothing come back — the recall failure mode.
    pub(super) printed: Arc<Mutex<String>>,
}

impl BlockApi {
    /// Acquire `id`'s lock (unless already held), holding the owned guard in the lock set to block end.
    pub(super) async fn lock(&self, id: MemoryId) {
        ensure_locked(&self.lock_set, &self.manager, [id]).await;
    }

    /// Acquire the locks for `ids` (skipping any already held) — the multi-memory operations (a link's
    /// two endpoints, a calendar query's whole result set).
    pub(super) async fn lock_all(&self, ids: impl IntoIterator<Item = MemoryId>) {
        ensure_locked(&self.lock_set, &self.manager, ids).await;
    }

    /// Lock the whole `same_as` class of `id` (plus `id` itself) before a traversing read, so a
    /// concurrent write to a sibling stub cannot tear the merged view (spec §Concurrency → class-wide
    /// locking). The class membership is read lock-free through the block; a graph failure routes to
    /// `infra`. The class boundary is read-then-locked, so a concurrent operator merge can shift it —
    /// an accepted edge the timeout backstops (a platform turn cannot merge).
    pub(super) async fn lock_class(&self, id: MemoryId) -> mlua::Result<()> {
        let members = self
            .block
            .lock()
            .class_members(id)
            .map_err(|error| route_error(error, &mut self.infra.lock()))?;
        ensure_locked(
            &self.lock_set,
            &self.manager,
            std::iter::once(id).chain(members),
        )
        .await;
        Ok(())
    }
}

/// The per-memory locks a block holds, keyed by memory and released together at block end (spec
/// §Concurrency → lifetime is the code block). The owned guards live here, not in the closures, so
/// [`release_locks`] can drop them deterministically at the end of `execute`.
#[derive(Default)]
pub(super) struct LockSet {
    held: HashMap<MemoryId, OwnedMutexGuard<()>>,
}

impl LockSet {
    fn holds(&self, id: MemoryId) -> bool {
        self.held.contains_key(&id)
    }

    fn insert(&mut self, id: MemoryId, guard: OwnedMutexGuard<()>) {
        self.held.insert(id, guard);
    }

    fn take(&mut self) -> Vec<OwnedMutexGuard<()>> {
        std::mem::take(&mut self.held).into_values().collect()
    }
}

/// Acquire the registry lock for each id not already held by `lock_set`, recording each owned guard.
/// The `lock_set` `parking_lot` guard is taken only to test membership and to insert, never held across
/// the acquire `.await`; the only long-held locks are the per-memory ones, so two blocks acquiring in
/// opposite orders deadlock only until the per-block timeout breaks and retries them (spec §Concurrency
/// → timeout-and-retry, not an ordering protocol). Within one block the calls are sequential (Lua runs
/// one operation at a time), so the membership test is race-free and never double-acquires an id.
async fn ensure_locked(
    lock_set: &Arc<Mutex<LockSet>>,
    manager: &Arc<MemoryLocks>,
    ids: impl IntoIterator<Item = MemoryId>,
) {
    for id in ids {
        if lock_set.lock().holds(id) {
            continue;
        }
        let guard = manager.acquire(id).await;
        lock_set.lock().insert(id, guard);
    }
}

/// Drain and drop the block's lock guards, releasing the per-memory locks so the next block (here or in
/// another conversation) can take them. The `'static` Lua closures still hold `Arc` clones of the
/// now-empty lock set, but no longer any guard — a leaked guard would deadlock the next block touching
/// that memory, so this is called on every exit path of `execute`.
pub(super) fn release_locks(lock_set: &Arc<Mutex<LockSet>>) {
    let guards = lock_set.lock().take();
    drop(guards);
}

/// The terminal cause for a block that blew its time budget: the budget in seconds, plus — when the
/// block exhausted its retries without an MCP call — the attempt count, so the give-up is auditable.
pub(super) fn timed_out_cause(budget: std::time::Duration, attempts: Option<u32>) -> TerminalCause {
    let secs = budget.as_secs();
    let message = match attempts {
        Some(attempts) => format!(
            "the block exceeded its time budget of {secs}s on each of {attempts} attempts and was aborted"
        ),
        None => format!("the block exceeded its time budget of {secs}s and was aborted"),
    };
    TerminalCause::Error(message)
}

/// Build a Lua handle table `{ id = "<ulid>" }` with the memory methods as its metatable index.
pub(super) fn make_handle(lua: &Lua, id: MemoryId, metatable: &Table) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", id.0.to_string())?;
    handle.set_metatable(Some(metatable.clone()))?;
    Ok(handle)
}

/// Build a relation result `{ name, inverse, from_card, to_card, symmetric, reflexive, description }`
/// backed by the relation metatable, so it prints readably. Cardinalities render lowercase, matching
/// the casing `links.register` accepts.
pub(super) fn make_relation_result(
    lua: &Lua,
    view: &RelationView,
    metatable: &Table,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("name", view.name.as_str())?;
    table.set("inverse", view.inverse.as_str())?;
    table.set("from_card", view.from_card.as_str().to_lowercase())?;
    table.set("to_card", view.to_card.as_str().to_lowercase())?;
    table.set("symmetric", view.symmetric)?;
    table.set("reflexive", view.reflexive)?;
    table.set("description", view.description.as_str())?;
    table.set_metatable(Some(metatable.clone()))?;
    Ok(table)
}

/// Wrap a list of memory ids as a Lua sequence of handles, in order — the `calendar.*` return shape.
pub(super) fn make_handle_list(
    lua: &Lua,
    ids: Vec<MemoryId>,
    metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        list.set(index + 1, make_handle(lua, id, metatable)?)?;
    }
    Ok(Value::Table(list))
}

/// Wrap a capped list of memory ids as a Lua sequence of handles — the `memory.list` return shape.
/// The value stays a plain sequence the agent can iterate (each element a handle, `handle.name`
/// readable), so `for _, m in ipairs(memory.list("person/")) do … end` works; the truncation note
/// rides only the *rendered* form, through the list metatable's `__tostring` reading the `more`
/// field this stores when matches were elided past the cap. So the returned value is unadorned data
/// while printing or returning it shows the `(+N more — narrow the prefix)` hint.
pub(super) fn make_capped_handle_list(
    lua: &Lua,
    ids: Vec<MemoryId>,
    more: usize,
    metatable: &Table,
    list_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        list.set(index + 1, make_handle(lua, id, metatable)?)?;
    }
    if more > 0 {
        list.set("more", more as i64)?;
    }
    list.set_metatable(Some(list_metatable.clone()))?;
    Ok(Value::Table(list))
}

/// Render a memory's whole record to the one string `mem:details` returns: a header line (its name,
/// its description, and a `formerly …` line when it has been renamed), the live entries under a count
/// header, every link in both directions, the applied tags, and the volatility — the sections joined by
/// blank lines. Entries and links render through the *same* handle rendering `mem:entries`/`mem:links`
/// use (each row minted as its handle and stringified through its metatable), so the record reads back
/// exactly as those readers show their rows — date, stale, disputed, visibility, and teller markers on
/// an entry; `relation → name` with a dated occurrence on a link. There is no entry cap: the render is
/// the whole record, which is what lets the agent conclude it holds nothing on a topic after one look.
pub(super) fn render_details(
    lua: &Lua,
    details: &MemoryDetails,
    entry_metatable: &Table,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<String> {
    let mut sections: Vec<String> = Vec::new();

    let mut header = details.name.clone();
    if !details.description.is_empty() {
        header.push_str(" — ");
        header.push_str(&details.description);
    }
    if !details.former_names.is_empty() {
        header.push_str(&format!("\nformerly {}", details.former_names.join(", ")));
    }
    sections.push(header);

    // The entries under a count header, each rendered as its own entry handle — the whole class read,
    // teller-private entries marked rather than omitted (this is the agent's own read).
    let count = details.entries.len();
    let mut entry_block = if count == 0 {
        "no entries".to_owned()
    } else {
        format!("{count} {}:", if count == 1 { "entry" } else { "entries" })
    };
    for entry in &details.entries {
        let handle = make_entry_handle(lua, entry, entry_metatable)?;
        entry_block.push('\n');
        entry_block.push_str(&render(lua, &Value::Table(handle)));
    }
    sections.push(entry_block);

    // Every link out of the merged identity in both directions, committed-only — the section is omitted
    // entirely when the memory has none.
    if !details.links.is_empty() {
        let mut link_block = String::from("links:");
        for link in &details.links {
            let handle = make_link_handle(lua, link, memory_metatable, link_metatable)?;
            link_block.push('\n');
            link_block.push_str(&render(lua, &Value::Table(handle)));
        }
        sections.push(link_block);
    }

    if !details.tags.is_empty() {
        let tags = details
            .tags
            .iter()
            .map(|tag| format!("#{}", tag.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        sections.push(format!("tags: {tags}"));
    }

    sections.push(format!("volatility: {}", details.volatility));

    Ok(sections.join("\n\n"))
}

/// Build a link result `{ relation, memory, name, direction, source }` backed by the link metatable,
/// so a link reader's list prints readably (`relation → name`) while each result keeps the far
/// memory as an actionable handle (`link.memory:append(...)`) and its provenance for the agent to weigh.
pub(super) fn make_link_handle(
    lua: &Lua,
    link: &LinkRef,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("relation", link.relation.as_str())?;
    table.set("memory", make_handle(lua, link.other, memory_metatable)?)?;
    table.set("name", link.other_name.as_str())?;
    table.set("direction", link_direction_label(link.direction))?;
    table.set("source", link.source.as_str_lowercase())?;
    // The teller who asserted the relationship, for a belief-bearing relation; absent (`nil`) for a
    // link with no teller behind it, like the adjudicated `same_as`.
    if let Some(told_by) = &link.told_by {
        table.set("told_by", told_by.as_str())?;
    }
    // The far memory's representative occurrence, when it holds a dated fact — the same tagged table
    // an entry or search hit carries (e.g. `link.occurred_at.day`), so a script reads a linked event's
    // date, and the metatable's `__tostring` renders it inline on the link line.
    if let Some(occurred_at) = &link.occurred_at {
        table.set("occurred_at", lua.to_value(occurred_at)?)?;
    }
    table.set_metatable(Some(link_metatable.clone()))?;
    Ok(table)
}

/// How many of a memory's links its rendered handle lists before eliding the rest — enough to reveal
/// a hub's shape (its events, its people) without flooding the transcript when a busy topic has many.
const NEIGHBORHOOD_CAP: usize = 8;

/// Render a memory's link neighborhood as the compact line its handle carries: each link as
/// `relation → name` (`←` for an incoming edge), with a dated far memory's occurrence appended as
/// `[when …]` (the same phrasing a search hit's date uses), capped at [`NEIGHBORHOOD_CAP`] with a
/// `(+N more)` note. A name-and-relation list, not the targets' content: it makes the spokes legible
/// at the hub so a recall follows them rather than relaying only the hub's own entries. Empty when the
/// memory has no links, so the caller omits the line entirely.
pub(super) fn render_neighborhood(links: &[LinkRef]) -> String {
    let mut rendered: Vec<String> = links
        .iter()
        .take(NEIGHBORHOOD_CAP)
        .map(render_link_summary)
        .collect();
    let elided = links.len().saturating_sub(NEIGHBORHOOD_CAP);
    if elided > 0 {
        rendered.push(format!("(+{elided} more)"));
    }
    rendered.join(", ")
}

/// One link on the neighborhood line: `relation → name` (or `←` for an incoming edge), plus the far
/// memory's occurrence as `[when …]` when it holds a dated fact.
fn render_link_summary(link: &LinkRef) -> String {
    let arrow = match link.direction {
        LinkDirection::Outgoing => "→",
        LinkDirection::Incoming => "←",
    };
    let mut summary = format!(
        "{} {arrow} {}",
        link.relation.as_str(),
        link.other_name.as_str()
    );
    if let Some(occurred_at) = &link.occurred_at {
        summary.push_str(&format!(" [when {}]", format_occurrence(occurred_at)));
    }
    summary
}

/// Render a search hit's salient relations as the compact segment its result line carries: each
/// relation as `relation → name` (`←` for an incoming edge), in the salience order (people first, then
/// recency), with a run of same-relation neighbours eliding the repeated label so
/// `participates_in ← person/maya, ← person/nadia` reads cleanly, and a trailing `(+N more)` when links
/// were elided past the cap. The same `relation → name` house style the neighborhood line uses, so a hit
/// passively reveals the cast already on the memory — the recognition signal that steers a search toward
/// reuse over a name-guessed duplicate. `None` when the hit carries no relations, so the caller omits the
/// segment.
pub(super) fn render_salient_relations(
    relations: &[SalientRelation],
    more: usize,
) -> Option<String> {
    if relations.is_empty() {
        return None;
    }
    let mut rendered: Vec<String> = Vec::with_capacity(relations.len() + 1);
    let mut previous: Option<&str> = None;
    for relation in relations {
        let arrow = match relation.direction {
            LinkDirection::Outgoing => "→",
            LinkDirection::Incoming => "←",
        };
        let name = relation.other_name.as_str();
        let segment = if previous == Some(relation.relation.as_str()) {
            format!("{arrow} {name}")
        } else {
            format!("{} {arrow} {name}", relation.relation.as_str())
        };
        rendered.push(segment);
        previous = Some(relation.relation.as_str());
    }
    if more > 0 {
        rendered.push(format!("(+{more} more)"));
    }
    Some(rendered.join(", "))
}

/// Wrap a list of link refs as a Lua sequence of link results, in order — the
/// `mem:outgoing()`/`incoming()`/`links()` return shape.
pub(super) fn make_link_handle_list(
    lua: &Lua,
    links: Vec<LinkRef>,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, link) in links.into_iter().enumerate() {
        list.set(
            index + 1,
            make_link_handle(lua, &link, memory_metatable, link_metatable)?,
        )?;
    }
    Ok(Value::Table(list))
}

/// Which way a link runs relative to the memory it was read from, as the agent-facing string a script
/// branches on — `outgoing` when the identity is the edge's source, `incoming` when its target.
fn link_direction_label(direction: LinkDirection) -> &'static str {
    match direction {
        LinkDirection::Outgoing => "outgoing",
        LinkDirection::Incoming => "incoming",
    }
}

pub(super) fn handle_id(handle: &Table) -> mlua::Result<MemoryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(MemoryId)
        .map_err(|source| HandleError::InvalidMemoryHandle { id, source }.into())
}

/// The `self` a `mem:*` handle method is invoked on. Extracting it through this newtype — rather than
/// a bare `Table` — is what turns a dot-call (`memory.append(...)`, which binds the first argument to
/// `self`) into the teachable colon hint: as the method's leftmost argument, `self` is converted
/// first, so a non-table `self` fails here (with [`HandleError::MethodCalledWithDot`]) before any
/// later argument's own type error can mask it. A colon call passes the handle table, which converts
/// cleanly; the method body then resolves its id through [`handle_id`].
pub(super) struct HandleSelf(pub(super) Table);

impl mlua::FromLua for HandleSelf {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::Table(handle) => Ok(HandleSelf(handle)),
            other => Err(HandleError::MethodCalledWithDot {
                type_name: other.type_name(),
            }
            .into()),
        }
    }
}

/// The `__newindex` guard shared by every read-only handle metatable (memory, entry, date, and search
/// result). A handle is a view, so assigning to a field silently did nothing before this — the
/// stale-date footgun. The guard raises a teachable error naming the operation that persists the
/// change instead, tailored for `occurred_at` (the traced slip) since a date lives on an entry, not a
/// handle field. It fires only for keys absent from the raw table, which is every agent-facing field
/// (they are read through `__index` or carried as data the metamethods read), so internal setup that
/// must write a raw field uses `raw_set` to bypass it.
pub(super) fn readonly_newindex(lua: &Lua, kind: HandleKind) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, (_, key, _): (Table, Value, Value)| {
        let field = lua
            .coerce_string(key)?
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let error = if field == "occurred_at" {
            HandleAssignmentError::OccurredAt { kind }
        } else {
            HandleAssignmentError::Other { kind, field }
        };
        Err::<(), mlua::Error>(error.into())
    })
}

/// Resolve a `:link`/`:unlink` target to its memory id. The target is normally a memory handle, but a
/// name string is accepted and looked up too — so the agent's natural call passing a name in place of
/// a handle works rather than failing the string-to-handle argument conversion, erroring, and rolling
/// the whole block back (silently dropping any co-located writes — the cause of lost sensitivity
/// markings). An unknown name is a clear error, not a silent miss.
pub(super) fn link_target_id(api: &BlockApi, other: Value) -> mlua::Result<MemoryId> {
    match other {
        Value::Table(handle) => handle_id(&handle),
        Value::String(name) => {
            let name = name.to_string_lossy();
            match api
                .block
                .lock()
                .get(&name)
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some((id, _)) => Ok(id),
                None => Err(HandleError::UnknownLinkTarget {
                    name: name.to_string(),
                }
                .into()),
            }
        }
        other => Err(HandleError::WrongLinkTargetType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Build an entry handle `{ id = "<ulid>", text = "..." }` backed by the entry metatable, so it
/// renders as its text (`__tostring` / `__concat`) yet stays addressable for `mem:supersede`.
pub(super) fn make_entry_handle(
    lua: &Lua,
    entry: &EntryRef,
    entry_metatable: &Table,
) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", entry.entry_id.0.to_string())?;
    handle.set("text", entry.text.as_str())?;
    // Carried so a read renders self-describingly (see the entry metatable's `__tostring`) and so a
    // script can branch on them: `entry.visibility` ("public"/"private"), `entry.told_by` (the teller),
    // `entry.disputed` (true when the fact is under an unresolved arbitration), and `entry.occurred_at`
    // (the occurrence as the *same* tagged table `append` accepts — `{ day = "…" }` etc. — so a read
    // round-trips to a write and a script can match on `entry.occurred_at.day`, not a string it has to
    // reparse; the metatable's `__tostring` renders it for display).
    handle.set("visibility", visibility_label(&entry.visibility))?;
    handle.set("told_by", entry.teller.as_str())?;
    handle.set("disputed", entry.disputed)?;
    // When set, `text` is already the withheld stub (the content never leaves the block); the flag
    // lets a script branch and lets the metatable render it as a withheld confidence, not bare text.
    handle.set("withheld", entry.withheld)?;
    // True when the fact has aged past usefulness on a high-volatility memory; the metatable renders a
    // `stale` segment so the agent hedges rather than asserting it as current.
    handle.set("stale", entry.stale)?;
    if let Some(occurred_at) = &entry.occurred_at {
        handle.set("occurred_at", lua.to_value(occurred_at)?)?;
    }
    handle.set_metatable(Some(entry_metatable.clone()))?;
    Ok(handle)
}

/// The agent-facing label for an entry's visibility — `public` for freely surfaceable, `attributed`
/// for an ordinary secondhand fact the agent should weigh as relayed, and `private` for a confidence
/// (`PrivateToTeller`/`Exclude`) that only resurfaces to its teller.
pub(super) fn visibility_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Attributed => "attributed",
        Visibility::PrivateToTeller | Visibility::Exclude(_) => "private",
    }
}

/// Wrap a list of entry refs as a Lua sequence of entry handles, in order — the `mem:entries()` /
/// `mem:history()` return shape.
pub(super) fn make_entry_handle_list(
    lua: &Lua,
    entries: Vec<EntryRef>,
    entry_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, entry) in entries.into_iter().enumerate() {
        list.set(index + 1, make_entry_handle(lua, &entry, entry_metatable)?)?;
    }
    Ok(Value::Table(list))
}

pub(super) fn entry_handle_id(handle: &Table) -> mlua::Result<EntryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(EntryId)
        .map_err(|source| HandleError::InvalidEntryHandle { id, source }.into())
}

/// Build a date handle `{ day = "YYYY-MM-DD" }` backed by the date metatable, so it renders as its ISO
/// day, carries calendar arithmetic (`:add_days` …), and — being a `{ day = … }` table — deserializes
/// straight into a `Day` occurrence when handed to `append` as `occurred_at`. So the agent computes a
/// date through operations the runtime executes, never as a string it works out in its head.
pub(super) fn make_date(lua: &Lua, iso: String, date_metatable: &Table) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("day", iso)?;
    handle.set_metatable(Some(date_metatable.clone()))?;
    Ok(handle)
}

/// Route a memory operation's error. A teachable violation (a duplicate name, an unknown relation)
/// becomes the Lua runtime error the agent sees as the block's terminal cause. A graph read failure
/// is infrastructure, not the agent's doing: it is stashed in the caller's `infra` slot for `execute`
/// to bubble up as a [`super::LuaError`], and the returned Lua error only serves to stop the script.
pub(super) fn route_error(error: MemoryError, infra: &mut Option<GraphError>) -> mlua::Error {
    match error {
        MemoryError::Graph(graph_error) => {
            *infra = Some(graph_error);
            mlua::Error::RuntimeError("internal graph error".to_owned())
        }
        teachable => mlua::Error::RuntimeError(teachable.to_string()),
    }
}

/// The default number of `memory.search` results when the caller gives no `limit`.
const DEFAULT_SEARCH_LIMIT: usize = 8;

/// The `opts` table `memory.search` accepts, deserialized from Lua.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct SearchOpts {
    namespace: Option<String>,
    tags: Vec<String>,
    limit: Option<usize>,
}

/// One ranked search result handed back to Lua as
/// `{ name, description, score, marker?, snippet?, occurred_at?, relations? }`. `snippet` is the matched
/// content that produced the hit, so a result stays legible even when the memory's description is stale
/// or empty; `occurred_at` is the memory's representative occurrence (the same tagged table `append`
/// takes), so a scheduled or dated fact's date rides on the result rather than surfacing only through
/// a separate `entries()` read; `relations` are the memory's most salient links (its cast), so the hit
/// passively carries who already participates in it — the recognition signal that steers a search
/// toward reusing the memory it found rather than minting a duplicate. `more_relations` counts the
/// salient links elided past the render cap, for the trailing `(+N more)` note.
pub(super) struct SearchRow {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) score: f32,
    pub(super) marker: Option<String>,
    pub(super) snippet: Option<String>,
    pub(super) occurred_at: Option<TemporalRef>,
    pub(super) relations: Vec<SalientRelation>,
    pub(super) more_relations: usize,
}

/// Run a `memory.search`: embed the query off every lock, read the search settings, then rank under a
/// brief graph + vector-index read lock (spec §Time → search scoring, §Visibility). The `Err` is the
/// agent-facing failure message — search is read-only, so a failure (no embedder, a transient embed or
/// backend error) terminates the block without corrupting anything.
pub(super) async fn run_memory_search(
    engine: &Engine,
    present_set: &[MemoryId],
    query: &str,
    opts: &SearchOpts,
) -> Result<Vec<SearchRow>, MemorySearchError> {
    // An empty or whitespace query has nothing to match on — reject it before the embedder is called,
    // so a degenerate "list everything in a namespace" search fails fast and teachably rather than
    // embedding the empty string and grinding the whole memory through the ranker.
    if query.trim().is_empty() {
        return Err(MemorySearchError::EmptyQuery);
    }
    let Some(retrieval) = &engine.retrieval else {
        return Err(MemorySearchError::NoRetrieval);
    };
    let started = std::time::Instant::now();
    let embedding = retrieval
        .embedder
        .embed(&[query.to_owned()])
        .await
        .map_err(MemorySearchError::Embed)?
        .into_iter()
        .next()
        .ok_or(MemorySearchError::NoVector)?;
    let settings = Settings::from_store(engine.store.lock().as_ref())
        .map_err(MemorySearchError::Settings)?
        .search;
    let now = engine.clock.now();
    let tags: Vec<TagName> = opts.tags.iter().map(|t| TagName::new(t)).collect();
    let request = SearchQuery {
        text: query,
        embedding: &embedding,
        namespace: opts.namespace.as_deref(),
        tags: &tags,
        present_set,
    };
    let limit = opts.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
    let hits = {
        // Graph before the vector index — the lock order `memory.search` and the indexer share. Both
        // are held only across the synchronous ranking, never an `.await`.
        let graph = engine.graph.lock();
        let vectors = retrieval.vectors.lock();
        search(&graph, vectors.as_ref(), &request, &settings, now, limit)
            .map_err(MemorySearchError::Search)?
    };
    crate::metrics::observe_search(started.elapsed());
    Ok(hits
        .into_iter()
        .map(|hit| SearchRow {
            name: hit.memory.name.as_str().to_owned(),
            description: hit.memory.description,
            score: hit.score,
            marker: hit.marker,
            snippet: hit.snippet,
            occurred_at: hit.occurred_at,
            relations: hit.relations,
            more_relations: hit.more_relations,
        })
        .collect())
}

/// Render a script's final value to the text the agent sees back (REPL-style).
/// Fold a block's `print` output and its final-value rendering into the one agent-visible result.
/// When nothing was printed this is just the value (the common `return …` case, unchanged). When the
/// block printed but returned nothing meaningful (a `for … print(x) end` loop, whose value is `nil`),
/// the printed output stands alone rather than being buried under a bare `nil`.
pub(super) fn combine_output(printed: String, value: String) -> String {
    let printed = printed.trim_end_matches('\n');
    if printed.is_empty() {
        value
    } else if value.is_empty() || value == "nil" {
        printed.to_owned()
    } else {
        format!("{printed}\n{value}")
    }
}

pub(super) fn render(lua: &Lua, value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_owned(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string_lossy(),
        // A table with a `__tostring` metamethod (an entry handle) renders through it, so a returned
        // entry — or a list of them — reads as its text rather than `<table>`. `coerce_string` would
        // not do this (it ignores `__tostring`), so call the `tostring` builtin, which honors it.
        Value::Table(t) => match tostring_via_metamethod(lua, value, t) {
            Some(text) => text,
            None => render_table(lua, value, t),
        },
        other => format!("<{}>", other.type_name()),
    }
}

/// Render a table through its `__tostring` metamethod, if it has one — the entry-handle case. `None`
/// for a plain table (no metamethod), so the caller falls back to the array rendering.
fn tostring_via_metamethod(lua: &Lua, value: &Value, table: &Table) -> Option<String> {
    let has_tostring = table
        .metatable()
        .is_some_and(|mt| mt.contains_key("__tostring").unwrap_or(false));
    if !has_tostring {
        return None;
    }
    lua.globals()
        .get::<mlua::Function>("tostring")
        .and_then(|tostring| tostring.call::<String>(value.clone()))
        .ok()
}

/// Render an undecorated table (no `__tostring`): a sequence as its elements joined by newlines (a
/// list of entry handles or search results), otherwise its structure via [`inspect_table`] — so a map
/// table the agent built, or one we have not given a `__tostring`, reads back as its fields rather than
/// an opaque `<table>` the model cannot act on.
fn render_table(lua: &Lua, value: &Value, table: &Table) -> String {
    let items: Vec<String> = table
        .clone()
        .sequence_values::<Value>()
        .filter_map(Result::ok)
        .map(|value| render(lua, &value))
        .collect();
    if items.is_empty() {
        inspect_table(lua, value)
    } else {
        items.join("\n")
    }
}

/// Pretty-print a table's structure through the vendored `inspect` global (loaded by
/// [`install_inspect`]). This is the fallback for a table with neither a `__tostring` nor a sequence
/// part; `inspect` only ever sees plain tables here, so its default options render clean
/// `{ key = value }` structure with no metatable noise. Falls back to the bare token if the global is
/// somehow absent.
fn inspect_table(lua: &Lua, value: &Value) -> String {
    lua.globals()
        .get::<mlua::Function>("inspect")
        .and_then(|inspect| inspect.call::<String>(value.clone()))
        .unwrap_or_else(|_| "<table>".to_owned())
}

/// The vendored `inspect.lua` pretty-printer (MIT-licensed, kikito/inspect.lua; see
/// `vendor/inspect.lua/VENDOR.md` for the exact commit). Loaded once per VM and exposed as the
/// `inspect` global, it backs [`render`]'s fallback for an undecorated table so the agent never
/// receives an opaque `<table>` it cannot read.
const INSPECT_LUA: &str = include_str!("../../../vendor/inspect.lua/inspect.lua");

/// Evaluate `inspect.lua` and bind its pretty-printer as the `inspect` global. The chunk ends in
/// `return inspect`, yielding the module — a *callable table* (it pretty-prints via a `__call`
/// metamethod). We bind its underlying `inspect.inspect` function rather than the table itself, so the
/// global is a plain function: `inspect(value)` still works from Lua, and the render fallback can fetch
/// it as an `mlua::Function` (a callable table is not one). Done once at VM construction, like the MCP
/// projection.
pub(super) fn install_inspect(lua: &Lua) -> mlua::Result<()> {
    let module: Table = lua.load(INSPECT_LUA).set_name("inspect.lua").eval()?;
    let inspect: mlua::Function = module.get("inspect")?;
    lua.globals().set("inspect", inspect)?;
    Ok(())
}

/// Wrap stock `table.concat` so a reader's handle list fails *legibly*. Stock Luau `table.concat` joins
/// only strings and numbers, so a handle list — `mem:entries()`, `hub:links()` — fails it with the
/// opaque "invalid value (table) at index … in table for 'concat'", one of the recurring recall
/// confusions. This shell keeps stock semantics exactly — it delegates the join to the original
/// function untouched, so an ordinary `table.concat(names, ",")` over a list the agent built joins as
/// before — and only rewrites the error, redirecting the two observed slips to teachable messages (see
/// [`ConcatError`]): the whole list argument being a reader *method* rather than its result, and a
/// handle list, which now points at string interpolation (a backtick string stringifies a handle) as
/// the way to compose text from entries, links, and dates. Installed before `Lua::sandbox(true)` freezes
/// the `table` library read-only, so the override is part of the frozen surface.
pub(super) fn install_table_concat(lua: &Lua) -> mlua::Result<()> {
    let table_lib: Table = lua.globals().get("table")?;
    let stock: mlua::Function = table_lib.get("concat")?;
    table_lib.set(
        "concat",
        lua.create_function(move |_, args: mlua::Variadic<Value>| {
            let list_type = args.first().map(Value::type_name).unwrap_or("nil");
            match stock.call::<Value>(args) {
                Ok(joined) => Ok(joined),
                // A table that stock concat rejected holds a non-joinable element (a handle list);
                // any other first argument is not a list at all (a reader method, most often).
                Err(_) if list_type == "table" => Err(ConcatError::NonJoinable.into()),
                Err(_) => Err(ConcatError::NotAList {
                    type_name: list_type,
                }
                .into()),
            }
        })?,
    )?;
    Ok(())
}

/// Render a value to its text for entry-handle `__concat`: an entry handle yields its `text`; any
/// other value coerces as Lua's `tostring` would (strings and numbers directly, otherwise empty).
pub(super) fn value_text(lua: &Lua, value: &Value) -> mlua::Result<String> {
    if let Value::Table(table) = value
        && let Ok(text) = table.get::<String>("text")
    {
        return Ok(text);
    }
    Ok(lua
        .coerce_string(value.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

/// The `__concat` metamethod shared by the read-surface handles (a memory, a link result, a search
/// result): each operand renders through its own `__tostring`, so `"Topic: " .. topic` and
/// `"- " .. link` compose the same text printing already shows, rather than erroring as a bare
/// table — the join the agent actually writes when assembling a reply. A plain string or number
/// operand coerces as Lua's `tostring` would.
pub(super) fn concat_via_tostring(lua: &Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(|lua, (left, right): (Value, Value)| {
        Ok(format!(
            "{}{}",
            tostring_text(lua, &left)?,
            tostring_text(lua, &right)?
        ))
    })
}

/// One `__concat` operand's text: a table with a `__tostring` renders through it; everything else
/// coerces as Lua's `tostring` would (strings and numbers directly, otherwise empty).
fn tostring_text(lua: &Lua, value: &Value) -> mlua::Result<String> {
    if let Value::Table(table) = value
        && let Some(text) = tostring_via_metamethod(lua, value, table)
    {
        return Ok(text);
    }
    Ok(lua
        .coerce_string(value.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

/// Render a value for a date handle's `__concat`: a date handle (a `{ day = "…" }` table) yields its
/// ISO day, and any other operand coerces as Lua's `tostring` would — so both `"on " .. friday` and
/// `friday .. " it is"` read the date while the surrounding text coerces normally.
pub(super) fn date_text(lua: &Lua, value: &Value) -> mlua::Result<String> {
    if let Value::Table(table) = value
        && let Ok(day) = table.get::<String>("day")
    {
        return Ok(day);
    }
    Ok(lua
        .coerce_string(value.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

/// The ISO `YYYY-MM-DD` day a temporal argument names: a date object (a `{ day = "…" }` table) yields
/// its `day`, a string is taken verbatim, and anything else is a teachable [`TemporalArgError`]. Shared
/// by `calendar.on` and the `occurred_at` normalization so a date object stands in for a date string
/// wherever a single day is wanted, without validating the string itself (the caller's block does that).
pub(super) fn day_string(value: &Value) -> mlua::Result<String> {
    match value {
        Value::String(day) => Ok(day.to_string_lossy()),
        Value::Table(table) => match table.get::<Option<String>>("day")? {
            Some(day) => Ok(day),
            None => Err(TemporalArgError::NotADate { type_name: "table" }.into()),
        },
        other => Err(TemporalArgError::NotADate {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Deserialize a Lua `opts` table into [`AppendOptions`], first normalizing any date handles inside its
/// `occurred_at` so a date object stands in for a `"YYYY-MM-DD"` string wherever a day is wanted. This
/// is the single seam every `occurred_at` taker — `<memory>:append`, `<memory>:revise`, and
/// `memory.create` — passes through, so accepting a date handle is decided once here rather than at each
/// call site; a future taker of the tagged table inherits it for free. `nil` opts yield `None`.
pub(super) fn append_options_from_lua(
    lua: &Lua,
    opts: Value,
) -> mlua::Result<Option<AppendOptions>> {
    if opts.is_nil() {
        return Ok(None);
    }
    if let Value::Table(table) = &opts
        && let Value::Table(occurred) = table.get::<Value>("occurred_at")?
    {
        normalize_temporal(&occurred)?;
    }
    Ok(Some(lua.from_value(opts)?))
}

/// Rewrite date-shaped values inside an `occurred_at` tagged table into the primitives its
/// [`TemporalRef`] deserialization expects, in place, so a day named as a date object *or* a bare
/// `"YYYY-MM-DD"` string stands in wherever a millisecond timestamp is wanted:
/// - a `{ day = <date object> }` field becomes `{ day = "…" }`;
/// - a range's `start`/`end` — a date object or a date string — becomes the day's bounding instant (its
///   first millisecond for `start`, its last for `end`, so a range from Monday to Friday spans all of
///   Friday);
/// - an `instant` given as a date object or date string becomes the day's first millisecond.
///
/// A position already holding a primitive is left untouched, so a millisecond count passes through. The
/// value itself being a date handle needs no rewrite — a date handle *is* a `{ day = "…" }` table, so it
/// already deserializes as a `Day`.
fn normalize_temporal(occurred: &Table) -> mlua::Result<()> {
    if let day @ Value::Table(_) = occurred.get::<Value>("day")? {
        occurred.set("day", day_string(&day)?)?;
    }
    if let Value::Table(range) = occurred.get::<Value>("range")? {
        coerce_range_bound(&range, "start", DayBound::Start)?;
        coerce_range_bound(&range, "end", DayBound::End)?;
    }
    let instant = occurred.get::<Value>("instant")?;
    if matches!(instant, Value::Table(_) | Value::String(_)) {
        occurred.set("instant", day_bound_millis(&instant, DayBound::Start)?)?;
    }
    Ok(())
}

/// Which instant of a day a millisecond-typed position resolves to when a date stands in for it: the
/// day's first millisecond for a `Start`, its last for an `End`, so a range covers the whole of both
/// boundary days and a bare instant lands at the start of its day.
enum DayBound {
    Start,
    End,
}

/// Replace a range endpoint given as a date object or a `"YYYY-MM-DD"` string with the day's bounding
/// instant in epoch milliseconds; a primitive already there (a millisecond count) is left untouched.
fn coerce_range_bound(range: &Table, key: &str, bound: DayBound) -> mlua::Result<()> {
    let endpoint = range.get::<Value>(key)?;
    if matches!(endpoint, Value::Table(_) | Value::String(_)) {
        range.set(key, day_bound_millis(&endpoint, bound)?)?;
    }
    Ok(())
}

/// Resolve a date object or `"YYYY-MM-DD"` string to one of its day's bounding instants in epoch
/// milliseconds — the shared coercion behind a range endpoint and a bare `instant`. An unparseable day
/// is a teachable [`TemporalArgError::InvalidDay`].
fn day_bound_millis(value: &Value, bound: DayBound) -> mlua::Result<i64> {
    let day = day_string(value)?;
    let midnight = civil_date_to_millis(&day).ok_or(TemporalArgError::InvalidDay { input: day })?;
    Ok(match bound {
        DayBound::Start => midnight,
        DayBound::End => midnight + MILLIS_PER_DAY - 1,
    })
}
