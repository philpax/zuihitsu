//! `install_handle_methods`: the `mem:*` handle methods on the metatable's `methods` table.

use crate::agent::lua::tables::*;

/// The `mem:*` handle methods (`append`, `entries`, `find_entry`, `history`, `supersede`, `retract`,
/// `revise`) on the
/// metatable's `methods` table. Each acts on the handle passed as `this`. `entry_metatable`
/// backs the entry handles the content reads and `append` return.
///
/// `features` gates the link readers (`:outgoing`, `:incoming`, `:links`), merging
/// (`:propose_merge`), and tagging (`:tag`, `:untag`) methods. Memory methods (`:append`,
/// `:supersede`, `:retract`, `:revise`, `:set_volatility`, `:rename`) are always installed. The content
/// writers (`:append`, `:supersede`, `:retract`, `:revise`) each run [`guard_search_write`] on their
/// receiver first, so a write through a `memory.search` hit the query did not name is refused before it
/// commits — the fuzzy-write guard. Link *writes*
/// (`links.create`/`links.remove`) live on the `links` module table rather than on a handle (see
/// [`crate::agent::lua::tables::modules::links_table`]), so both endpoints read as explicit arguments and neither is a
/// privileged receiver.
pub(super) fn install_handle_methods(
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
            move |lua, (this, text, opts): (HandleSelf, Value, Value)| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    let text: String = arg(
                        &lua,
                        text,
                        "mem:append",
                        "the entry text as a string",
                        "mem:append(\"leads the volcano project\")",
                    )?;
                    check_interpolated("entry text", &text)?;
                    guard_search_write(&this.0)?;
                    let id = handle_id(&this.0)?;
                    guard_search_taint(&api, id)?;
                    api.lock(id).await;
                    let opts = append_options_from_lua(&api, &lua, opts)?.unwrap_or_default();
                    // Embed the candidate for the dedup check, if retrieval is configured. The
                    // embedding is computed off the block lock (it is async), then passed into
                    // `append_dedup` which searches the vector index under a brief sync lock.
                    let (engine, _) = api.block.lock().retrieval_handle();
                    let embedding = if let Some(retrieval) = &engine.retrieval {
                        retrieval
                            .embedder
                            .embed(std::slice::from_ref(&text))
                            .await
                            .ok()
                            .and_then(|v| v.into_iter().next())
                    } else {
                        None
                    };
                    let entry = {
                        let mut block = api.block.lock();
                        let entry_id = block
                            .append_dedup(id, &text, opts, embedding.as_deref())
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

    // mem:find_entry(text) — the one live entry whose text contains `text`, matched case-insensitively
    // and diacritic-folded (the same fold the fuzzy-write guard uses). Reads exactly the set
    // `entries()` returns (the merged identity's live entries plus this block's pending appends), so
    // the model can locate an entry by a phrase it composed rather than text-scanning the list itself —
    // the idiom that silently misses on case and paraphrase. A lone match returns that entry handle; no
    // match returns nil; several is a teachable error listing each candidate, since silently taking the
    // first is the correct-the-wrong-entry hazard. A class-traversing read, so it locks the whole
    // `same_as` class like `entries`.
    methods.set(
        "find_entry",
        lua.create_async_function({
            let api = api.clone();
            let entry_metatable = entry_metatable.clone();
            move |lua, (this, text): (HandleSelf, Value)| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    let text: String = arg(
                        &lua,
                        text,
                        "mem:find_entry",
                        "a distinctive phrase string from the entry",
                        "mem:find_entry(\"leads the volcano project\")",
                    )?;
                    let needle = fold_lower(text.trim());
                    if needle.is_empty() {
                        return Err(FindEntryError::EmptyNeedle.into());
                    }
                    let id = handle_id(&this.0)?;
                    api.lock_class(id).await?;
                    let entries = api
                        .block
                        .lock()
                        .entries(id)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    let mut matches = entries
                        .into_iter()
                        .filter(|entry| fold_lower(&entry.text).contains(&needle));
                    let Some(first) = matches.next() else {
                        return Ok(Value::Nil);
                    };
                    let rest: Vec<_> = matches.collect();
                    if rest.is_empty() {
                        return Ok(Value::Table(make_entry_handle(
                            &lua,
                            &first,
                            &entry_metatable,
                        )?));
                    }
                    let candidates = std::iter::once(first)
                        .chain(rest)
                        .map(|entry| (entry.entry_id, entry.text))
                        .collect();
                    Err(FindEntryError::Ambiguous {
                        needle: text.trim().to_owned(),
                        candidates,
                    }
                    .into())
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

    // mem:supersede(old, new) — correct or retract a fact: mark `old` superseded by `new`. Each is an
    // entry handle read from this memory, or its id (or a unique id prefix) as a string — the id the
    // rendered entry line leads with. Locks the whole class, since it validates against and mutates the
    // merged identity's entries.
    methods.set(
        "supersede",
        lua.create_async_function({
            let api = api.clone();
            move |_, (this, old, new): (HandleSelf, Value, Value)| {
                let api = api.clone();
                async move {
                    guard_search_write(&this.0)?;
                    let id = handle_id(&this.0)?;
                    guard_search_taint(&api, id)?;
                    let (old, new) = (entry_selector(&old)?, entry_selector(&new)?);
                    api.lock_class(id).await?;
                    api.block
                        .lock()
                        .supersede(id, old, new)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;

    // mem:retract(entry, reason) — withdraw a fact outright to a tombstone, recording why (an entry
    // handle, or its id or a unique id prefix as a string, like supersede's argument). Unlike
    // supersede there is no
    // replacement: this is the honest fix when a fact was filed on the wrong memory — retract it here
    // and re-assert it on the right memory with a fresh append. Runs the same fuzzy-write guard and
    // block taint as the other content writers, and locks the whole class (the block validates the
    // entry against the merged identity and runs the foreign-confidence guard).
    methods.set(
        "retract",
        lua.create_async_function({
            let api = api.clone();
            move |lua, (this, entry, reason): (HandleSelf, Value, Value)| {
                let api = api.clone();
                async move {
                    let reason: String = arg(
                        &lua,
                        reason,
                        "mem:retract",
                        "the reason as a string",
                        "mem:retract(entry, \"filed on the wrong memory\")",
                    )?;
                    check_interpolated("retraction reason", &reason)?;
                    guard_search_write(&this.0)?;
                    let id = handle_id(&this.0)?;
                    guard_search_taint(&api, id)?;
                    let entry = entry_selector(&entry)?;
                    api.lock_class(id).await?;
                    api.block
                        .lock()
                        .retract(id, entry, &reason)
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
            move |lua, (this, old, text, opts): (HandleSelf, Value, Value, Value)| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    let old: Table = arg(
                        &lua,
                        old,
                        "mem:revise",
                        "the entry handle being corrected (from mem:entries or mem:find_entry)",
                        "mem:revise(mem:find_entry(\"old text\"), \"corrected text\")",
                    )?;
                    let text: String = arg(
                        &lua,
                        text,
                        "mem:revise",
                        "the corrected text as a string",
                        "mem:revise(entry, \"corrected text\")",
                    )?;
                    check_interpolated("entry text", &text)?;
                    guard_search_write(&this.0)?;
                    let id = handle_id(&this.0)?;
                    guard_search_taint(&api, id)?;
                    let old = entry_handle_id(&old)?;
                    api.lock_class(id).await?;
                    let opts = append_options_from_lua(&api, &lua, opts)?.unwrap_or_default();
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

    // The link *writers* (`links.create`/`links.remove`) live on the `links` module table, not on a
    // handle, so the subject and object read as explicit arguments with neither a privileged receiver
    // (see [`crate::agent::lua::tables::modules::links_table`]). The link *readers* stay handle methods, gated on the same
    // `linking` feature.
    if features.linking {
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
                    move |lua, (this, relation): (HandleSelf, Value)| {
                        let api = api.clone();
                        let memory_metatable = memory_metatable.clone();
                        let link_metatable = link_metatable.clone();
                        async move {
                            let relation: String = arg(
                                &lua,
                                relation,
                                if incoming {
                                    "mem:incoming"
                                } else {
                                    "mem:outgoing"
                                },
                                "a relation name string",
                                if incoming {
                                    "mem:incoming(\"knows\")"
                                } else {
                                    "mem:outgoing(\"knows\")"
                                },
                            )?;
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
    // across platforms, for the operator to weigh on the evidence and confirm. `opts.rationale` states
    // the grounds for the match, the proposer's stated claim, not evidence in itself. Not a merge: it
    // surfaces nothing and merges nothing until the operator confirms it. Locks both endpoints.
    if features.merging {
        methods.set(
            "propose_merge",
            lua.create_async_function({
                let api = api.clone();
                move |lua, (this, other, opts): (HandleSelf, Value, Option<Table>)| {
                    let api = api.clone();
                    async move {
                        let other: Table = arg(
                            &lua,
                            other,
                            "mem:propose_merge",
                            "the other memory's handle (from memory.get or memory.create)",
                            "mem:propose_merge(memory.get(\"person/dave@slack\"))",
                        )?;
                        let (from, to) = (handle_id(&this.0)?, handle_id(&other)?);
                        let rationale = match opts {
                            Some(opts) => opts
                                .get::<Option<String>>("rationale")?
                                .map(|text| text.trim().to_owned())
                                .filter(|text| !text.is_empty()),
                            None => None,
                        };
                        if let Some(rationale) = &rationale {
                            check_interpolated("merge rationale", rationale)?;
                        }
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
                move |lua, (this, name): (HandleSelf, Value)| {
                    let api = api.clone();
                    async move {
                        let name: String = arg(
                            &lua,
                            name,
                            "mem:tag",
                            "a tag name string",
                            "mem:tag(\"priority\")",
                        )?;
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
                move |lua, (this, name): (HandleSelf, Value)| {
                    let api = api.clone();
                    async move {
                        let name: String = arg(
                            &lua,
                            name,
                            "mem:untag",
                            "a tag name string",
                            "mem:untag(\"priority\")",
                        )?;
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
    // decay). The level is parsed in the block so an unknown level is a teachable error. Deliberately
    // outside the fuzzy-write guards: volatility carries no content and no identity, so a mis-aimed
    // set is low-harm and freely correctable, unlike the guarded writers.
    methods.set(
        "set_volatility",
        lua.create_async_function({
            let api = api.clone();
            move |lua, (this, level): (HandleSelf, Value)| {
                let api = api.clone();
                async move {
                    let level: String = arg(
                        &lua,
                        level,
                        "mem:set_volatility",
                        "one of the level strings \"low\", \"medium\", or \"high\"",
                        "mem:set_volatility(\"high\")",
                    )?;
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
            move |lua, (this, new_name): (HandleSelf, Value)| {
                let api = api.clone();
                async move {
                    let new_name: String = arg(
                        &lua,
                        new_name,
                        "mem:rename",
                        "the new handle as a string",
                        "mem:rename(\"person/sarah\")",
                    )?;
                    check_interpolated("memory name", &new_name)?;
                    // Renaming rewrites identity, so it is guarded like the content writers: a rename
                    // through a mismatched search hit — or of a name this block's searches tainted —
                    // would also launder the taint (the map is keyed by name, and the write after a
                    // rename would look up the new one), so both guards run before the rename mutates.
                    guard_search_write(&this.0)?;
                    let id = handle_id(&this.0)?;
                    guard_search_taint(&api, id)?;
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
