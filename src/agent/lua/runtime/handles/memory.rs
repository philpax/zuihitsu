//! Memory handles and their record rendering: minting the `{ id = … }` handle a `mem:*` method binds
//! to, the relation and capped-list result shapes, the whole-record render `mem:details` returns, the
//! `HandleSelf` newtype that turns a dot-call into a teachable hint, and the read-only `__newindex`
//! guard every handle metatable shares.

use mlua::{Lua, Table, Value};

use crate::{
    agent::lua::{
        error::{HandleAssignmentError, HandleError, HandleKind},
        runtime::{
            handles::{make_entry_handle, make_link_handle},
            render,
        },
    },
    graph::RelationView,
    ids::MemoryId,
    memory::memory_block::MemoryDetails,
};
use ulid::Ulid;

/// Build a Lua handle table `{ id = "<ulid>" }` with the memory methods as its metatable index.
pub(crate) fn make_handle(lua: &Lua, id: MemoryId, metatable: &Table) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", id.0.to_string())?;
    handle.set_metatable(Some(metatable.clone()))?;
    Ok(handle)
}

/// Build a relation result `{ name, inverse, from_card, to_card, symmetric, reflexive, description }`
/// backed by the relation metatable, so it prints readably. Cardinalities render lowercase, matching
/// the casing `links.register` accepts.
pub(crate) fn make_relation_result(
    lua: &Lua,
    view: &RelationView,
    metatable: &Table,
) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("name", view.name.as_str())?;
    table.set("inverse", view.inverse.as_str())?;
    table.set("from_card", view.from_card.as_str().to_lowercase())?;
    table.set("to_card", view.to_card.as_str().to_lowercase())?;
    table.set("symmetric", view.symmetric)?;
    table.set("reflexive", view.reflexive)?;
    table.set("description", view.description.as_str())?;
    table.set_metatable(Some(metatable.clone()))?;
    Ok(table)
}

/// Wrap a list of memory ids as a Lua sequence of handles, in order — the `calendar.*` return shape.
pub(crate) fn make_handle_list(
    lua: &Lua,
    ids: Vec<MemoryId>,
    metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        list.set(index + 1, make_handle(lua, id, metatable)?)?;
    }
    Ok(Value::Table(list))
}

/// Wrap a capped list of memory ids as a Lua sequence of handles — the `memory.list` return shape.
/// The value stays a plain sequence the agent can iterate (each element a handle, `handle.name`
/// readable), so `for _, m in ipairs(memory.list("person/")) do … end` works; the truncation note
/// rides only the *rendered* form, through the list metatable's `__tostring` reading the `more`
/// field this stores when matches were elided past the cap. So the returned value is unadorned data
/// while printing or returning it shows the `(+N more — narrow the prefix)` hint.
pub(crate) fn make_capped_handle_list(
    lua: &Lua,
    ids: Vec<MemoryId>,
    more: usize,
    metatable: &Table,
    list_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, id) in ids.into_iter().enumerate() {
        list.set(index + 1, make_handle(lua, id, metatable)?)?;
    }
    if more > 0 {
        list.set("more", more as i64)?;
    }
    list.set_metatable(Some(list_metatable.clone()))?;
    Ok(Value::Table(list))
}

/// Render a memory's whole record to the one string `mem:details` returns: a header line (its name,
/// its description, and a `formerly …` line when it has been renamed), the live entries under a count
/// header, every link in both directions, the applied tags, and the volatility — the sections joined by
/// blank lines. Entries and links render through the *same* handle rendering `mem:entries`/`mem:links`
/// use (each row minted as its handle and stringified through its metatable), so the record reads back
/// exactly as those readers show their rows — date, stale, disputed, visibility, and teller markers on
/// an entry; `relation → name` with a dated occurrence on a link. There is no entry cap: the render is
/// the whole record, which is what lets the agent conclude it holds nothing on a topic after one look.
pub(crate) fn render_details(
    lua: &Lua,
    details: &MemoryDetails,
    entry_metatable: &Table,
    memory_metatable: &Table,
    link_metatable: &Table,
) -> mlua::Result<String> {
    let mut sections: Vec<String> = Vec::new();

    let mut header = details.name.clone();
    if !details.description.is_empty() {
        header.push_str(" — ");
        header.push_str(&details.description);
    }
    if !details.former_names.is_empty() {
        header.push_str(&format!("\nformerly {}", details.former_names.join(", ")));
    }
    sections.push(header);

    // The entries under a count header, each rendered as its own entry handle — the whole class read,
    // teller-private entries marked rather than omitted (this is the agent's own read).
    let count = details.entries.len();
    let mut entry_block = if count == 0 {
        "no entries".to_owned()
    } else {
        format!("{count} {}:", if count == 1 { "entry" } else { "entries" })
    };
    for entry in &details.entries {
        let handle = make_entry_handle(lua, entry, entry_metatable)?;
        entry_block.push('\n');
        entry_block.push_str(&render(lua, &Value::Table(handle)));
    }
    sections.push(entry_block);

    // Every link out of the merged identity in both directions, committed-only — the section is omitted
    // entirely when the memory has none.
    if !details.links.is_empty() {
        let mut link_block = String::from("links:");
        for link in &details.links {
            let handle = make_link_handle(lua, link, memory_metatable, link_metatable)?;
            link_block.push('\n');
            link_block.push_str(&render(lua, &Value::Table(handle)));
        }
        sections.push(link_block);
    }

    if !details.tags.is_empty() {
        let tags = details
            .tags
            .iter()
            .map(|tag| format!("#{}", tag.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        sections.push(format!("tags: {tags}"));
    }

    sections.push(format!("volatility: {}", details.volatility));

    Ok(sections.join("\n\n"))
}

pub(crate) fn handle_id(handle: &Table) -> mlua::Result<MemoryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(MemoryId)
        .map_err(|source| HandleError::InvalidMemoryHandle { id, source }.into())
}

/// The `self` a `mem:*` handle method is invoked on. Extracting it through this newtype — rather than
/// a bare `Table` — is what turns a dot-call (`memory.append(...)`, which binds the first argument to
/// `self`) into the teachable colon hint: as the method's leftmost argument, `self` is converted
/// first, so a non-table `self` fails here (with [`HandleError::MethodCalledWithDot`]) before any
/// later argument's own type error can mask it. A colon call passes the handle table, which converts
/// cleanly; the method body then resolves its id through [`handle_id`].
pub(crate) struct HandleSelf(pub(crate) Table);

impl mlua::FromLua for HandleSelf {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        match value {
            Value::Table(handle) => Ok(HandleSelf(handle)),
            other => Err(HandleError::MethodCalledWithDot {
                type_name: other.type_name(),
            }
            .into()),
        }
    }
}

/// The `__newindex` guard shared by every read-only handle metatable (memory, entry, date, and search
/// result). A handle is a view, so assigning to a field silently did nothing before this — the
/// stale-date footgun. The guard raises a teachable error naming the operation that persists the
/// change instead, tailored for `occurred_at` (the traced slip) since a date lives on an entry, not a
/// handle field. It fires only for keys absent from the raw table, which is every agent-facing field
/// (they are read through `__index` or carried as data the metamethods read), so internal setup that
/// must write a raw field uses `raw_set` to bypass it.
pub(crate) fn readonly_newindex(lua: &Lua, kind: HandleKind) -> mlua::Result<mlua::Function> {
    lua.create_function(move |lua, (_, key, _): (Table, Value, Value)| {
        let field = lua
            .coerce_string(key)?
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let error = if field == "occurred_at" {
            HandleAssignmentError::OccurredAt { kind }
        } else {
            HandleAssignmentError::Other { kind, field }
        };
        Err::<(), mlua::Error>(error.into())
    })
}
