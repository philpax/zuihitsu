//! The `links` global: `register` adds a relation to the schema, `list` and `get` read it.

use super::{metatables::*, *};

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
