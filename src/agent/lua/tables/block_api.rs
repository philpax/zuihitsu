//! `install_block_api`: install the per-block memory API as `'static` async Lua functions.

use super::*;

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
pub(crate) fn install_block_api(
    lua: &Lua,
    api: &BlockApi,
    methods: &Table,
    metatable: &Table,
    entry_metatable: &Table,
    features: &InstanceFeatures,
) -> mlua::Result<()> {
    let link_metatable = link_result_metatable(lua)?;
    super::handles::install_handle_methods(
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
    globals.set("turn", turn_table(lua, api)?)?;
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
    // The web module is installed only when browsing is on and a fetcher is connected. Browsing gates
    // the three prompt surfaces (registration, reference, scaffold); the fetcher is the runtime
    // dependency, absent on a fetcher-less in-memory instance — where the module stays absent (nil), a
    // teachable nil-call error, rather than a call that could not fetch anyway.
    if features.browsing && api.web.is_some() {
        globals.set("web", web_table(lua, api)?)?;
    }
    Ok(())
}
