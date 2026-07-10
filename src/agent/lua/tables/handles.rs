//! `install_handle_methods`: the `mem:*` handle methods on the metatable's `methods` table.

use super::*;

/// The `mem:*` handle methods (`append`, `entries`, `history`, `supersede`, `revise`) on the
/// metatable's `methods` table. Each acts on the handle passed as `this`. `entry_metatable`
/// backs the entry handles the content reads and `append` return.
///
/// `features` gates the link readers (`:outgoing`, `:incoming`, `:links`), merging
/// (`:propose_merge`), and tagging (`:tag`, `:untag`) methods. Memory methods (`:append`,
/// `:supersede`, `:revise`, `:set_volatility`, `:rename`) are always installed. Link *writes*
/// (`links.create`/`links.remove`) live on the `links` module table rather than on a handle (see
/// [`super::modules::links_table`]), so both endpoints read as explicit arguments and neither is a
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
            move |lua, (this, text, opts): (HandleSelf, String, Value)| {
                let api = api.clone();
                let entry_metatable = entry_metatable.clone();
                async move {
                    check_interpolated("entry text", &text)?;
                    let id = handle_id(&this.0)?;
                    api.lock(id).await;
                    let opts = append_options_from_lua(&api, &lua, opts)?.unwrap_or_default();
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
                    check_interpolated("entry text", &text)?;
                    let id = handle_id(&this.0)?;
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
    // (see [`super::modules::links_table`]). The link *readers* stay handle methods, gated on the same
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
                    check_interpolated("memory name", &new_name)?;
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
