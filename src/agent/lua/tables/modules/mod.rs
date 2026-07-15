//! The per-block Lua module tables — `memory`, `block`, `context`, `calendar`, `tags`, `links`,
//! `convo`, and `turn` — and the helpers that assemble their rows.

use super::*;

mod block;
mod calendar;
mod convo;
mod links;
mod memory;
mod tags;
mod turn;
mod web;

pub(in crate::agent::lua) use block::{block_table, context_table};
pub(in crate::agent::lua) use calendar::calendar_table;
pub(in crate::agent::lua) use convo::convo_table;
pub(in crate::agent::lua) use links::links_table;
pub(in crate::agent::lua) use memory::memory_table;
pub(in crate::agent::lua) use tags::tags_table;
pub(in crate::agent::lua) use turn::turn_table;
pub(in crate::agent::lua) use web::web_table;
