//! The `links` global: `create`/`remove` write and clear edges, `register` adds a relation to the
//! schema, and `list`/`get` read the registry.

use super::{metatables::*, *};

/// The `links` global. `create`/`remove` instantiate a relation as an edge; `register` adds a
/// relation to the schema, and `list`/`get` read the registry (spec §Link relation registry). A
/// link *edge* is written with `links.create(subject, relation, object[, opts])` — a triadic call
/// whose subject and object are ordinary arguments (neither is a privileged receiver), so the call
/// reads as a sentence and an asymmetric edge is not recorded backwards.
pub(in crate::agent::lua) fn links_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let links = lua.create_table()?;
    let result_metatable = relation_result_metatable(lua)?;
    // links.create(subject, relation, object[, opts]) — record a relation such as `knows`, locking
    // both endpoints. Both `subject` and `object` resolve through `link_target_id`, so either may be
    // a memory handle or a name string. The script names the relation as a string; it is recognized
    // into its typed `RelationName` here, at the wrapper boundary. The optional `opts` table carries
    // `visibility` to force the link's posture instead of the write-time default.
    links.set(
        "create",
        lua.create_async_function({
            let api = api.clone();
            move |lua, (subject, relation, object, opts): (Value, String, Value, Value)| {
                let api = api.clone();
                async move {
                    let from = link_target_id(&api, subject)?;
                    let to = link_target_id(&api, object)?;
                    api.lock_all([from, to]).await;
                    let opts = link_options_from_lua(&api, &lua, opts)?;
                    api.block
                        .lock()
                        .link(from, to, RelationName::new(&relation), opts)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;
    // links.remove(subject, relation, object) — clear a relation made with `links.create`, locking
    // both endpoints. Mirrors `create`'s target resolution: either endpoint may be a handle or a name.
    links.set(
        "remove",
        lua.create_async_function({
            let api = api.clone();
            move |_, (subject, relation, object): (Value, String, Value)| {
                let api = api.clone();
                async move {
                    let from = link_target_id(&api, subject)?;
                    let to = link_target_id(&api, object)?;
                    api.lock_all([from, to]).await;
                    api.block
                        .lock()
                        .unlink(from, to, RelationName::new(&relation))
                        .map_err(|error| route_error(error, &mut api.infra.lock()))
                }
            }
        })?,
    )?;
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
                    check_interpolated("relation name", &spec.name)?;
                    check_interpolated("relation name", &spec.inverse)?;
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
    // (The `create` opts resolver `link_options_from_lua` is defined below the table builder.)
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

/// Deserialize a `links.create` `opts` table into [`LinkOptions`], resolving the `exclude` list of
/// handles or names at the boundary (serde cannot decode a memory handle) before deserializing the
/// rest. The `exclude` key is dropped from a copy so the agent's own table is left untouched, mirroring
/// `append`'s option handling. `nil` opts yield `None`.
fn link_options_from_lua(
    api: &BlockApi,
    lua: &Lua,
    opts: Value,
) -> mlua::Result<Option<LinkOptions>> {
    if opts.is_nil() {
        return Ok(None);
    }
    let Value::Table(table) = &opts else {
        // A non-nil, non-table opts is a shape slip serde surfaces.
        return Ok(Some(lua.from_value(opts)?));
    };
    let exclude = resolve_exclude(api, table.get::<Value>("exclude")?)?;
    let rest = lua.create_table()?;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        if let Value::String(name) = &key
            && name.to_string_lossy() == "exclude"
        {
            continue;
        }
        rest.set(key, value)?;
    }
    let mut options: LinkOptions = lua.from_value(Value::Table(rest))?;
    options.exclude = exclude;
    Ok(Some(options))
}
