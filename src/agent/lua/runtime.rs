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
    event::{LinkSource, TerminalCause, Visibility},
    graph::{GraphError, RelationView},
    ids::{EntryId, MemoryId},
    memory::{
        memory_block::{EntryRef, LinkDirection, LinkRef, MemoryBlock, MemoryError},
        search::{SearchQuery, search},
    },
    settings::Settings,
    vocabulary::TagName,
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

/// Build a relation result `{ name, inverse, from_card, to_card, symmetric, reflexive }` backed by the
/// relation metatable, so it prints readably. Cardinalities render lowercase, matching the casing
/// `links.register` accepts.
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
    table.set("source", link_source_label(link.source))?;
    // The teller who asserted the relationship, for a belief-bearing relation; absent (`nil`) for a
    // link with no teller behind it, like the adjudicated `same_as`.
    if let Some(told_by) = &link.told_by {
        table.set("told_by", told_by.as_str())?;
    }
    table.set_metatable(Some(link_metatable.clone()))?;
    Ok(table)
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

/// A link's provenance, lowercased to match the entry teller register: `agent` for the agent's own
/// link, `operator` for one asserted from the console, `adjudicated` for a merge-pass `same_as`.
fn link_source_label(source: LinkSource) -> &'static str {
    match source {
        LinkSource::Agent => "agent",
        LinkSource::Operator => "operator",
        LinkSource::Adjudicated => "adjudicated",
    }
}

pub(super) fn handle_id(handle: &Table) -> mlua::Result<MemoryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(MemoryId)
        .map_err(|e| mlua::Error::RuntimeError(format!("invalid memory handle id {id:?}: {e}")))
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
                None => Err(mlua::Error::RuntimeError(format!(
                    "link target \"{name}\" is not a known memory — pass a handle from memory.get or \
                     memory.create, or an existing memory's name"
                ))),
            }
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "link target must be a memory handle (from memory.get/create) or a memory name, got {}",
            other.type_name()
        ))),
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
        .map_err(|e| mlua::Error::RuntimeError(format!("invalid entry handle id {id:?}: {e}")))
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

/// One ranked search result handed back to Lua as `{ name, description, score, marker? }`.
pub(super) struct SearchRow {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) score: f32,
    pub(super) marker: Option<String>,
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
) -> Result<Vec<SearchRow>, String> {
    let Some(retrieval) = &engine.retrieval else {
        return Err(
            "memory.search is unavailable on this instance (no embedding endpoint configured)"
                .to_owned(),
        );
    };
    let embedding = retrieval
        .embedder
        .embed(&[query.to_owned()])
        .await
        .map_err(|error| format!("memory.search: embedding the query failed: {error}"))?
        .into_iter()
        .next()
        .ok_or_else(|| "memory.search: the embedder returned no vector".to_owned())?;
    let settings = Settings::from_store(engine.store.lock().as_ref())
        .map_err(|error| format!("memory.search: {error}"))?
        .search;
    let now = engine.clock.now();
    let tags: Vec<TagName> = opts.tags.iter().map(TagName::new).collect();
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
            .map_err(|error| format!("memory.search: {error}"))?
    };
    Ok(hits
        .into_iter()
        .map(|hit| SearchRow {
            name: hit.memory.name.as_str().to_owned(),
            description: hit.memory.description,
            score: hit.score,
            marker: hit.marker,
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
