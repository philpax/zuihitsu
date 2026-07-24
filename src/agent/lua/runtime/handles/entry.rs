//! Entry handles: minting the addressable `{ id, text, … }` handle a `mem:entries`/`history` read
//! returns, the agent-facing visibility label an entry carries, the sequence shape a list of entries
//! wraps to, and the entry-selector resolution `mem:supersede`/`retract` accept.

use mlua::{Lua, LuaSerdeExt, Table, Value};

use crate::{
    event::Visibility,
    ids::EntryId,
    memory::memory_block::{EntryRef, EntrySelector},
};

use crate::agent::lua::error::HandleError;
use ulid::Ulid;

/// Build an entry handle `{ id = "<ulid>", text = "..." }` backed by the entry metatable, so it
/// renders as its text (`__tostring` / `__concat`) yet stays addressable for `mem:supersede`.
pub(crate) fn make_entry_handle(
    lua: &Lua,
    entry: &EntryRef,
    entry_metatable: &Table,
) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("id", entry.entry_id.0.to_string())?;
    handle.set("text", entry.text.as_str())?;
    // Carried so a read renders self-describingly (see the entry metatable's `__tostring`) and so a
    // script can branch on them: `entry.visibility` ("public"/"private"), `entry.told_by` (the teller),
    // `entry.disputed` (true when the fact is under an unresolved arbitration), and `entry.occurred_at`
    // (the occurrence as the *same* tagged table `append` accepts — `{ day = "…" }` etc. — so a read
    // round-trips to a write and a script can match on `entry.occurred_at.day`, not a string it has to
    // reparse; the metatable's `__tostring` renders it for display).
    handle.set("visibility", visibility_label(&entry.visibility))?;
    handle.set("told_by", entry.teller.as_str())?;
    // The tellers standing behind the fact (its visible attestation subset, the agent skipped) — so a
    // multiply-attested fact reads `from person/erin, person/dave` and a script can branch on
    // `entry.attested_by`. Empty for an agent-only entry, where the metatable falls back to `told_by`.
    if !entry.attesters.is_empty() {
        handle.set(
            "attested_by",
            lua.create_sequence_from(entry.attesters.iter().map(String::as_str))?,
        )?;
    }
    handle.set("disputed", entry.disputed)?;
    // When set, `text` is already the withheld stub (the content never leaves the block); the flag
    // lets a script branch and lets the metatable render it as a withheld confidence, not bare text.
    handle.set("withheld", entry.withheld)?;
    // True when the fact has aged past usefulness on a high-volatility memory; the metatable renders a
    // `stale` segment so the agent hedges rather than asserting it as current.
    handle.set("stale", entry.stale)?;
    if let Some(occurred_at) = &entry.occurred_at {
        handle.set("occurred_at", lua.to_value(occurred_at)?)?;
    }
    // A retracted entry surfaces only through `mem:history`; its reason is carried so the metatable
    // renders it as a tombstone (`[retracted: …]`) and a script can branch on `entry.retracted_reason`.
    if let Some(reason) = &entry.retracted_reason {
        handle.set("retracted_reason", reason.as_str())?;
    }
    handle.set_metatable(Some(entry_metatable.clone()))?;
    Ok(handle)
}

/// The agent-facing label for an entry's visibility — `public` for freely surfaceable, `attributed`
/// for an ordinary secondhand fact the agent should weigh as relayed, and `private` for a confidence
/// (`PrivateToTeller`/`Exclude`) that only resurfaces to its teller.
pub(crate) fn visibility_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "public",
        Visibility::Attributed => "attributed",
        Visibility::PrivateToTeller | Visibility::Exclude(_) => "private",
    }
}

/// Wrap a list of entry refs as a Lua sequence of entry handles, in order — the `mem:entries()` /
/// `mem:history()` return shape.
pub(crate) fn make_entry_handle_list(
    lua: &Lua,
    entries: Vec<EntryRef>,
    entry_metatable: &Table,
) -> mlua::Result<Value> {
    let list = lua.create_table()?;
    for (index, entry) in entries.into_iter().enumerate() {
        list.set(index + 1, make_entry_handle(lua, &entry, entry_metatable)?)?;
    }
    Ok(Value::Table(list))
}

pub(crate) fn entry_handle_id(handle: &Table) -> mlua::Result<EntryId> {
    let id: String = handle.get("id")?;
    Ulid::from_string(&id)
        .map(EntryId)
        .map_err(|source| HandleError::InvalidEntryHandle { id, source }.into())
}

/// Resolve an entry argument to an [`EntrySelector`] the block resolves against the memory's class.
/// The argument is either an entry handle (a `{ id = … }` table read from
/// `mem:entries`/`mem:history`/`mem:append`), whose full id addresses the entry directly, or a bare
/// string — a full entry id, or a unique prefix of one read off a rendered entry line. These are the
/// forms `mem:supersede` and `mem:retract` accept, so a script can address an entry it holds a handle
/// to or one it names by (part of) its id, without re-scanning the text.
pub(crate) fn entry_selector(value: &Value) -> mlua::Result<EntrySelector> {
    match value {
        Value::Table(handle) => Ok(EntrySelector::Id(entry_handle_id(handle)?)),
        Value::String(reference) => Ok(EntrySelector::Ref(reference.to_str()?.to_owned())),
        other => Err(HandleError::WrongEntryType {
            type_name: other.type_name(),
        }
        .into()),
    }
}
