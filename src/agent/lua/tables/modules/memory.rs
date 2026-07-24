//! The `memory` global: `create`, `get`, `get_or_create`, `search`, and `list`.

use crate::agent::lua::tables::modules::{metatables::*, *};

/// How many handles `memory.list` returns before eliding the rest — enough to reveal which stems
/// exist without flooding the transcript when a broad prefix matches many. The elided count rides the
/// rendered form as a `(+N more — narrow the prefix)` note.
const LIST_CAP: usize = 50;

/// The `memory` global: `create`, `get`, and `get_or_create`, all of which mint handles (hence the
/// metatable).
pub(crate) fn memory_table(lua: &Lua, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
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
            move |lua, (name, content, opts): (Value, Value, Value)| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let name: String = arg(
                        &lua,
                        name,
                        "memory.create",
                        "a memory name string",
                        "pass the handle directly, memory.create(\"person/dave\")",
                    )?;
                    let content: Option<String> = arg(
                        &lua,
                        content,
                        "memory.create",
                        "the first entry's text as a string (or nil for none)",
                        "memory.create(\"person/dave\", \"met at the conference\")",
                    )?;
                    check_interpolated("memory name", &name)?;
                    if let Some(content) = &content {
                        check_interpolated("entry text", content)?;
                    }
                    let opts = append_options_from_lua(&api, &lua, opts)?;
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
    // memory.get(name_or_handle) — resolve through the block's pending creates, then the graph,
    // locking the resolved stub. Accepts a name string or an existing memory handle (the natural
    // `memory.get(h)` over a handle from `memory.list`/`memory.create`), a handle resolving by its
    // current name so the lookup is identical either way. A renamed person still resolves by a former
    // name (see `resolve_existing_handle`).
    memory.set(
        "get",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, target: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let name = get_argument_name(&api, target)?;
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
            move |lua, (target, content, opts): (Value, Value, Value)| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let name = get_argument_name(&api, target)?;
                    let content: Option<String> = arg(
                        &lua,
                        content,
                        "memory.get_or_create",
                        "the first entry's text as a string (or nil for none)",
                        "memory.get_or_create(\"person/dave\", \"met at the conference\")",
                    )?;
                    if let Some(content) = &content {
                        check_interpolated("entry text", content)?;
                    }
                    if let Some(handle) =
                        resolve_existing_handle(&lua, &api, &metatable, &name).await?
                    {
                        return Ok(handle);
                    }
                    let opts = append_options_from_lua(&api, &lua, opts)?;
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
    // A hit is also a usable memory handle: any key it does not carry itself (a method like `append`,
    // or a lazy `name`/`description` read) falls through to the memory-handle metatable's `__index`,
    // so `hits[1]:append(…)`/`:details()` work without a `memory.get` round-trip. The hit's
    // own fields (`name`, `description`, `score`, `snippet`, …) are real table entries, so `__index` is
    // only consulted for the handle behavior and never shadows the hit's carried data. The handle
    // `__index` is fully wired by the time `memory.search` is installed (see `install_block_api`).
    result_metatable.raw_set("__index", metatable.raw_get::<Value>("__index")?)?;
    memory.set(
        "search",
        lua.create_async_function({
            let api = api.clone();
            let result_metatable = result_metatable.clone();
            move |lua, (query, opts): (Value, Value)| {
                let api = api.clone();
                let result_metatable = result_metatable.clone();
                async move {
                    let query: String = arg(
                        &lua,
                        query,
                        "memory.search",
                        "a query string",
                        "pass the search text directly, memory.search(\"dave\")",
                    )?;
                    check_interpolated("search query", &query)?;
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
                        // The memory's id backs the hit's double life as a handle: the result
                        // metatable's `__index` falls through to the memory-handle methods, which
                        // resolve their memory from this field (see `handle_id`). It is a raw field
                        // so the readonly `__newindex` never intercepts writing it here.
                        table.raw_set("id", row.id.0.to_string())?;
                        // The query this hit came from, hidden on the result so the fuzzy-write guard
                        // can verify a later write's target names it (see `guard_search_write`). A raw
                        // field, like `id`: it neither renders nor trips the read-only `__newindex`.
                        table.raw_set(SEARCH_QUERY_FIELD, query.as_str())?;
                        // A hit the query did not name taints its memory for the rest of the block, so a
                        // write to it through *any* handle — not just this hit — is refused
                        // (`guard_search_taint`). First query to surface a given name wins, so its
                        // message names the search the agent actually ran.
                        if !query_names_handle(&query, &row.name) {
                            api.search_taint
                                .lock()
                                .entry(row.name.clone())
                                .or_insert_with(|| query.clone());
                        }
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
    // memory.list(prefix) — the live memories whose handle begins with `prefix`, alphabetical, as
    // lightweight handles (name/description read lazily, full methods, the list printing as its lines).
    // A `same_as` identity spanning a canonical profile and its platform stubs collapses to one row
    // under the class primary, so a person lists once rather than as several near-identical stubs.
    // Discovery by stem — which spellings of a name already exist — where memory.search is recall by
    // meaning; reach for it before minting a handle so an existing one is reused, not duplicated under a
    // guessed variant. The prefix is required and matched literally (its `%`/`_` do not wildcard); a
    // blank one is a teachable error. Capped at [`LIST_CAP`], the remainder noted in the rendered form.
    let list_metatable = handle_list_metatable(lua)?;
    memory.set(
        "list",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            let list_metatable = list_metatable.clone();
            move |lua, prefix: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                let list_metatable = list_metatable.clone();
                async move {
                    let prefix: Option<String> = arg(
                        &lua,
                        prefix,
                        "memory.list",
                        "a name-prefix string like \"person/\"",
                        "memory.list(\"person/\") to find handles by stem",
                    )?;
                    let prefix = prefix.unwrap_or_default();
                    if prefix.trim().is_empty() {
                        return Err(ListError::EmptyPrefix.into());
                    }
                    check_interpolated("name prefix", &prefix)?;
                    let rows = api
                        .block
                        .lock()
                        .list_by_prefix(&prefix)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    let total = rows.len();
                    let capped: Vec<_> = rows.into_iter().take(LIST_CAP).collect();
                    let more = total.saturating_sub(capped.len());
                    // Lock the handles the list hands back, like the calendar readers: the query read
                    // them and each is actionable, so a concurrent write serializes on them. A collapsed
                    // class locks on its primary, the id the row's handle carries.
                    let ids: Vec<_> = capped.iter().map(|row| row.id).collect();
                    api.lock_all(ids).await;
                    make_capped_listed_handle_list(&lua, capped, more, &metatable, &list_metatable)
                }
            }
        })?,
    )?;
    Ok(memory)
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
    // readers). Committed-only; visibility-filtered through `link_visible` when an audience is present,
    // mirroring `<memory>:links`. Written with
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
