//! The per-block Lua module tables — `memory`, `block`, `context`, `calendar`, `tags`, `links`,
//! and `convo` — and the helpers that assemble their rows.

use super::{metatables::*, *};

/// The `tags` global: `create` and `describe` mutate the vocabulary, `list` reads it. Creation and
/// application are deliberately distinct — applying (`mem:tag`) never mutates a tag's description,
/// creating always forces a purpose (spec §Tag operations).
pub(in crate::agent::lua) fn tags_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
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

/// The `links` global: `register` adds a relation to the schema, `list` and `get` read it. Link
/// *edges* are made on memory handles (`mem:link`/`mem:unlink`); this global manages the relation
/// *registry* they instantiate (spec §Link relation registry).
pub(in crate::agent::lua) fn links_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
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
pub(in crate::agent::lua) fn memory_table(
    lua: &Lua,
    api: &BlockApi,
    metatable: &Table,
) -> mlua::Result<Table> {
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
    // inconsistently within a script does not trip the already-exists error. When the memory exists
    // its `content`/`opts` are ignored — it is returned as it stands, its description untouched — so a
    // fetch never silently overwrites what is already recorded. This is distinct from `memory.create`,
    // whose fail-on-exists strictness is load-bearing: creating a second person stub over an existing
    // name must stay a deliberate act (the merge and identity flows rely on it), so `create` keeps
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
pub(in crate::agent::lua) fn block_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
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

/// The `context` global: `current()`, the current conversation's [`Namespace::Context`] memory (its
/// `#confidential` tag tells the agent whether the room is confidential), or nil if there is none.
/// The resolved context memory is locked like any other touched memory.
pub(in crate::agent::lua) fn context_table(
    lua: &Lua,
    api: &BlockApi,
    metatable: &Table,
) -> mlua::Result<Table> {
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
pub(in crate::agent::lua) fn convo_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
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

/// The `calendar` global: `upcoming`, `overdue`, `on`, and `recurring`, each returning a list of memory
/// handles, soonest first. Unlike the brief's `<upcoming/>` block these are the agent's own
/// queries and are not visibility-filtered (like `mem:entries`, the agent sees its whole memory).
/// Strict locking: each returned memory is locked, since the query read (and touched) it.
pub(in crate::agent::lua) fn calendar_table(
    lua: &Lua,
    api: &BlockApi,
    metatable: &Table,
) -> mlua::Result<Table> {
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
