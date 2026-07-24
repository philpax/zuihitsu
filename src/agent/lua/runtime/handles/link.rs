//! Link handles and neighborhood rendering, plus the target resolution the block's write sites lean
//! on: minting a link result from a [`LinkRef`], the compact neighborhood and salient-relation lines a
//! handle or search hit carries, and resolving a `:link`/`:unlink` target, an `exclude` option, and a
//! `memory.get` argument to memory ids.

use mlua::{Lua, LuaSerdeExt, Table, Value};

use crate::{
    ids::MemoryId,
    memory::{
        memory_block::{LinkDirection, LinkRef},
        search::SalientRelation,
    },
    time::format_occurrence,
};

use std::collections::BTreeSet;

use crate::agent::lua::{
    error::HandleError,
    runtime::{
        BlockApi, check_interpolated,
        handles::{handle_id, make_handle},
        route_error,
    },
};

/// Build a link result `{ relation, memory, name, direction, source }` backed by the link metatable,
/// so a link reader's list prints readably (`relation → name`) while each result keeps the far
/// memory as an actionable handle (`link.memory:append(...)`) and its provenance for the agent to weigh.
pub(crate) fn make_link_handle(
    lua: &Lua,
    link: &LinkRef,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("relation", link.relation.as_str())?;
    table.set("memory", make_handle(lua, link.other, memory_metatable)?)?;
    table.set("name", link.other_name.as_str())?;
    table.set("direction", link_direction_label(link.direction))?;
    table.set("source", link.source.as_str_lowercase())?;
    // The teller who asserted the relationship, for a belief-bearing relation; absent (`nil`) for a
    // link with no teller behind it, like an operator-authored `same_as`.
    if let Some(told_by) = &link.told_by {
        table.set("told_by", told_by.as_str())?;
    }
    // The far memory's representative occurrence, when it holds a dated fact — the same tagged table
    // an entry or search hit carries (e.g. `link.occurred_at.day`), so a script reads a linked event's
    // date, and the metatable's `__tostring` renders it inline on the link line.
    if let Some(occurred_at) = &link.occurred_at {
        table.set("occurred_at", lua.to_value(occurred_at)?)?;
    }
    table.set_metatable(Some(link_metatable.clone()))?;
    Ok(table)
}

/// How many of a memory's links its rendered handle lists before eliding the rest — enough to reveal
/// a hub's shape (its events, its people) without flooding the transcript when a busy topic has many.
pub(crate) const NEIGHBORHOOD_CAP: usize = 8;

/// Render a memory's link neighborhood as the compact line its handle carries: each link as
/// `relation → name` (`←` for an incoming edge), with a dated far memory's occurrence appended as
/// `[when …]` (the same phrasing a search hit's date uses), capped at [`NEIGHBORHOOD_CAP`] with a
/// `(+N more)` note. A name-and-relation list, not the targets' content: it makes the spokes legible
/// at the hub so a recall follows them rather than relaying only the hub's own entries. Empty when the
/// memory has no links, so the caller omits the line entirely.
pub(crate) fn render_neighborhood(links: &[LinkRef]) -> String {
    let mut rendered: Vec<String> = links
        .iter()
        .take(NEIGHBORHOOD_CAP)
        .map(render_link_summary)
        .collect();
    let elided = links.len().saturating_sub(NEIGHBORHOOD_CAP);
    if elided > 0 {
        rendered.push(format!("(+{elided} more)"));
    }
    rendered.join(", ")
}

/// One link on the neighborhood line: `relation → name` (or `←` for an incoming edge), plus the far
/// memory's occurrence as `[when …]` when it holds a dated fact.
fn render_link_summary(link: &LinkRef) -> String {
    let arrow = match link.direction {
        LinkDirection::Outgoing => "→",
        LinkDirection::Incoming => "←",
    };
    let mut summary = format!(
        "{} {arrow} {}",
        link.relation.as_str(),
        link.other_name.as_str()
    );
    if let Some(occurred_at) = &link.occurred_at {
        summary.push_str(&format!(" [when {}]", format_occurrence(occurred_at)));
    }
    summary
}

/// Render a search hit's salient relations as the compact segment its result line carries: each
/// relation as `relation → name` (`←` for an incoming edge), in the salience order (people first, then
/// recency), with a run of same-relation neighbours eliding the repeated label so
/// `participates_in ← person/maya, ← person/nadia` reads cleanly, and a trailing `(+N more)` when links
/// were elided past the cap. The same `relation → name` house style the neighborhood line uses, so a hit
/// passively reveals the cast already on the memory — the recognition signal that steers a search toward
/// reuse over a name-guessed duplicate. `None` when the hit carries no relations, so the caller omits the
/// segment.
pub(crate) fn render_salient_relations(
    relations: &[SalientRelation],
    more: usize,
) -> Option<String> {
    if relations.is_empty() {
        return None;
    }
    let mut rendered: Vec<String> = Vec::with_capacity(relations.len() + 1);
    let mut previous: Option<&str> = None;
    for relation in relations {
        let arrow = match relation.direction {
            LinkDirection::Outgoing => "→",
            LinkDirection::Incoming => "←",
        };
        let name = relation.other_name.as_str();
        let segment = if previous == Some(relation.relation.as_str()) {
            format!("{arrow} {name}")
        } else {
            format!("{} {arrow} {name}", relation.relation.as_str())
        };
        rendered.push(segment);
        previous = Some(relation.relation.as_str());
    }
    if more > 0 {
        rendered.push(format!("(+{more} more)"));
    }
    Some(rendered.join(", "))
}

/// Wrap a list of link refs as a Lua sequence of link results, in order — the
/// `mem:outgoing()`/`incoming()`/`links()` return shape.
pub(crate) fn make_link_handle_list(
    lua: &Lua,
    links: Vec<LinkRef>,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, link) in links.into_iter().enumerate() {
        list.set(
            index + 1,
            make_link_handle(lua, &link, memory_metatable, link_metatable)?,
        )?;
    }
    Ok(Value::Table(list))
}

/// Which way a link runs relative to the memory it was read from, as the agent-facing string a script
/// branches on — `outgoing` when the identity is the edge's source, `incoming` when its target.
fn link_direction_label(direction: LinkDirection) -> &'static str {
    match direction {
        LinkDirection::Outgoing => "outgoing",
        LinkDirection::Incoming => "incoming",
    }
}

/// Resolve a `:link`/`:unlink` target to its memory id. The target is normally a memory handle, but a
/// name string is accepted and looked up too — so the agent's natural call passing a name in place of
/// a handle works rather than failing the string-to-handle argument conversion, erroring, and rolling
/// the whole block back (silently dropping any co-located writes — the cause of lost sensitivity
/// markings). An unknown name is a clear error, not a silent miss.
pub(crate) fn link_target_id(api: &BlockApi, other: Value) -> mlua::Result<MemoryId> {
    match other {
        Value::Table(handle) => handle_id(&handle),
        Value::String(name) => {
            let name = name.to_string_lossy();
            match api
                .block
                .lock()
                .get(&name)
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some((id, _)) => Ok(id),
                None => Err(HandleError::UnknownLinkTarget {
                    name: name.to_string(),
                }
                .into()),
            }
        }
        other => Err(HandleError::WrongLinkTargetType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Resolve an append's or link's `exclude` option to the memory ids the entry is a confidence withheld
/// from whenever they are present — the parties of a `Visibility::Exclude`. Accepts a Lua list
/// (sequence) of person handles or name strings (each resolved through [`resolve_excludee`]), a bare
/// handle or name for the single-party case, or `nil` (no exclusion). An empty list resolves to an
/// empty vec, which the block rejects as a teachable error — an exclude naming no one is just a private
/// confidence, not an exclusion. Mirrors `told_by`'s handle-or-name resolution.
pub(crate) fn resolve_exclude(
    api: &BlockApi,
    value: Value,
) -> mlua::Result<Option<BTreeSet<MemoryId>>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(_) => Ok(Some(BTreeSet::from([resolve_excludee(api, value)?]))),
        Value::Table(table) => {
            // A sequence (a `[1]` element) is a list of parties resolved one by one; a bare handle
            // table (an `id` field, no `[1]`) is a single party; anything else — an empty `{}` — yields
            // an empty set the block turns into the teachable "exclude names no one" error. The excluded
            // parties are a set: naming the same party twice, or in a different order, is the same edge.
            if !table.get::<Value>(1)?.is_nil() {
                let mut ids = BTreeSet::new();
                for element in table.sequence_values::<Value>() {
                    ids.insert(resolve_excludee(api, element?)?);
                }
                Ok(Some(ids))
            } else if table.contains_key("id")? {
                Ok(Some(BTreeSet::from([resolve_excludee(
                    api,
                    Value::Table(table),
                )?])))
            } else {
                Ok(Some(BTreeSet::new()))
            }
        }
        other => Err(HandleError::WrongExcludeeType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Resolve one `exclude` party — a memory handle or a name string — to its memory id. An unknown name,
/// or a value that is neither a handle nor a name, is a teachable error.
fn resolve_excludee(api: &BlockApi, value: Value) -> mlua::Result<MemoryId> {
    match value {
        Value::Table(handle) => handle_id(&handle),
        Value::String(name) => {
            let name = name.to_string_lossy();
            match api
                .block
                .lock()
                .get(&name)
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some((id, _)) => Ok(id),
                None => Err(HandleError::UnknownExcludee {
                    name: name.to_string(),
                }
                .into()),
            }
        }
        other => Err(HandleError::WrongExcludeeType {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Resolve a `memory.get` / `memory.get_or_create` argument to the name to look up. The argument is
/// normally a name string, but an existing memory handle (from `memory.list`, `memory.create`, or a
/// prior `memory.get`) is accepted too — so the natural `memory.get(h)` over a handle the agent
/// already holds works rather than failing the string-to-handle conversion and rolling the whole block
/// back. A handle resolves by its current name (through the block's pending creates, then the graph),
/// so the lookup that follows keeps read semantics identical to `memory.get("name")` — including the
/// renamed-identity affordances. An unknown handle (its id resolves to no memory) is a clear error, as
/// is any other value.
pub(crate) fn get_argument_name(api: &BlockApi, value: Value) -> mlua::Result<String> {
    match value {
        Value::String(name) => {
            let name = name.to_string_lossy();
            check_interpolated("memory name", &name)?;
            Ok(name)
        }
        Value::Table(handle) => {
            let id = handle_id(&handle)?;
            match api
                .block
                .lock()
                .handle_field(id, "name")
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some(name) => Ok(name),
                None => Err(HandleError::UnknownMemoryHandle {
                    id: id.0.to_string(),
                }
                .into()),
            }
        }
        other => Err(HandleError::WrongGetArgType {
            type_name: other.type_name(),
        }
        .into()),
    }
}
