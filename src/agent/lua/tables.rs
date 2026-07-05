//! The free-function builders that mint the per-block Lua globals, their handle metatables, and the
//! `mem:*` handle methods. These translate script calls into [`MemoryBlock`] transaction calls over the
//! shared [`BlockApi`] seam; they never touch the buffer, the events, or the visibility rules directly.

use mlua::{Lua, LuaSerdeExt, Table, Value};
use ulid::Ulid;

use crate::{
    InstanceFeatures,
    agent::turn::{ResolvedTurn, TurnResolution, TurnWindow, resolve_turn},
    event::TurnRole,
    ids::{MemoryName, TurnId},
    memory::memory_block::{LinkDirection, RelationSpec},
    time,
    vocabulary::{RelationName, TagName},
};

use super::{
    error::{BlockConsistencyError, CalendarError, HandleKind, TurnResolveError},
    runtime::{
        BlockApi, HandleSelf, SearchOpts, append_options_from_lua, concat_via_tostring, date_text,
        day_string, entry_handle_id, handle_id, link_target_id, make_date, make_entry_handle,
        make_entry_handle_list, make_handle, make_handle_list, make_link_handle_list,
        make_relation_result, readonly_newindex, render, render_neighborhood,
        render_salient_relations, route_error, run_memory_search, value_text,
    },
};

/// How many turns before and after the focal turn `convo.turn` includes in its window — a few on
/// each side, enough to place the linked moment in its immediate exchange without replaying the room.
const TURN_WINDOW_BEFORE: usize = 3;
const TURN_WINDOW_AFTER: usize = 3;

/// Install the per-block memory API as `'static` async Lua functions over the shared [`BlockApi`]
/// seam. Before its operation, each function acquires the lock on every memory it touches and holds
/// the owned guard (in `api.lock_set`) to block end, so a concurrent block in another conversation
/// serializes on a shared memory (spec §Concurrency). A graph-read failure is routed to `api.infra`
/// (infrastructure, bubbled up); a teachable violation becomes the Lua runtime error the agent sees.
/// The handle `metatable`/`methods` tables back every minted memory handle. The registration is
/// split table by table so each group stays legible.
///
/// `features` gates which functions are installed: a disabled feature's methods and module tables
/// are simply not installed, so calling them yields the standard Lua "attempt to call a nil value"
/// error — a teachable failure the agent sees and adapts to. This is the first of the three gates
/// (Lua registration, API reference, scaffold) that must stay in lockstep.
pub(super) fn install_block_api(
    lua: &Lua,
    api: &BlockApi,
    methods: &Table,
    metatable: &Table,
    entry_metatable: &Table,
    features: &InstanceFeatures,
) -> mlua::Result<()> {
    let link_metatable = link_result_metatable(lua)?;
    install_handle_methods(
        lua,
        api,
        methods,
        metatable,
        entry_metatable,
        &link_metatable,
        features,
    )?;
    // A memory handle reads `handle.name` and `handle.description` lazily from its id, so a handle
    // minted from only an id — a `calendar.*` or relation result — still reads its name, not just
    // one the agent already named via `memory.get`. Any other key dispatches to the methods table
    // (`handle:append`, `handle:entries`, …). Without this a script iterating calendar results and
    // reading `m.name` got nil and concluded the calendar was empty.
    metatable.set("__index", {
        let methods = methods.clone();
        let api = api.clone();
        lua.create_function(move |lua, (handle, key): (Table, String)| {
            if key == "name" || key == "description" {
                let id = handle_id(&handle)?;
                let field = api
                    .block
                    .lock()
                    .handle_field(id, &key)
                    .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                return Ok(match field {
                    Some(text) => Value::String(lua.create_string(&text)?),
                    None => Value::Nil,
                });
            }
            methods.get::<Value>(key)
        })?
    })?;
    // A memory handle is a read-only view: assigning to a field (`m.occurred_at = ...`) was a silent
    // no-op that misled the agent into thinking a change landed. The guard raises a teachable error
    // naming the persisting operation instead. Internal setup writes raw fields with `raw_set` to
    // bypass it (see `resolve_existing_handle`).
    metatable.set("__newindex", readonly_newindex(lua, HandleKind::Memory)?)?;
    // `"Topic: " .. topic` composes the handle's rendered text — the join the agent writes when
    // assembling a reply — rather than erroring as a bare table.
    metatable.set("__concat", concat_via_tostring(lua)?)?;
    // A memory handle renders self-describingly: its name, its description, and — for a handle minted
    // by `memory.get`/`memory.get_or_create`, which precompute it (see `resolve_existing_handle`) — a
    // compact `links:` line naming its neighborhood (each link as `relation → name`, a dated target's
    // occurrence appended). So printing or returning a topic hub reveals at a glance that its decisions
    // live one link away on the spokes, rather than the agent reading only the hub's own entries and
    // dropping a fact that sits on a linked event. `name` and `description` read lazily through
    // `__index`; `neighborhood` is a raw field present only on the precomputed handles.
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| {
            let name = this.get::<Option<String>>("name")?.unwrap_or_default();
            let mut line = name;
            if let Some(description) = this
                .get::<Option<String>>("description")?
                .filter(|d| !d.is_empty())
            {
                line.push_str(" — ");
                line.push_str(&description);
            }
            if let Some(neighborhood) = this
                .get::<Option<String>>("neighborhood")?
                .filter(|n| !n.is_empty())
            {
                line.push_str("\n  links: ");
                line.push_str(&neighborhood);
            }
            Ok(line)
        })?,
    )?;
    let globals = lua.globals();
    // `print(...)` captures into the block's output buffer (rendered the same way returned values
    // are), so the agent sees what it prints fed back — Lua's default `print` writes to a process
    // stdout the model never reads. Tab-separated args, newline-terminated, matching Lua semantics.
    globals.set(
        "print",
        lua.create_function({
            let printed = api.printed.clone();
            move |lua, args: mlua::Variadic<Value>| {
                let mut buffer = printed.lock();
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        buffer.push('\t');
                    }
                    buffer.push_str(&render(lua, arg));
                }
                buffer.push('\n');
                Ok(())
            }
        })?,
    )?;
    globals.set("memory", memory_table(lua, api, metatable)?)?;
    globals.set("block", block_table(lua, api)?)?;
    globals.set("context", context_table(lua, api, metatable)?)?;
    // The calendar, tags, and links module tables are installed only when their feature is on;
    // a disabled module is simply absent (nil), so calling through it is a teachable nil-call
    // error rather than a silent no-op.
    if features.calendar {
        globals.set("calendar", calendar_table(lua, api, metatable)?)?;
    }
    if features.tagging {
        globals.set("tags", tags_table(lua, api)?)?;
    }
    if features.linking {
        globals.set("links", links_table(lua, api)?)?;
    }
    if features.transcripts {
        globals.set("convo", convo_table(lua, api)?)?;
    }
    Ok(())
}

/// The `mem:*` handle methods (`append`, `entries`, `history`, `supersede`, `revise`, `link`,
/// `unlink`) on
/// the metatable's `methods` table. Each acts on the handle passed as `this`. `entry_metatable`
/// backs the entry handles the content reads and `append` return.
///
/// `features` gates the linking (`:link`, `:unlink`, `:outgoing`, `:incoming`, `:links`),
/// merging (`:propose_merge`), and tagging (`:tag`, `:untag`) methods. Memory methods
/// (`:append`, `:supersede`, `:revise`, `:set_volatility`, `:rename`) are always installed.
fn install_handle_methods(
    lua: &Lua,
    api: &BlockApi,
    methods: &Table,
    memory_metatable: &Table,
    entry_metatable: &Table,
    link_metatable: &Table,
    features: &InstanceFeatures,
) -> mlua::Result<()> {
    // mem:append(text[, opts]) — `opts` is the typed override struct, deserialized from the table.
    // Locks the target memory before writing it. Returns the new entry as an addressable handle.
    methods.set(
        "append",
        lua.create_async_function({
            let api = api.clone();
            let entry_metatable = entry_metatable.clone();
            move |lua, (this, text, opts): (HandleSelf, String, Value)| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    api.lock(id).await;
                    let opts = append_options_from_lua(&lua, opts)?.unwrap_or_default();
                    let entry = {
                        let mut block = api.block.lock();
                        let entry_id = block
                            .append(id, &text, opts)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        block.entry_ref_by_id(entry_id)
                    };
                    let entry = entry.ok_or(BlockConsistencyError::AppendedEntryMissing)?;
                    make_entry_handle(&lua, &entry, &entry_metatable)
                }
            }
        })?,
    )?;

    // mem:entries() — the memory's live entries across its merged identity plus pending writes,
    // each an addressable entry handle that renders as its text. A traversing read, so it locks the
    // whole `same_as` class before reading (spec §Concurrency → class-wide locking).
    methods.set(
        "entries",
        lua.create_async_function({
            let api = api.clone();
            let entry_metatable = entry_metatable.clone();
            move |lua, this: HandleSelf| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    api.lock_class(id).await?;
                    let entries = api
                        .block
                        .lock()
                        .entries(id)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    make_entry_handle_list(&lua, entries, &entry_metatable)
                }
            }
        })?,
    )?;

    // mem:history() — the memory's entries including superseded ones (spec §Per-memory history),
    // the read where history is the point and the live filter is bypassed. Like `entries`, a
    // class-traversing read.
    methods.set(
        "history",
        lua.create_async_function({
            let api = api.clone();
            let entry_metatable = entry_metatable.clone();
            move |lua, this: HandleSelf| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    api.lock_class(id).await?;
                    let entries = api
                        .block
                        .lock()
                        .history(id)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    make_entry_handle_list(&lua, entries, &entry_metatable)
                }
            }
        })?,
    )?;

    // mem:supersede(old, new) — correct or retract a fact: mark `old` superseded by `new` (both
    // entry handles read from this memory). Locks the whole class, since it validates against and
    // mutates the merged identity's entries.
    methods.set(
        "supersede",
        lua.create_async_function({
            let api = api.clone();
            move |_, (this, old, new): (HandleSelf, Table, Table)| {
                let api = api.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    let (old, new) = (entry_handle_id(&old)?, entry_handle_id(&new)?);
                    api.lock_class(id).await?;
                    api.block
                        .lock()
                        .supersede(id, old, new)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;

    // mem:revise(old, new_text[, opts]) — correct a fact in one call: append new_text and supersede
    // `old` with it, returning the new entry. The find-and-supersede flow without the
    // append-then-supersede two-step; a failed supersede rolls the append back with it (no
    // half-applied correction). Locks the class, like supersede.
    methods.set(
        "revise",
        lua.create_async_function({
            let api = api.clone();
            let entry_metatable = entry_metatable.clone();
            move |lua, (this, old, text, opts): (HandleSelf, Table, String, Value)| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    let old = entry_handle_id(&old)?;
                    api.lock_class(id).await?;
                    let opts = append_options_from_lua(&lua, opts)?.unwrap_or_default();
                    let entry = {
                        let mut block = api.block.lock();
                        let new = block
                            .revise(id, old, &text, opts)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        block.entry_ref_by_id(new)
                    };
                    let entry = entry.ok_or(BlockConsistencyError::RevisedEntryMissing)?;
                    make_entry_handle(&lua, &entry, &entry_metatable)
                }
            }
        })?,
    )?;

    // mem:link(relation, other) / mem:unlink(relation, other) — record (or clear) a relation such
    // as `knows`, locking both endpoints. The script names the relation as a string; it is
    // recognized into its typed [`RelationName`] here, at the wrapper boundary.
    if features.linking {
        methods.set(
            "link",
            lua.create_async_function({
                let api = api.clone();
                move |_, (this, relation, other): (HandleSelf, String, Value)| {
                    let api = api.clone();
                    async move {
                        let from = handle_id(&this.0)?;
                        let to = link_target_id(&api, other)?;
                        api.lock_all([from, to]).await;
                        api.block
                            .lock()
                            .link(from, to, RelationName::new(&relation))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
        methods.set(
            "unlink",
            lua.create_async_function({
                let api = api.clone();
                move |_, (this, relation, other): (HandleSelf, String, Value)| {
                    let api = api.clone();
                    async move {
                        let from = handle_id(&this.0)?;
                        let to = link_target_id(&api, other)?;
                        api.lock_all([from, to]).await;
                        api.block
                            .lock()
                            .unlink(from, to, RelationName::new(&relation))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;

        // mem:outgoing(relation) / mem:incoming(relation) — the memory's links under `relation` out to
        // other memories, across its merged identity, in the canonical forward (outgoing) or reverse
        // (incoming) direction. Each result keeps the far memory as an actionable handle and renders as
        // `relation → name`. A traversing read, so it locks the whole `same_as` class.
        for (name, incoming) in [("outgoing", false), ("incoming", true)] {
            methods.set(
                name,
                lua.create_async_function({
                    let api = api.clone();
                    let memory_metatable = memory_metatable.clone();
                    let link_metatable = link_metatable.clone();
                    move |lua, (this, relation): (HandleSelf, String)| {
                        let api = api.clone();
                        let memory_metatable = memory_metatable.clone();
                        let link_metatable = link_metatable.clone();
                        async move {
                            let id = handle_id(&this.0)?;
                            api.lock_class(id).await?;
                            let links = {
                                let mut block = api.block.lock();
                                let result = if incoming {
                                    block.incoming(id, &relation)
                                } else {
                                    block.outgoing(id, &relation)
                                };
                                result.map_err(|error| route_error(error, &mut api.infra.lock()))?
                            };
                            make_link_handle_list(&lua, links, &memory_metatable, &link_metatable)
                        }
                    }
                })?,
            )?;
        }

        // mem:links() — every link out of the merged identity, in every relation and both directions:
        // the relationship overview. A traversing read, so it locks the whole `same_as` class.
        methods.set(
            "links",
            lua.create_async_function({
                let api = api.clone();
                let memory_metatable = memory_metatable.clone();
                let link_metatable = link_metatable.clone();
                move |lua, this: HandleSelf| {
                    let api = api.clone();
                    let memory_metatable = memory_metatable.clone();
                    let link_metatable = link_metatable.clone();
                    async move {
                        let id = handle_id(&this.0)?;
                        api.lock_class(id).await?;
                        let links = api
                            .block
                            .lock()
                            .links(id)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        make_link_handle_list(&lua, links, &memory_metatable, &link_metatable)
                    }
                }
            })?,
        )?;
    }

    // mem:propose_merge(other[, opts]) — record that this memory and `other` may be the same person
    // across platforms, for the adjudication pass to weigh on the evidence. `opts.rationale` states the
    // grounds for the match, which the adjudicator weighs as the proposer's claim, not as evidence. Not
    // a merge: it surfaces nothing until adjudicated. Locks both endpoints.
    if features.merging {
        methods.set(
            "propose_merge",
            lua.create_async_function({
                let api = api.clone();
                move |_, (this, other, opts): (HandleSelf, Table, Option<Table>)| {
                    let api = api.clone();
                    async move {
                        let (from, to) = (handle_id(&this.0)?, handle_id(&other)?);
                        let rationale = match opts {
                            Some(opts) => opts
                                .get::<Option<String>>("rationale")?
                                .map(|text| text.trim().to_owned())
                                .filter(|text| !text.is_empty()),
                            None => None,
                        };
                        api.lock_all([from, to]).await;
                        api.block
                            .lock()
                            .propose_merge(from, to, rationale)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
    }

    // mem:tag(name) / mem:untag(name) — apply or clear a vocabulary tag on this memory, locking it
    // first. The tag must have been created (`tags.create`); the name is recognized into its typed
    // [`TagName`] here, at the wrapper boundary.
    if features.tagging {
        methods.set(
            "tag",
            lua.create_async_function({
                let api = api.clone();
                move |_, (this, name): (HandleSelf, String)| {
                    let api = api.clone();
                    async move {
                        let id = handle_id(&this.0)?;
                        api.lock(id).await;
                        api.block
                            .lock()
                            .tag(id, TagName::new(&name))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
        methods.set(
            "untag",
            lua.create_async_function({
                let api = api.clone();
                move |_, (this, name): (HandleSelf, String)| {
                    let api = api.clone();
                    async move {
                        let id = handle_id(&this.0)?;
                        api.lock(id).await;
                        api.block
                            .lock()
                            .untag(id, TagName::new(&name))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
    }
    // `mem:set_volatility("high"|"medium"|"low")` — how fast this memory's facts age (spec §Time →
    // decay). The level is parsed in the block so an unknown level is a teachable error.
    methods.set(
        "set_volatility",
        lua.create_async_function({
            let api = api.clone();
            move |_, (this, level): (HandleSelf, String)| {
                let api = api.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    api.lock(id).await;
                    api.block
                        .lock()
                        .set_volatility(id, &level)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;
    // `mem:rename("person/sarah")` — the same memory under a new handle, for when someone changes
    // the name they go by (spec §Identity → Renaming). Locks the memory; the collision and self
    // guards live in the block.
    methods.set(
        "rename",
        lua.create_async_function({
            let api = api.clone();
            move |_, (this, new_name): (HandleSelf, String)| {
                let api = api.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    api.lock(id).await;
                    api.block
                        .lock()
                        .rename(id, &new_name)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;

    Ok(())
}

/// The metatable backing entry handles: `__tostring` and `__concat` render the handle as its
/// `text`, so a content read stays ergonomic (printable, concatenable) while the handle remains an
/// addressable entry for `mem:supersede`.
pub(super) fn entry_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    // An entry renders self-describingly: its text prefixed by what governs reading it — when the
    // fact occurs (if dated), a `disputed` marker when it is under an unresolved arbitration, the
    // visibility, and who it came from, e.g. "[2027-03-15 · disputed · private · from person/erin]
    // …". So printing a memory's entries shows at a glance when a dated fact happens, which are
    // contested, which are confidences to hold, and whose they are — rather than bare text whose
    // date and provenance the agent has to reconstruct (or search for) separately.
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let text = this.get::<String>("text")?;
            let mut segments = Vec::new();
            // `occurred_at` is the structured tagged table; render it back to a date for display.
            let occurred = this.get::<Value>("occurred_at")?;
            if !occurred.is_nil()
                && let Ok(temporal) = lua.from_value::<crate::time::TemporalRef>(occurred)
            {
                segments.push(time::format_occurrence(&temporal));
            }
            if this.get::<Option<bool>>("disputed")?.unwrap_or(false) {
                segments.push("disputed".to_owned());
            }
            if this.get::<Option<bool>>("stale")?.unwrap_or(false) {
                segments.push(crate::decay::STALE_LABEL.to_owned());
            }
            if let (Some(visibility), Some(teller)) = (
                this.get::<Option<String>>("visibility")?,
                this.get::<Option<String>>("told_by")?,
            ) {
                segments.push(format!("{visibility} · from {teller}"));
            }
            Ok(if segments.is_empty() {
                text
            } else {
                format!("[{}] {text}", segments.join(" · "))
            })
        })?,
    )?;
    metatable.set(
        "__concat",
        lua.create_function(|lua, (left, right): (Value, Value)| {
            Ok(format!(
                "{}{}",
                value_text(lua, &left)?,
                value_text(lua, &right)?
            ))
        })?,
    )?;
    // An entry is a read-only view (its fields carry the read; a change is an append/revise/supersede,
    // not a field write), so assigning to one raises the teachable error rather than silently doing
    // nothing.
    metatable.set("__newindex", readonly_newindex(lua, HandleKind::Entry)?)?;
    Ok(metatable)
}

/// The metatable backing the date objects `calendar` constructs. `__tostring` and `__concat` render
/// the ISO day (so a date prints and concatenates as `"YYYY-MM-DD"` — `"Reminder for " .. friday`
/// works — rather than erroring as a bare table), and `:to_string()` returns that same day. The other
/// methods are calendar-correct arithmetic returning new date objects (`:add_days`, `:add_weeks`,
/// `:add_months`), plus `:weekday()`. A date object is `{ day = "YYYY-MM-DD" }`, so it doubles as an
/// `occurred_at` value — the runtime does the date math the model would otherwise slip on.
pub(super) fn date_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| this.get::<String>("day"))?,
    )?;
    metatable.set(
        "__concat",
        lua.create_function(|lua, (left, right): (Value, Value)| {
            Ok(format!(
                "{}{}",
                date_text(lua, &left)?,
                date_text(lua, &right)?
            ))
        })?,
    )?;
    let methods = lua.create_table()?;
    // :to_string() — the ISO day as a string, the explicit form of the `__tostring`/`__concat`
    // rendering for a script that wants the string in hand.
    methods.set(
        "to_string",
        lua.create_function(|_, this: Table| this.get::<String>("day"))?,
    )?;
    // :add_days(n) / :add_weeks(n) — shift by whole days (a UTC day plus whole days is exact).
    for (name, per) in [("add_days", 1i64), ("add_weeks", 7)] {
        let mt = metatable.clone();
        methods.set(
            name,
            lua.create_function(move |lua, (this, count): (Table, i64)| {
                let day = this.get::<String>("day")?;
                let shifted = time::add_days(&day, count.saturating_mul(per))
                    .ok_or_else(|| CalendarError::InvalidDay { input: day.clone() })?;
                make_date(lua, shifted, &mt)
            })?,
        )?;
    }
    // :add_months(n) — calendar arithmetic, clamping a day past the target month's length.
    let mt = metatable.clone();
    methods.set(
        "add_months",
        lua.create_function(move |lua, (this, count): (Table, i64)| {
            let day = this.get::<String>("day")?;
            let shifted = time::add_months(&day, count)
                .ok_or_else(|| CalendarError::InvalidDay { input: day.clone() })?;
            make_date(lua, shifted, &mt)
        })?,
    )?;
    // :weekday() — the day's weekday name.
    methods.set(
        "weekday",
        lua.create_function(|_, this: Table| {
            let day = this.get::<String>("day")?;
            Ok(time::weekday(&day)
                .ok_or_else(|| CalendarError::InvalidDay { input: day.clone() })?)
        })?,
    )?;
    metatable.set("__index", methods)?;
    // A date object is a read-only value (arithmetic returns a *new* date); assigning to its `day`
    // field is a silent no-op, so the guard raises the teachable error instead.
    metatable.set("__newindex", readonly_newindex(lua, HandleKind::Date)?)?;
    Ok(metatable)
}

/// The metatable backing `memory.search` result objects: `__tostring` renders one as a readable
/// line (name, score, description, the matched-content snippet, the representative occurrence, the
/// salient relations, and any teller-private marker), so returning the result list reads back as text
/// rather than `<table>` while each result keeps its fields for the agent to inspect (`result.name` to
/// fetch, `result.score` to weigh, `result.occurred_at.day` to read a date, `result.relations` to read
/// the cast). The snippet is the content that produced the hit, so a result stays triageable even when
/// the description is stale or empty; the occurrence carries a scheduled or dated fact's date so a recall
/// relayed from the line keeps the *when*; the relations carry the memory's most salient links, so the
/// hit reveals who already participates in it — the recognition signal that steers a search toward reuse.
fn search_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let name: String = this.get("name")?;
            let description: String = this.get("description")?;
            let score: f32 = this.get("score")?;
            let marker: Option<String> = this.get("marker")?;
            let snippet: Option<String> = this.get("snippet")?;
            let mut line = format!("{name} (score {score:.2})");
            if !description.is_empty() {
                line.push_str(" — ");
                line.push_str(&description);
            }
            if let Some(snippet) = snippet.filter(|s| !s.is_empty()) {
                line.push_str(&format!(" match: \"{snippet}\""));
            }
            // The representative occurrence renders inline (like an entry's date on read), so a recall
            // that relays the hit line still carries a scheduled or dated fact's date. The stored value
            // is the structured tagged table; render it back to a date for display.
            let occurred = this.get::<Value>("occurred_at")?;
            if !occurred.is_nil()
                && let Ok(temporal) = lua.from_value::<crate::time::TemporalRef>(occurred)
            {
                line.push_str(&format!(" [when {}]", time::format_occurrence(&temporal)));
            }
            // The salient relations (its cast) render inline as `relation → name`, pre-rendered when the
            // result was built, so the printed hit reveals who already participates in this memory —
            // the recognition signal that steers a recall toward reuse over a name-guessed duplicate.
            let relations_line: Option<String> = this.get("relations_line")?;
            if let Some(relations_line) = relations_line.filter(|line| !line.is_empty()) {
                line.push_str(" — ");
                line.push_str(&relations_line);
            }
            if let Some(marker) = marker {
                line.push(' ');
                line.push_str(&marker);
            }
            Ok(line)
        })?,
    )?;
    // A search result is a read-only row; assigning to its fields does nothing, so the guard raises
    // the teachable error naming the operation that persists a change.
    metatable.set(
        "__newindex",
        readonly_newindex(lua, HandleKind::SearchResult)?,
    )?;
    // A hit concatenates as its rendered line, mirroring how it prints.
    metatable.set("__concat", concat_via_tostring(lua)?)?;
    Ok(metatable)
}

/// The metatable backing `tags.list` result objects: `__tostring` renders one as `name — purpose
/// (N uses)`, so the vocabulary reads back as text rather than `<table>` while each result keeps
/// its `name`, `description`, and `count` fields.
fn tag_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| {
            let name: String = this.get("name")?;
            let description: String = this.get("description")?;
            let count: i64 = this.get("count")?;
            let uses = if count == 1 {
                "1 use".to_owned()
            } else {
                format!("{count} uses")
            };
            let mut line = name;
            if !description.is_empty() {
                line.push_str(" — ");
                line.push_str(&description);
            }
            line.push_str(&format!(" ({uses})"));
            Ok(line)
        })?,
    )?;
    Ok(metatable)
}

/// The metatable backing the link results `mem:outgoing`/`incoming`/`links` return: `__tostring`
/// renders one as `relation → name` (outgoing) or `relation ← name` (incoming) — with a dated far
/// memory's occurrence appended as `[when …]` (the same phrasing a search hit uses) — so a reader's
/// list reads back as readable relationships that keep the linked event's *when*, while each result
/// keeps its `relation`, `memory` (the far memory as a handle), `name`, `direction`, `source`, and
/// `occurred_at` fields for the agent to inspect and act on.
fn link_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let relation: String = this.get("relation")?;
            let name: String = this.get("name")?;
            let direction: String = this.get("direction")?;
            let arrow = if direction == "incoming" {
                "←"
            } else {
                "→"
            };
            let mut line = format!("{relation} {arrow} {name}");
            // The far memory's occurrence renders inline (like a search hit's date), so a link to a
            // dated event carries *when* on the line. The stored value is the structured tagged table;
            // render it back to a date for display.
            let occurred = this.get::<Value>("occurred_at")?;
            if !occurred.is_nil()
                && let Ok(temporal) = lua.from_value::<crate::time::TemporalRef>(occurred)
            {
                line.push_str(&format!(" [when {}]", time::format_occurrence(&temporal)));
            }
            Ok(line)
        })?,
    )?;
    // `"- " .. link` composes the link's rendered line — the join the agent writes when listing a
    // memory's relationships — rather than erroring as a bare table.
    metatable.set("__concat", concat_via_tostring(lua)?)?;
    Ok(metatable)
}

/// The `tags` global: `create` and `describe` mutate the vocabulary, `list` reads it. Creation and
/// application are deliberately distinct — applying (`mem:tag`) never mutates a tag's description,
/// creating always forces a purpose (spec §Tag operations).
pub(super) fn tags_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let tags = lua.create_table()?;
    // tags.create(name, description) — add a tag to the vocabulary with a one-line purpose.
    tags.set(
        "create",
        lua.create_async_function({
            let api = api.clone();
            move |_, (name, description): (String, String)| {
                let api = api.clone();
                async move {
                    api.block
                        .lock()
                        .create_tag(TagName::new(&name), &description)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;
    // tags.describe(name, description) — change an existing tag's purpose.
    tags.set(
        "describe",
        lua.create_async_function({
            let api = api.clone();
            move |_, (name, description): (String, String)| {
                let api = api.clone();
                async move {
                    api.block
                        .lock()
                        .describe_tag(TagName::new(&name), &description)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;
    // tags.list() — the whole vocabulary, each result `{ name, description, count }` printing as a
    // readable line.
    let result_metatable = tag_result_metatable(lua)?;
    tags.set(
        "list",
        lua.create_async_function({
            let api = api.clone();
            move |lua, ()| {
                let api = api.clone();
                let result_metatable = result_metatable.clone();
                async move {
                    let entries = api
                        .block
                        .lock()
                        .all_tags()
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    let list = lua.create_table()?;
                    for (index, entry) in entries.into_iter().enumerate() {
                        let table = lua.create_table()?;
                        table.set("name", entry.name.as_str())?;
                        table.set("description", entry.description)?;
                        table.set("count", entry.count)?;
                        table.set_metatable(Some(result_metatable.clone()))?;
                        list.set(index + 1, table)?;
                    }
                    Ok(Value::Table(list))
                }
            }
        })?,
    )?;
    Ok(tags)
}

/// The metatable backing `links.list`/`links.get` result objects: `__tostring` renders one as
/// `name / inverse — from-to[, symmetric][, reflexive]`, so the registry reads back as text while
/// each result keeps its fields.
fn relation_result_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| {
            let name: String = this.get("name")?;
            let inverse: String = this.get("inverse")?;
            let from_card: String = this.get("from_card")?;
            let to_card: String = this.get("to_card")?;
            let symmetric: bool = this.get("symmetric")?;
            let reflexive: bool = this.get("reflexive")?;
            let description: String = this.get("description")?;
            let mut line = format!("{name} / {inverse} — {from_card}-to-{to_card}");
            if symmetric {
                line.push_str(", symmetric");
            }
            if reflexive {
                line.push_str(", reflexive");
            }
            line.push_str(&format!(": {description}"));
            Ok(line)
        })?,
    )?;
    Ok(metatable)
}

/// The `links` global: `register` adds a relation to the schema, `list` and `get` read it. Link
/// *edges* are made on memory handles (`mem:link`/`mem:unlink`); this global manages the relation
/// *registry* they instantiate (spec §Link relation registry).
pub(super) fn links_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let links = lua.create_table()?;
    let result_metatable = relation_result_metatable(lua)?;
    // links.register{ name, inverse, from_card, to_card, symmetric?, reflexive? } — register one
    // relation, accessible under either label; the inverse view's cardinality is computed.
    links.set(
        "register",
        lua.create_async_function({
            let api = api.clone();
            move |lua, spec: Value| {
                let api = api.clone();
                async move {
                    let spec: RelationSpec = lua.from_value(spec)?;
                    api.block
                        .lock()
                        .register_relation(spec)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;
    // links.list() — the whole registry, each result printing as a readable line.
    links.set(
        "list",
        lua.create_async_function({
            let api = api.clone();
            let result_metatable = result_metatable.clone();
            move |lua, ()| {
                let api = api.clone();
                let result_metatable = result_metatable.clone();
                async move {
                    let views = api
                        .block
                        .lock()
                        .all_relations()
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    let list = lua.create_table()?;
                    for (index, view) in views.into_iter().enumerate() {
                        let table = make_relation_result(&lua, &view, &result_metatable)?;
                        list.set(index + 1, table)?;
                    }
                    Ok(Value::Table(list))
                }
            }
        })?,
    )?;
    // links.get(name) — one relation by either label, or nil.
    links.set(
        "get",
        lua.create_async_function({
            let api = api.clone();
            move |lua, name: String| {
                let api = api.clone();
                let result_metatable = result_metatable.clone();
                async move {
                    let view = api
                        .block
                        .lock()
                        .relation(&name)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    match view {
                        Some(view) => Ok(Value::Table(make_relation_result(
                            &lua,
                            &view,
                            &result_metatable,
                        )?)),
                        None => Ok(Value::Nil),
                    }
                }
            }
        })?,
    )?;
    Ok(links)
}

/// Resolve an existing memory to an enriched handle, or `None` when nothing resolves. Locks the
/// resolved stub, mints the handle, and carries the renamed-identity affordances: `former_names` on
/// any renamed memory, and — when resolved *by* a former name — a `former_handle` field plus an active
/// rename note into the agent's own output, so an old-name lookup is never mistaken for a second
/// person. Shared by `memory.get` (which returns nil when this is `None`) and `memory.get_or_create`
/// (which creates instead), so both read a renamed person identically. The enrichment fields are
/// written with `raw_set`, bypassing the handle's read-only `__newindex` guard.
async fn resolve_existing_handle(
    lua: &Lua,
    api: &BlockApi,
    metatable: &Table,
    name: &str,
) -> mlua::Result<Option<Table>> {
    let resolved = api
        .block
        .lock()
        .get(name)
        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
    let Some((id, via_former)) = resolved else {
        return Ok(None);
    };
    api.lock(id).await;
    let handle = make_handle(lua, id, metatable)?;
    // Precompute the memory's link neighborhood and stash it as a rendered line on the handle, so a
    // recall that fetches a topic hub sees its spokes — the linked events its decisions live on — the
    // moment the handle renders, rather than reading only the hub's own entries and dropping a
    // spoke-held fact. A traversing read, so it locks the whole `same_as` class (like the link
    // readers). Committed-only and not visibility-filtered, mirroring `<memory>:links`. Written with
    // `raw_set` to bypass the read-only `__newindex` guard; absent (so no line renders) when the memory
    // has no links.
    api.lock_class(id).await?;
    let links = api
        .block
        .lock()
        .links(id)
        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
    if !links.is_empty() {
        handle.raw_set("neighborhood", render_neighborhood(&links))?;
    }
    // A renamed memory carries its prior handles in `former_names`, so the agent reads it as the same
    // person under their current `name` and connects its old-name content rather than splitting them.
    let former = api
        .block
        .lock()
        .former_names(id)
        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
    if !former.is_empty() {
        handle.raw_set("former_names", lua.create_sequence_from(former)?)?;
    }
    // Resolved *by* a former name: flag which one, and — because the passive fields are easy for a
    // small model to skip (it reads `e.text` and concludes the old and new handle are two people) —
    // emit an active note into the agent's own output, so an old-name lookup cannot be mistaken for a
    // second person however the handle is inspected. The note rides the agent's result only, never a
    // participant, so it stays deadname-safe.
    if via_former {
        handle.raw_set("former_handle", name)?;
        let current = api
            .block
            .lock()
            .handle_field(id, "name")
            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
        if let Some(current) = current {
            api.printed.lock().push_str(&format!(
                "note: \"{name}\" now goes by \"{current}\" — the same person, renamed.\n"
            ));
        }
    }
    Ok(Some(handle))
}

/// The `memory` global: `create`, `get`, and `get_or_create`, all of which mint handles (hence the
/// metatable).
pub(super) fn memory_table(lua: &Lua, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
    let memory = lua.create_table()?;
    // memory.create(name[, content][, opts]) — create a memory and optionally its first entry,
    // then lock the freshly-minted id (uncontended — no other block knows it yet). `opts` carries
    // the same overrides as `mem:append` (`occurred_at`, `visibility`, `volatility`), so a reminder
    // can be created and timed in one call.
    memory.set(
        "create",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, (name, content, opts): (String, Option<String>, Value)| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let opts = append_options_from_lua(&lua, opts)?;
                    let id = api
                        .block
                        .lock()
                        .create_with_opts(MemoryName::new(name), content.as_deref(), opts)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock(id).await;
                    make_handle(&lua, id, &metatable)
                }
            }
        })?,
    )?;
    // memory.get(name) — resolve through the block's pending creates, then the graph, locking the
    // resolved stub. A renamed person still resolves by a former name (see `resolve_existing_handle`).
    memory.set(
        "get",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, name: String| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    match resolve_existing_handle(&lua, &api, &metatable, &name).await? {
                        Some(handle) => Ok(Value::Table(handle)),
                        None => Ok(Value::Nil),
                    }
                }
            }
        })?,
    )?;
    // memory.get_or_create(name[, content][, opts]) — fetch the memory if it exists, otherwise create
    // it (with the same optional first entry and overrides as `memory.create`). The idiomatic
    // `memory.get(name) or memory.create(name, ...)` in one call, so an agent that applies that idiom
    // inconsistently within a script no longer trips the already-exists error. When the memory exists
    // its `content`/`opts` are ignored — it is returned as it stands, its description untouched — so a
    // fetch never silently overwrites what is already recorded. This is distinct from `memory.create`,
    // whose fail-on-exists strictness is load-bearing: creating a second person stub over an existing
    // name must stay a deliberate act (merge and identity scenarios rely on it), so `create` keeps
    // raising while `get_or_create` is the tool for when existence is uncertain.
    memory.set(
        "get_or_create",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, (name, content, opts): (String, Option<String>, Value)| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    if let Some(handle) =
                        resolve_existing_handle(&lua, &api, &metatable, &name).await?
                    {
                        return Ok(handle);
                    }
                    let opts = append_options_from_lua(&lua, opts)?;
                    let id = api
                        .block
                        .lock()
                        .create_with_opts(MemoryName::new(name), content.as_deref(), opts)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock(id).await;
                    make_handle(&lua, id, &metatable)
                }
            }
        })?,
    )?;
    // memory.search(query[, opts]) — semantic + lexical recall over the agent's whole memory,
    // visibility-filtered against who is present (a teller-private hit only surfaces while its
    // teller is here, with a marker). Embeds the query off any lock, then ranks under a brief read
    // lock. Returns a list of result objects
    // (`{ name, description, score, marker?, snippet?, occurred_at?, relations? }`), best first; each
    // prints as a readable line so `return memory.search(...)` reads back the results rather than
    // `<table>`.
    let result_metatable = search_result_metatable(lua)?;
    memory.set(
        "search",
        lua.create_async_function({
            let api = api.clone();
            let result_metatable = result_metatable.clone();
            move |lua, (query, opts): (String, Value)| {
                let api = api.clone();
                let result_metatable = result_metatable.clone();
                async move {
                    let (engine, present_set) = api.block.lock().retrieval_handle();
                    let opts: SearchOpts = if opts.is_nil() {
                        SearchOpts::default()
                    } else {
                        lua.from_value(opts)?
                    };
                    let rows = run_memory_search(&engine, &present_set, &query, &opts).await?;
                    let list = lua.create_table()?;
                    for (index, row) in rows.into_iter().enumerate() {
                        let table = lua.create_table()?;
                        table.set("name", row.name)?;
                        table.set("description", row.description)?;
                        table.set("score", row.score)?;
                        if let Some(marker) = row.marker {
                            table.set("marker", marker)?;
                        }
                        if let Some(snippet) = row.snippet {
                            table.set("snippet", snippet)?;
                        }
                        // The occurrence rides as the same tagged table `append` accepts (e.g.
                        // `{ day = "…" }`), so a script can read `result.occurred_at.day` and the
                        // metatable's `__tostring` renders the date on the result line.
                        if let Some(occurred_at) = row.occurred_at {
                            table.set("occurred_at", lua.to_value(&occurred_at)?)?;
                        }
                        // The salient relations as a structural array the agent can read
                        // (`result.relations[1].name` to recognize the cast, `.relation`/`.direction`
                        // to read the edge), plus a pre-rendered line the metatable's `__tostring`
                        // appends — so the hit passively carries who already participates in this
                        // memory. Absent when the memory has no out-of-class links.
                        if !row.relations.is_empty() {
                            let relations = lua.create_table()?;
                            for (position, relation) in row.relations.iter().enumerate() {
                                let entry = lua.create_table()?;
                                entry.set("relation", relation.relation.as_str())?;
                                entry.set("name", relation.other_name.as_str())?;
                                entry.set(
                                    "direction",
                                    match relation.direction {
                                        LinkDirection::Incoming => "incoming",
                                        LinkDirection::Outgoing => "outgoing",
                                    },
                                )?;
                                relations.set(position + 1, entry)?;
                            }
                            table.set("relations", relations)?;
                        }
                        if let Some(line) =
                            render_salient_relations(&row.relations, row.more_relations)
                        {
                            table.raw_set("relations_line", line)?;
                        }
                        table.set_metatable(Some(result_metatable.clone()))?;
                        list.set(index + 1, table)?;
                    }
                    Ok(Value::Table(list))
                }
            }
        })?,
    )?;
    Ok(memory)
}

/// The `block` global: `abort(reason)`, which discards the buffer and ends the block. It touches no
/// memory, so it stays a synchronous function and takes no lock.
pub(super) fn block_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let block_tbl = lua.create_table()?;
    block_tbl.set(
        "abort",
        lua.create_function({
            let block = api.block.clone();
            move |_, reason: Option<String>| {
                block.lock().abort(reason);
                Err::<(), _>(mlua::Error::RuntimeError("block aborted".to_owned()))
            }
        })?,
    )?;
    Ok(block_tbl)
}

/// The `context` global: `current()`, the current conversation's `context/*` memory (its
/// `#confidential` tag tells the agent whether the room is confidential), or nil if there is none.
/// The resolved context memory is locked like any other touched memory.
pub(super) fn context_table(lua: &Lua, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
    let context = lua.create_table()?;
    context.set(
        "current",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, ()| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let current = api.block.lock().current_context();
                    match current {
                        Some(id) => {
                            api.lock(id).await;
                            Ok(Value::Table(make_handle(&lua, id, &metatable)?))
                        }
                        None => Ok(Value::Nil),
                    }
                }
            }
        })?,
    )?;
    Ok(context)
}

/// The `convo` global: `turn(id)` resolves a conversation turn link — the id carried in a
/// `[turn:<ulid>]` token, the canonical agent-facing reference form — to that moment and a small
/// window of the surrounding turns in its session. A console deep-link's `?turn=<ulid>` never reaches
/// here: the connector normalizes any pasted URL to the token before the message reaches the agent
/// (see [`turn_ref`](zuihitsu_core::turn_ref)), so this resolver reads a bare ULID and nothing more.
/// The result is a table `{ id, ref, text, speaker, role, at,
/// window }` — the focal turn's fields at the top (`ref` the canonical `[turn:…]` to cite it by), and
/// `window` the ordered surrounding turns (the focal one included, flagged `focused`) — that prints as
/// a readable transcript excerpt so `return convo.turn(id)` reads back as the exchange. Resolution
/// obeys the audience rule: a moment resolves only when everyone present here was in its audience. A
/// malformed id, an id whose moment the present audience did not all share, and an unknown id are three
/// distinct teachable errors (see [`TurnResolveError`]); resolving is read-only and touches no memory,
/// so it takes no lock.
pub(super) fn convo_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let convo = lua.create_table()?;
    let line_metatable = turn_line_metatable(lua)?;
    let window_metatable = turn_window_metatable(lua)?;
    convo.set(
        "turn",
        lua.create_function({
            let api = api.clone();
            let line_metatable = line_metatable.clone();
            let window_metatable = window_metatable.clone();
            move |lua, id: String| {
                let turn_id = TurnId(Ulid::from_string(&id).map_err(|source| {
                    TurnResolveError::InvalidTurnId {
                        id: id.clone(),
                        source,
                    }
                })?);
                let (engine, present_set) = api.block.lock().turn_resolution_handle();
                let window = match resolve_turn(
                    engine.as_ref(),
                    &present_set,
                    turn_id,
                    TURN_WINDOW_BEFORE,
                    TURN_WINDOW_AFTER,
                )
                .map_err(TurnResolveError::Store)?
                {
                    TurnResolution::Resolved(window) => window,
                    TurnResolution::AudienceMismatch => {
                        return Err(TurnResolveError::AudienceMismatch { id }.into());
                    }
                    TurnResolution::NotFound => {
                        return Err(TurnResolveError::NotFound { id }.into());
                    }
                };
                make_turn_window(lua, &window, &line_metatable, &window_metatable)
            }
        })?,
    )?;
    Ok(convo)
}

/// Build the `convo.turn` result: the focal turn's fields at the top, and `window` the ordered
/// surrounding turns (the focal one flagged `focused`), each a line backed by [`turn_line_metatable`].
fn make_turn_window(
    lua: &Lua,
    window: &TurnWindow,
    line_metatable: &Table,
    window_metatable: &Table,
) -> mlua::Result<Table> {
    let list = lua.create_table()?;
    for (index, turn) in window.turns.iter().enumerate() {
        let line = make_turn_line(lua, turn, index == window.focus, line_metatable)?;
        list.set(index + 1, line)?;
    }
    let focus = &window.turns[window.focus];
    let result = lua.create_table()?;
    result.set("id", focus.turn_id.0.to_string())?;
    result.set("ref", focus.reference.as_str())?;
    result.set("text", focus.text.as_str())?;
    result.set("speaker", focus.speaker.as_str())?;
    result.set("role", turn_role_label(focus.role))?;
    result.set("at", time::format_stamp(focus.recorded_at))?;
    result.set("window", list)?;
    result.set_metatable(Some(window_metatable.clone()))?;
    Ok(result)
}

/// One turn in a `convo.turn` window as `{ id, ref, text, speaker, role, at, focused }`, backed by
/// [`turn_line_metatable`] so it prints as a transcript line.
fn make_turn_line(
    lua: &Lua,
    turn: &ResolvedTurn,
    focused: bool,
    line_metatable: &Table,
) -> mlua::Result<Table> {
    let line = lua.create_table()?;
    line.set("id", turn.turn_id.0.to_string())?;
    line.set("ref", turn.reference.as_str())?;
    line.set("text", turn.text.as_str())?;
    line.set("speaker", turn.speaker.as_str())?;
    line.set("role", turn_role_label(turn.role))?;
    line.set("at", time::format_stamp(turn.recorded_at))?;
    line.set("focused", focused)?;
    line.set_metatable(Some(line_metatable.clone()))?;
    Ok(line)
}

/// The agent-facing role label for a resolved turn — the string a script branches on.
fn turn_role_label(role: TurnRole) -> &'static str {
    match role {
        TurnRole::Participant => "participant",
        TurnRole::Agent => "agent",
        TurnRole::System => "system",
    }
}

/// The metatable backing a `convo.turn` window line: `__tostring` renders it as `[at] speaker: text`,
/// with a `»` marker on the focal turn, so a window reads back as a transcript excerpt with the linked
/// moment called out.
fn turn_line_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|_, this: Table| {
            let at: String = this.get("at")?;
            let speaker: String = this.get("speaker")?;
            let text: String = this.get("text")?;
            let focused: bool = this.get("focused")?;
            let marker = if focused { "» " } else { "  " };
            Ok(format!("{marker}[{at}] {speaker}: {text}"))
        })?,
    )?;
    Ok(metatable)
}

/// The metatable backing the `convo.turn` result: `__tostring` renders its `window` as the joined
/// transcript lines, so `return convo.turn(id)` reads back as the exchange around the linked moment.
fn turn_window_metatable(lua: &Lua) -> mlua::Result<Table> {
    let metatable = lua.create_table()?;
    metatable.set(
        "__tostring",
        lua.create_function(|lua, this: Table| {
            let window: Table = this.get("window")?;
            let lines: Vec<String> = window
                .sequence_values::<Value>()
                .filter_map(Result::ok)
                .map(|line| render(lua, &line))
                .collect();
            Ok(lines.join("\n"))
        })?,
    )?;
    Ok(metatable)
}

/// The `calendar` global: `upcoming`, `overdue`, `on`, and `recurring`, each returning a list of memory
/// handles, soonest first. Unlike the brief's `<upcoming/>` block these are the agent's own
/// queries and are not visibility-filtered (like `mem:entries`, the agent sees its whole memory).
/// Strict locking: each returned memory is locked, since the query read (and touched) it.
pub(super) fn calendar_table(lua: &Lua, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
    let calendar = lua.create_table()?;
    calendar.set(
        "upcoming",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, opts: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let within = within_arg(opts)?;
                    let ids = api
                        .block
                        .lock()
                        .upcoming(within.as_deref())
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;
    calendar.set(
        "overdue",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, opts: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let within = within_arg(opts)?;
                    let ids = api
                        .block
                        .lock()
                        .overdue(within.as_deref())
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;
    calendar.set(
        "on",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, date: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    // Accept a date object (`calendar.today()`, `calendar.next(...)`) as readily as a
                    // `"YYYY-MM-DD"` string, so the calendar's own return value feeds its sibling.
                    let date = day_string(&date)?;
                    let ids = api
                        .block
                        .lock()
                        .on(&date)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;
    calendar.set(
        "recurring",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, ()| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let ids = api
                        .block
                        .lock()
                        .recurring()
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;

    // Date construction: the agent names a relative date and the runtime computes it, so a date is
    // never arithmetic the model carries in its head. Each returns a date object (see
    // `date_metatable`) that doubles as an `occurred_at` value. Synchronous — they read the clock and
    // do pure date math, touching no memory, so they need no lock.
    let date_metatable = date_metatable(lua)?;
    calendar.set("today", {
        let api = api.clone();
        let dmt = date_metatable.clone();
        lua.create_function(move |lua, ()| {
            let now = api.block.lock().now();
            make_date(lua, time::today(now), &dmt)
        })?
    })?;
    calendar.set("next", {
        let api = api.clone();
        let dmt = date_metatable.clone();
        lua.create_function(move |lua, weekday: String| {
            let now = api.block.lock().now();
            let day = time::next_weekday(now, &weekday)
                .ok_or(CalendarError::NotAWeekday { input: weekday })?;
            make_date(lua, day, &dmt)
        })?
    })?;
    for (name, per) in [("in_days", 1i64), ("in_weeks", 7)] {
        let api = api.clone();
        let dmt = date_metatable.clone();
        calendar.set(
            name,
            lua.create_function(move |lua, count: i64| {
                let now = api.block.lock().now();
                let day = time::add_days(&time::today(now), count.saturating_mul(per)).ok_or(
                    CalendarError::DateOutOfRange {
                        days: count.saturating_mul(per),
                    },
                )?;
                make_date(lua, day, &dmt)
            })?,
        )?;
    }
    calendar.set("date", {
        let dmt = date_metatable.clone();
        lua.create_function(move |lua, day: String| {
            if time::civil_date_to_millis(&day).is_none() {
                return Err(CalendarError::InvalidDate { input: day }.into());
            }
            make_date(lua, day, &dmt)
        })?
    })?;
    Ok(calendar)
}

/// The window argument `calendar.upcoming` and `calendar.overdue` share: a bare duration string
/// ("31 days", "2 weeks") stands for the window directly — the shape the agent naturally writes —
/// while `{ within = "…" }` and `nil` (the default window) keep working. Anything else is a teachable
/// [`CalendarError`] rather than an opaque conversion failure; an unparseable duration string still
/// errors downstream where the duration is parsed, with its own teachable message.
fn within_arg(opts: Value) -> mlua::Result<Option<String>> {
    match opts {
        Value::Nil => Ok(None),
        Value::String(within) => Ok(Some(within.to_string_lossy())),
        Value::Table(table) => Ok(table.get("within")?),
        other => Err(CalendarError::NotAWindow {
            type_name: other.type_name(),
        }
        .into()),
    }
}
