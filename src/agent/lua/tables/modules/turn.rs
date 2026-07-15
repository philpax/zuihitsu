//! The `turn` global: `skip(reason)`, which commits the block's buffered writes but signals the
//! turn to end silently. It touches no memory, so it stays a synchronous function and takes no lock.

use super::*;

/// The `turn` global: `skip(reason)`, which commits the block's buffered writes but ends the turn
/// silently. Unlike `block.abort` (which discards the buffer), a skip commits — the agent may have
/// done useful memory writes before deciding not to respond.
pub(in crate::agent::lua) fn turn_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let turn_tbl = lua.create_table()?;
    turn_tbl.set(
        "skip",
        lua.create_function({
            let block = api.block.clone();
            move |_, reason: Option<String>| {
                block.lock().skip(reason);
                Err::<(), _>(mlua::Error::RuntimeError("turn skipped".to_owned()))
            }
        })?,
    )?;
    Ok(turn_tbl)
}
