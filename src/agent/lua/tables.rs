//! The `impl Session` builders that mint the per-block Lua globals, their handle metatables, and the
//! `mem:*` handle methods. These translate script calls into [`MemoryBlock`] transaction calls over the
//! shared [`BlockApi`] seam; they never touch the buffer, the events, or the visibility rules directly.

use mlua::{LuaSerdeExt, Table, Value};

use crate::{
    memory::memory_block::{AppendOptions, RelationSpec},
    vocabulary::{RelationName, TagName},
};

use super::{
    Session,
    runtime::{
        BlockApi, SearchOpts, entry_handle_id, handle_id, make_entry_handle,
        make_entry_handle_list, make_handle, make_handle_list, make_relation_result, render,
        route_error, run_memory_search, value_text,
    },
};

impl Session {
    /// Install the per-block memory API as `'static` async Lua functions over the shared [`BlockApi`]
    /// seam. Before its operation, each function acquires the lock on every memory it touches and holds
    /// the owned guard (in `api.lock_set`) to block end, so a concurrent block in another conversation
    /// serializes on a shared memory (spec §Concurrency). A graph-read failure is routed to `api.infra`
    /// (infrastructure, bubbled up); a teachable violation becomes the Lua runtime error the agent sees.
    /// The handle `metatable`/`methods` tables back every minted memory handle. The registration is
    /// split table by table so each group stays legible.
    pub(super) fn install_block_api(
        &self,
        api: &BlockApi,
        methods: &Table,
        metatable: &Table,
        entry_metatable: &Table,
    ) -> mlua::Result<()> {
        self.install_handle_methods(api, methods, entry_metatable)?;
        let globals = self.lua.globals();
        // `print(...)` captures into the block's output buffer (rendered the same way returned values
        // are), so the agent sees what it prints fed back — Lua's default `print` writes to a process
        // stdout the model never reads. Tab-separated args, newline-terminated, matching Lua semantics.
        globals.set(
            "print",
            self.lua.create_function({
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
        globals.set("memory", self.memory_table(api, metatable)?)?;
        globals.set("block", self.block_table(api)?)?;
        globals.set("context", self.context_table(api, metatable)?)?;
        globals.set("calendar", self.calendar_table(api, metatable)?)?;
        globals.set("tags", self.tags_table(api)?)?;
        globals.set("links", self.links_table(api)?)?;
        Ok(())
    }

    /// The `mem:*` handle methods (`append`, `entries`, `history`, `supersede`, `link`, `unlink`) on
    /// the metatable's `methods` table. Each acts on the handle passed as `this`. `entry_metatable`
    /// backs the entry handles the content reads and `append` return.
    fn install_handle_methods(
        &self,
        api: &BlockApi,
        methods: &Table,
        entry_metatable: &Table,
    ) -> mlua::Result<()> {
        // mem:append(text[, opts]) — `opts` is the typed override struct, deserialized from the table.
        // Locks the target memory before writing it. Returns the new entry as an addressable handle.
        methods.set(
            "append",
            self.lua.create_async_function({
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                move |lua, (this, text, opts): (Table, String, Value)| {
                    let api = api.clone();
                    let entry_metatable = entry_metatable.clone();
                    async move {
                        let id = handle_id(&this)?;
                        api.lock(id).await;
                        let opts: AppendOptions = if opts.is_nil() {
                            AppendOptions::default()
                        } else {
                            lua.from_value(opts)?
                        };
                        let entry = {
                            let mut block = api.block.lock();
                            let entry_id = block
                                .append(id, &text, opts)
                                .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                            block.entry_ref_by_id(entry_id)
                        };
                        let entry = entry.ok_or_else(|| {
                            mlua::Error::runtime(
                                "the appended entry was not found in the block buffer",
                            )
                        })?;
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
            self.lua.create_async_function({
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                move |lua, this: Table| {
                    let api = api.clone();
                    let entry_metatable = entry_metatable.clone();
                    async move {
                        let id = handle_id(&this)?;
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
            self.lua.create_async_function({
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                move |lua, this: Table| {
                    let api = api.clone();
                    let entry_metatable = entry_metatable.clone();
                    async move {
                        let id = handle_id(&this)?;
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
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, old, new): (Table, Table, Table)| {
                    let api = api.clone();
                    async move {
                        let id = handle_id(&this)?;
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

        // mem:link(relation, other) / mem:unlink(relation, other) — flag (or clear) a relation such
        // as `active_in`, locking both endpoints. The script names the relation as a string; it is
        // recognized into its typed [`RelationName`] here, at the wrapper boundary.
        methods.set(
            "link",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, relation, other): (Table, String, Table)| {
                    let api = api.clone();
                    async move {
                        let (from, to) = (handle_id(&this)?, handle_id(&other)?);
                        api.lock_all([from, to]).await;
                        api.block
                            .lock()
                            .link(from, to, RelationName::new(relation))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
        methods.set(
            "unlink",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, relation, other): (Table, String, Table)| {
                    let api = api.clone();
                    async move {
                        let (from, to) = (handle_id(&this)?, handle_id(&other)?);
                        api.lock_all([from, to]).await;
                        api.block
                            .lock()
                            .unlink(from, to, RelationName::new(relation))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;

        // mem:tag(name) / mem:untag(name) — apply or clear a vocabulary tag on this memory, locking it
        // first. The tag must have been created (`tags.create`); the name is recognized into its typed
        // [`TagName`] here, at the wrapper boundary.
        methods.set(
            "tag",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, name): (Table, String)| {
                    let api = api.clone();
                    async move {
                        let id = handle_id(&this)?;
                        api.lock(id).await;
                        api.block
                            .lock()
                            .tag(id, TagName::new(name))
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
        methods.set(
            "untag",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (this, name): (Table, String)| {
                    let api = api.clone();
                    async move {
                        let id = handle_id(&this)?;
                        api.lock(id).await;
                        api.block
                            .lock()
                            .untag(id, TagName::new(name))
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
    pub(super) fn entry_metatable(&self) -> mlua::Result<Table> {
        let metatable = self.lua.create_table()?;
        // An entry renders self-describingly: its text prefixed by what governs sharing it — the
        // visibility and who it came from, and a `disputed` marker when the fact is under an unresolved
        // arbitration, e.g. "[disputed · private · from person/erin] …". So printing a memory's entries
        // shows at a glance which are confidences to hold, whose they are, and which are contested,
        // rather than bare text the agent has to reason about the provenance of separately.
        metatable.set(
            "__tostring",
            self.lua.create_function(|_, this: Table| {
                let text = this.get::<String>("text")?;
                let disputed = this.get::<Option<bool>>("disputed")?.unwrap_or(false);
                let prefix = match (
                    this.get::<Option<String>>("visibility")?,
                    this.get::<Option<String>>("told_by")?,
                ) {
                    (Some(visibility), Some(teller)) if disputed => {
                        Some(format!("disputed · {visibility} · from {teller}"))
                    }
                    (Some(visibility), Some(teller)) => {
                        Some(format!("{visibility} · from {teller}"))
                    }
                    _ => disputed.then(|| "disputed".to_owned()),
                };
                Ok(match prefix {
                    Some(prefix) => format!("[{prefix}] {text}"),
                    None => text,
                })
            })?,
        )?;
        metatable.set(
            "__concat",
            self.lua
                .create_function(|lua, (left, right): (Value, Value)| {
                    Ok(format!(
                        "{}{}",
                        value_text(lua, &left)?,
                        value_text(lua, &right)?
                    ))
                })?,
        )?;
        Ok(metatable)
    }

    /// The metatable backing `memory.search` result objects: `__tostring` renders one as a readable
    /// line (name, score, description, and any teller-private marker), so returning the result list
    /// reads back as text rather than `<table>` while each result keeps its fields for the agent to
    /// inspect (`result.name` to fetch, `result.score` to weigh).
    fn search_result_metatable(&self) -> mlua::Result<Table> {
        let metatable = self.lua.create_table()?;
        metatable.set(
            "__tostring",
            self.lua.create_function(|_, this: Table| {
                let name: String = this.get("name")?;
                let description: String = this.get("description")?;
                let score: f32 = this.get("score")?;
                let marker: Option<String> = this.get("marker")?;
                let mut line = format!("{name} (score {score:.2})");
                if !description.is_empty() {
                    line.push_str(" — ");
                    line.push_str(&description);
                }
                if let Some(marker) = marker {
                    line.push(' ');
                    line.push_str(&marker);
                }
                Ok(line)
            })?,
        )?;
        Ok(metatable)
    }

    /// The metatable backing `tags.list` result objects: `__tostring` renders one as `name — purpose
    /// (N uses)`, so the vocabulary reads back as text rather than `<table>` while each result keeps
    /// its `name`, `description`, and `count` fields.
    fn tag_result_metatable(&self) -> mlua::Result<Table> {
        let metatable = self.lua.create_table()?;
        metatable.set(
            "__tostring",
            self.lua.create_function(|_, this: Table| {
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

    /// The `tags` global: `create` and `describe` mutate the vocabulary, `list` reads it. Creation and
    /// application are deliberately distinct — applying (`mem:tag`) never mutates a tag's description,
    /// creating always forces a purpose (spec §Tag operations).
    pub(super) fn tags_table(&self, api: &BlockApi) -> mlua::Result<Table> {
        let tags = self.lua.create_table()?;
        // tags.create(name, description) — add a tag to the vocabulary with a one-line purpose.
        tags.set(
            "create",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (name, description): (String, String)| {
                    let api = api.clone();
                    async move {
                        api.block
                            .lock()
                            .create_tag(TagName::new(name), &description)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
        // tags.describe(name, description) — change an existing tag's purpose.
        tags.set(
            "describe",
            self.lua.create_async_function({
                let api = api.clone();
                move |_, (name, description): (String, String)| {
                    let api = api.clone();
                    async move {
                        api.block
                            .lock()
                            .describe_tag(TagName::new(name), &description)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))
                    }
                }
            })?,
        )?;
        // tags.list() — the whole vocabulary, each result `{ name, description, count }` printing as a
        // readable line.
        let result_metatable = self.tag_result_metatable()?;
        tags.set(
            "list",
            self.lua.create_async_function({
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
    fn relation_result_metatable(&self) -> mlua::Result<Table> {
        let metatable = self.lua.create_table()?;
        metatable.set(
            "__tostring",
            self.lua.create_function(|_, this: Table| {
                let name: String = this.get("name")?;
                let inverse: String = this.get("inverse")?;
                let from_card: String = this.get("from_card")?;
                let to_card: String = this.get("to_card")?;
                let symmetric: bool = this.get("symmetric")?;
                let reflexive: bool = this.get("reflexive")?;
                let mut line = format!("{name} / {inverse} — {from_card}-to-{to_card}");
                if symmetric {
                    line.push_str(", symmetric");
                }
                if reflexive {
                    line.push_str(", reflexive");
                }
                Ok(line)
            })?,
        )?;
        Ok(metatable)
    }

    /// The `links` global: `register` adds a relation to the schema, `list` and `get` read it. Link
    /// *edges* are made on memory handles (`mem:link`/`mem:unlink`); this global manages the relation
    /// *registry* they instantiate (spec §Link relation registry).
    pub(super) fn links_table(&self, api: &BlockApi) -> mlua::Result<Table> {
        let links = self.lua.create_table()?;
        let result_metatable = self.relation_result_metatable()?;
        // links.register{ name, inverse, from_card, to_card, symmetric?, reflexive? } — register one
        // relation, accessible under either label; the inverse view's cardinality is computed.
        links.set(
            "register",
            self.lua.create_async_function({
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
            self.lua.create_async_function({
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
            self.lua.create_async_function({
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

    /// The `memory` global: `create` and `get`, both of which mint handles (hence the metatable).
    pub(super) fn memory_table(&self, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
        let memory = self.lua.create_table()?;
        // memory.create(name[, content]) — create a memory and optionally its first entry, then lock
        // the freshly-minted id (uncontended — no other block knows it yet).
        memory.set(
            "create",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, (name, content): (String, Option<String>)| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let id = api
                            .block
                            .lock()
                            .create(&name, content.as_deref())
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        api.lock(id).await;
                        make_handle(&lua, id, &metatable)
                    }
                }
            })?,
        )?;
        // memory.get(name) — resolve through the block's pending creates, then the graph, locking the
        // resolved stub.
        memory.set(
            "get",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, name: String| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let resolved = api
                            .block
                            .lock()
                            .get(&name)
                            .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                        match resolved {
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
        // memory.search(query[, opts]) — semantic + lexical recall over the agent's whole memory,
        // visibility-filtered against who is present (a teller-private hit only surfaces while its
        // teller is here, with a marker). Embeds the query off any lock, then ranks under a brief read
        // lock. Returns a list of result objects (`{ name, description, score, marker? }`), best first;
        // each prints as a readable line so `return memory.search(...)` reads back the results rather
        // than `<table>`.
        let result_metatable = self.search_result_metatable()?;
        memory.set(
            "search",
            self.lua.create_async_function({
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
                        let rows = run_memory_search(&engine, &present_set, &query, &opts)
                            .await
                            .map_err(mlua::Error::RuntimeError)?;
                        let list = lua.create_table()?;
                        for (index, row) in rows.into_iter().enumerate() {
                            let table = lua.create_table()?;
                            table.set("name", row.name)?;
                            table.set("description", row.description)?;
                            table.set("score", row.score)?;
                            if let Some(marker) = row.marker {
                                table.set("marker", marker)?;
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
    pub(super) fn block_table(&self, api: &BlockApi) -> mlua::Result<Table> {
        let block_tbl = self.lua.create_table()?;
        block_tbl.set(
            "abort",
            self.lua.create_function({
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
    pub(super) fn context_table(&self, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
        let context = self.lua.create_table()?;
        context.set(
            "current",
            self.lua.create_async_function({
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

    /// The `calendar` global: `upcoming`, `on`, and `recurring`, each returning a list of memory
    /// handles, soonest first. Unlike the brief's `<upcoming/>` block these are the agent's own
    /// queries and are not visibility-filtered (like `mem:entries`, the agent sees its whole memory).
    /// Strict locking: each returned memory is locked, since the query read (and touched) it.
    pub(super) fn calendar_table(&self, api: &BlockApi, metatable: &Table) -> mlua::Result<Table> {
        let calendar = self.lua.create_table()?;
        calendar.set(
            "upcoming",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, opts: Option<Table>| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
                        let within: Option<String> = match opts {
                            Some(table) => table.get("within")?,
                            None => None,
                        };
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
            "on",
            self.lua.create_async_function({
                let api = api.clone();
                let metatable = metatable.clone();
                move |lua, date: String| {
                    let api = api.clone();
                    let metatable = metatable.clone();
                    async move {
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
            self.lua.create_async_function({
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
        Ok(calendar)
    }
}
