//! The `turn` global: `skip(reason)`, which commits the block's buffered writes but signals the
//! turn to end silently. It touches no memory, so it stays a synchronous function and takes no lock.

use std::fmt;

use super::*;

/// The error raised by `turn.skip()` to stop Lua execution. Unlike `block.abort`'s
/// `RuntimeError`, this is a typed `External` error so the execute path can distinguish a
/// deliberate skip from a runtime error by downcast, not by checking the block's `skip` field.
#[derive(Debug)]
pub(in crate::agent::lua) struct TurnSkip(pub Option<String>);

impl fmt::Display for TurnSkip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(reason) => write!(f, "turn skipped: {reason}"),
            None => write!(f, "turn skipped"),
        }
    }
}

impl std::error::Error for TurnSkip {}

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
                block.lock().skip(reason.clone());
                Err::<(), _>(mlua::Error::external(TurnSkip(reason)))
            }
        })?,
    )?;
    Ok(turn_tbl)
}
