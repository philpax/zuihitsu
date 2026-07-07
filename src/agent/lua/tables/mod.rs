//! The free-function builders that mint the per-block Lua globals, their handle metatables, and the
//! `mem:*` handle methods. These translate script calls into [`MemoryBlock`] transaction calls over the
//! shared [`BlockApi`] seam; they never touch the buffer, the events, or the visibility rules directly.

mod block_api;
mod handles;
mod metatables;
mod modules;

pub(super) use mlua::{Lua, LuaSerdeExt, Table, Value};
pub(super) use ulid::Ulid;

pub(super) use crate::{
    InstanceFeatures,
    agent::turn::{ResolvedTurn, TurnResolution, TurnWindow, resolve_turn},
    event::TurnRole,
    ids::{MemoryName, TurnId},
    memory::memory_block::{LinkDirection, RelationSpec},
    time,
    vocabulary::{RelationName, TagName},
};

pub(super) use super::{
    error::{BlockConsistencyError, CalendarError, HandleKind, ListError, TurnResolveError},
    runtime::{
        BlockApi, HandleSelf, SearchOpts, append_options_from_lua, concat_via_tostring, date_text,
        day_string, entry_handle_id, get_argument_name, handle_id, link_target_id,
        make_capped_handle_list, make_date, make_entry_handle, make_entry_handle_list, make_handle,
        make_handle_list, make_link_handle_list, make_relation_result, readonly_newindex, render,
        render_details, render_neighborhood, render_salient_relations, route_error,
        run_memory_search, value_text,
    },
};

/// How many turns before and after the focal turn `convo.turn` includes in its window — a few on
/// each side, enough to place the linked moment in its immediate exchange without replaying the room.
pub(super) const TURN_WINDOW_BEFORE: usize = 3;
pub(super) const TURN_WINDOW_AFTER: usize = 3;

pub(super) use metatables::entry_metatable;
use metatables::*;
use modules::*;

pub(super) use block_api::install_block_api;
