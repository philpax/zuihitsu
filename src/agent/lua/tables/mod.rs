//! The free-function builders that mint the per-block Lua globals, their handle metatables, and the
//! `mem:*` handle methods. These translate script calls into [`MemoryBlock`] transaction calls over the
//! shared [`BlockApi`] seam; they never touch the buffer, the events, or the visibility rules directly.

pub(super) use mlua::{Lua, LuaSerdeExt, Table, Value};
pub(super) use ulid::Ulid;

pub(super) use crate::{
    InstanceFeatures,
    agent::turn::{ResolvedTurn, TurnResolution, TurnWindow, resolve_turn},
    event::TurnRole,
    ids::{MemoryName, TurnId},
    memory::memory_block::{LinkDirection, RelationSpec},
    time,
    vocabulary::{RelationName, TagName},
};

pub(super) use super::{
    error::{BlockConsistencyError, CalendarError, HandleKind, ListError, TurnResolveError},
    runtime::{
        BlockApi, HandleSelf, SearchOpts, append_options_from_lua, concat_via_tostring, date_text,
        day_string, entry_handle_id, handle_id, link_target_id, make_capped_handle_list, make_date,
        make_entry_handle, make_entry_handle_list, make_handle, make_handle_list,
        make_link_handle_list, make_relation_result, readonly_newindex, render, render_details,
        render_neighborhood, render_salient_relations, route_error, run_memory_search, value_text,
    },
};

/// How many turns before and after the focal turn `convo.turn` includes in its window — a few on
/// each side, enough to place the linked moment in its immediate exchange without replaying the room.
pub(super) const TURN_WINDOW_BEFORE: usize = 3;
pub(super) const TURN_WINDOW_AFTER: usize = 3;

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
mod metatables;
mod modules;

pub(super) use metatables::entry_metatable;
use metatables::*;
use modules::*;

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
    // reading `m.name` gets nil and concludes the calendar is empty.
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
    // A memory handle is a read-only view: assigning to a field (`m.occurred_at = ...`) would be a
    // silent no-op that misleads the agent into thinking a change landed. The guard raises a teachable
    // error naming the persisting operation instead. Internal setup writes raw fields with `raw_set` to
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

    // mem:details() — the memory's whole record in one string: header (name, description, former
    // names), every entry under a count header, links in both directions, tags, and volatility, each
    // section reusing the same rendering the dedicated readers use. A class-traversing read, so it
    // locks the whole `same_as` class. Always installed (like `entries`); its link and tag sections are
    // simply empty on an instance without those features.
    methods.set(
        "details",
        lua.create_async_function({
            let api = api.clone();
            let entry_metatable = entry_metatable.clone();
            let memory_metatable = memory_metatable.clone();
            let link_metatable = link_metatable.clone();
            move |lua, this: HandleSelf| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                let memory_metatable = memory_metatable.clone();
                let link_metatable = link_metatable.clone();
                async move {
                    let id = handle_id(&this.0)?;
                    api.lock_class(id).await?;
                    let details = api
                        .block
                        .lock()
                        .details(id)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    render_details(
                        &lua,
                        &details,
                        &entry_metatable,
                        &memory_metatable,
                        &link_metatable,
                    )
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
