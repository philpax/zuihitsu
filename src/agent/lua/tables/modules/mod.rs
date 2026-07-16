//! The per-block Lua module tables — `memory`, `block`, `context`, `calendar`, `tags`, `links`,
//! `convo`, and `turn` — and the helpers that assemble their rows.

use crate::agent::lua::tables::*;

mod block;
mod calendar;
mod convo;
mod links;
mod memory;
mod tags;
mod turn;
mod web;

pub(crate) use block::{block_table, context_table};
pub(crate) use calendar::calendar_table;
pub(crate) use convo::convo_table;
pub(crate) use links::links_table;
pub(crate) use memory::memory_table;
pub(crate) use tags::tags_table;
pub(crate) use turn::{TurnSkip, turn_table};
pub(crate) use web::web_table;
