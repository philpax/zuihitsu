//! The `block` and `context` globals.

use super::*;

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
