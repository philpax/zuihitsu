//! The `tags` global: `create` and `describe` mutate the vocabulary, `list` reads it.

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
