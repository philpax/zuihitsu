//! The `convo` global: `turn(id)` resolves a conversation turn link to a window of surrounding turns.

use crate::agent::lua::tables::modules::{metatables::*, *};

/// The `convo` global: `turn(id)` resolves a conversation turn link — the id carried in a
/// `[turn:<ulid>]` token, the canonical agent-facing reference form — to that moment and a small
/// window of the surrounding turns in its session. A console deep-link's `?turn=<ulid>` never reaches
/// here: the connector normalizes any pasted URL to the token before the message reaches the agent
/// (see [`turn_ref`](zuihitsu_core::turn_ref)), so this resolver reads a bare ULID and nothing more.
/// The result is a table `{ id, ref, text, speaker, role, at,
/// window }` — the focal turn's fields at the top (`ref` the canonical `[turn:…]` to cite it by), and
/// `window` the ordered surrounding turns (the focal one included, flagged `focused`) — that prints as
/// a readable transcript excerpt so `return convo.turn(id)` reads back as the exchange. Resolution
/// obeys the audience rule: a moment resolves only when everyone present here was in its audience. A
/// malformed id, an id whose moment the present audience did not all share, and an unknown id are three
/// distinct teachable errors (see [`TurnResolveError`]); resolving is read-only and touches no memory,
/// so it takes no lock.
pub(crate) fn convo_table(lua: &Lua, api: &BlockApi) -> mlua::Result<Table> {
    let convo = lua.create_table()?;
    let line_metatable = turn_line_metatable(lua)?;
    let window_metatable = turn_window_metatable(lua)?;
    convo.set(
        "turn",
        lua.create_function({
            let api = api.clone();
            let line_metatable = line_metatable.clone();
            let window_metatable = window_metatable.clone();
            move |lua, id: String| {
                let turn_id = TurnId(Ulid::from_string(&id).map_err(|source| {
                    TurnResolveError::InvalidTurnId {
                        id: id.clone(),
                        source,
                    }
                })?);
                let (engine, present_set) = api.block.lock().turn_resolution_handle();
                let window = match resolve_turn(
                    engine.as_ref(),
                    &present_set,
                    turn_id,
                    TURN_WINDOW_BEFORE,
                    TURN_WINDOW_AFTER,
                )
                .map_err(TurnResolveError::Store)?
                {
                    TurnResolution::Resolved(window) => window,
                    TurnResolution::AudienceMismatch => {
                        return Err(TurnResolveError::AudienceMismatch { id }.into());
                    }
                    TurnResolution::NotFound => {
                        return Err(TurnResolveError::NotFound { id }.into());
                    }
                };
                make_turn_window(lua, &window, &line_metatable, &window_metatable)
            }
        })?,
    )?;
    Ok(convo)
}

/// Build the `convo.turn` result: the focal turn's fields at the top, and `window` the ordered
/// surrounding turns (the focal one flagged `focused`), each a line backed by [`turn_line_metatable`].
fn make_turn_window(
    lua: &Lua,
    window: &TurnWindow,
    line_metatable: &Table,
    window_metatable: &Table,
) -> mlua::Result<Table> {
    let list = lua.create_table()?;
    for (index, turn) in window.turns.iter().enumerate() {
        let line = make_turn_line(lua, turn, index == window.focus, line_metatable)?;
        list.set(index + 1, line)?;
    }
    let focus = &window.turns[window.focus];
    let result = lua.create_table()?;
    result.set("id", focus.turn_id.0.to_string())?;
    result.set("ref", focus.reference.as_str())?;
    result.set("text", focus.text.as_str())?;
    result.set("speaker", focus.speaker.as_str())?;
    result.set("role", turn_role_label(focus.role))?;
    result.set("at", time::format_stamp(focus.recorded_at))?;
    result.set("window", list)?;
    result.set_metatable(Some(window_metatable.clone()))?;
    Ok(result)
}

/// One turn in a `convo.turn` window as `{ id, ref, text, speaker, role, at, focused }`, backed by
/// [`turn_line_metatable`] so it prints as a transcript line.
fn make_turn_line(
    lua: &Lua,
    turn: &ResolvedTurn,
    focused: bool,
    line_metatable: &Table,
) -> mlua::Result<Table> {
    let line = lua.create_table()?;
    line.set("id", turn.turn_id.0.to_string())?;
    line.set("ref", turn.reference.as_str())?;
    line.set("text", turn.text.as_str())?;
    line.set("speaker", turn.speaker.as_str())?;
    line.set("role", turn_role_label(turn.role))?;
    line.set("at", time::format_stamp(turn.recorded_at))?;
    line.set("focused", focused)?;
    line.set_metatable(Some(line_metatable.clone()))?;
    Ok(line)
}

/// The agent-facing role label for a resolved turn — the string a script branches on.
fn turn_role_label(role: TurnRole) -> &'static str {
    match role {
        TurnRole::Participant => "participant",
        TurnRole::Agent => "agent",
        TurnRole::System => "system",
    }
}
