//! Temporal helpers: date handles, day coercion, and `occurred_at` normalization for `append` options.

use mlua::{Lua, LuaSerdeExt, Table, Value};

use crate::{
    event::Teller,
    memory::memory_block::AppendOptions,
    time::{CivilDate, MILLIS_PER_DAY, TemporalRef, civil_date_to_millis},
};

use crate::agent::lua::{
    error::{HandleError, TemporalArgError},
    runtime::{
        BlockApi,
        handles::{handle_id, resolve_exclude},
        route_error,
    },
};

/// Build a date handle `{ day = "YYYY-MM-DD" }` backed by the date metatable, so it renders as its ISO
/// day, carries calendar arithmetic (`:add_days` …), and — being a `{ day = … }` table — deserializes
/// straight into a `Day` occurrence when handed to `append` as `occurred_at`. So the agent computes a
/// date through operations the runtime executes, never as a string it works out in its head.
pub(crate) fn make_date(lua: &Lua, iso: String, date_metatable: &Table) -> mlua::Result<Table> {
    let handle = lua.create_table()?;
    handle.set("day", iso)?;
    handle.set_metatable(Some(date_metatable.clone()))?;
    Ok(handle)
}

/// The ISO `YYYY-MM-DD` day a temporal argument names: a date object (a `{ day = "…" }` table) yields
/// its `day`, a string is taken verbatim, and anything else is a teachable [`TemporalArgError`]. Shared
/// by `calendar.on` and the `occurred_at` normalization so a date object stands in for a date string
/// wherever a single day is wanted, without validating the string itself (the caller's block does that).
pub(crate) fn day_string(value: &Value) -> mlua::Result<String> {
    match value {
        Value::String(day) => Ok(day.to_string_lossy()),
        Value::Table(table) => match table.get::<Option<String>>("day")? {
            Some(day) => Ok(day),
            None => Err(TemporalArgError::NotADate { type_name: "table" }.into()),
        },
        other => Err(TemporalArgError::NotADate {
            type_name: other.type_name(),
        }
        .into()),
    }
}

/// Deserialize a Lua `opts` table into [`AppendOptions`], first normalizing any date handles inside its
/// `occurred_at` so a date object stands in for a `"YYYY-MM-DD"` string wherever a day is wanted. This
/// is the single seam every `occurred_at` taker — `<memory>:append`, `<memory>:revise`, and
/// `memory.create` — passes through, so accepting a date handle is decided once here rather than at each
/// call site; a future taker of the tagged table inherits it for free. `nil` opts yield `None`.
pub(crate) fn append_options_from_lua(
    api: &BlockApi,
    lua: &Lua,
    opts: Value,
) -> mlua::Result<Option<AppendOptions>> {
    if opts.is_nil() {
        return Ok(None);
    }
    let Value::Table(table) = &opts else {
        // A non-nil, non-table opts is a shape slip the agent should see named; serde surfaces it.
        return Ok(Some(lua.from_value(opts)?));
    };
    // Resolve the three options serde cannot decode from a raw Lua value: `occurred_at` may be a bare
    // date string or a date handle, `told_by` is a handle or name naming a teller, and `exclude` is a
    // list of handles or names to withhold the entry from. The rest deserializes from a copy with those
    // three keys dropped — the agent's own opts table is never mutated, so a table reused across appends
    // keeps its fields.
    let occurred_at = occurred_at_from_lua(lua, table)?;
    let told_by = match table.get::<Value>("told_by")? {
        Value::Nil => None,
        other => Some(resolve_teller(api, other)?),
    };
    let exclude = resolve_exclude(api, table.get::<Value>("exclude")?)?;
    let rest = lua.create_table()?;
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        if let Value::String(name) = &key
            && matches!(
                name.to_string_lossy().as_str(),
                "occurred_at" | "told_by" | "exclude"
            )
        {
            continue;
        }
        rest.set(key, value)?;
    }
    let mut options: AppendOptions = lua.from_value(Value::Table(rest))?;
    options.occurred_at = occurred_at;
    options.told_by = told_by;
    options.exclude = exclude;
    Ok(Some(options))
}

/// Resolve an `occurred_at` option to its [`TemporalRef`]: a bare `"YYYY-MM-DD"` string coerces to the
/// day shape, so the intuitive `occurred_at = "2026-06-15"` lands the same `Day` as
/// `{ day = "2026-06-15" }`; a tagged table is normalized (a date handle or string in a day/range/
/// instant position stands in for its primitive, see [`normalize_temporal`]) and then decoded. A value
/// that names no occurrence — a non-date string, a number, or a table with no recognized tag — is a
/// teachable [`TemporalArgError::UnknownOccurrence`] naming the accepted shapes, never serde's raw
/// enum-variant list. `nil` yields `None`.
fn occurred_at_from_lua(lua: &Lua, table: &Table) -> mlua::Result<Option<TemporalRef>> {
    match table.get::<Value>("occurred_at")? {
        Value::Nil => Ok(None),
        Value::String(day) => {
            let day = day.to_string_lossy();
            if civil_date_to_millis(&day).is_some() {
                Ok(Some(TemporalRef::Day(CivilDate(day.into()))))
            } else {
                Err(TemporalArgError::UnknownOccurrence {
                    got: format!("the string {day:?}"),
                }
                .into())
            }
        }
        Value::Table(occurred) => {
            normalize_temporal(&occurred)?;
            lua.from_value::<TemporalRef>(Value::Table(occurred))
                .map(Some)
                .map_err(|_| {
                    TemporalArgError::UnknownOccurrence {
                        got: "a table with no recognized tag".to_owned(),
                    }
                    .into()
                })
        }
        other => Err(TemporalArgError::UnknownOccurrence {
            got: format!("a {}", other.type_name()),
        }
        .into()),
    }
}

/// Resolve an append's `told_by` option to the participant [`Teller`] it attributes the entry to.
/// Dual-accepts a person handle (from `memory.get`/`memory.create`) or a name string, mirroring
/// `links.create`'s target resolution, so a relayed claim recorded while someone else speaks is stamped with
/// its real source — "X said …" told by X, not the current speaker. An unknown name is a teachable
/// error; so is a value that is neither a handle nor a name.
pub(crate) fn resolve_teller(api: &BlockApi, value: Value) -> mlua::Result<Teller> {
    let id = match value {
        Value::Table(handle) => handle_id(&handle)?,
        Value::String(name) => {
            let name = name.to_string_lossy();
            match api
                .block
                .lock()
                .get(&name)
                .map_err(|error| route_error(error, &mut api.infra.lock()))?
            {
                Some((id, _)) => id,
                None => {
                    return Err(HandleError::UnknownTeller {
                        name: name.to_string(),
                    }
                    .into());
                }
            }
        }
        other => {
            return Err(HandleError::WrongTellerType {
                type_name: other.type_name(),
            }
            .into());
        }
    };
    Ok(Teller::Participant(id))
}

/// Rewrite date-shaped values inside an `occurred_at` tagged table into the primitives its
/// [`TemporalRef`] deserialization expects, in place, so a day named as a date object *or* a bare
/// `"YYYY-MM-DD"` string stands in wherever a millisecond timestamp is wanted:
/// - a `{ day = <date object> }` field becomes `{ day = "…" }`;
/// - a range's `start`/`end` — a date object or a date string — becomes the day's bounding instant (its
///   first millisecond for `start`, its last for `end`, so a range from Monday to Friday spans all of
///   Friday);
/// - an `instant` given as a date object or date string becomes the day's first millisecond.
///
/// A position already holding a primitive is left untouched, so a millisecond count passes through. The
/// value itself being a date handle needs no rewrite — a date handle *is* a `{ day = "…" }` table, so it
/// already deserializes as a `Day`.
fn normalize_temporal(occurred: &Table) -> mlua::Result<()> {
    if let day @ Value::Table(_) = occurred.get::<Value>("day")? {
        occurred.set("day", day_string(&day)?)?;
    }
    if let Value::Table(range) = occurred.get::<Value>("range")? {
        coerce_range_bound(&range, "start", DayBound::Start)?;
        coerce_range_bound(&range, "end", DayBound::End)?;
    }
    let instant = occurred.get::<Value>("instant")?;
    if matches!(instant, Value::Table(_) | Value::String(_)) {
        occurred.set("instant", day_bound_millis(&instant, DayBound::Start)?)?;
    }
    Ok(())
}

/// Which instant of a day a millisecond-typed position resolves to when a date stands in for it: the
/// day's first millisecond for a `Start`, its last for an `End`, so a range covers the whole of both
/// boundary days and a bare instant lands at the start of its day.
enum DayBound {
    Start,
    End,
}

/// Replace a range endpoint given as a date object or a `"YYYY-MM-DD"` string with the day's bounding
/// instant in epoch milliseconds; a primitive already there (a millisecond count) is left untouched.
fn coerce_range_bound(range: &Table, key: &str, bound: DayBound) -> mlua::Result<()> {
    let endpoint = range.get::<Value>(key)?;
    if matches!(endpoint, Value::Table(_) | Value::String(_)) {
        range.set(key, day_bound_millis(&endpoint, bound)?)?;
    }
    Ok(())
}

/// Resolve a date object or `"YYYY-MM-DD"` string to one of its day's bounding instants in epoch
/// milliseconds — the shared coercion behind a range endpoint and a bare `instant`. An unparseable day
/// is a teachable [`TemporalArgError::InvalidDay`].
fn day_bound_millis(value: &Value, bound: DayBound) -> mlua::Result<i64> {
    let day = day_string(value)?;
    let midnight = civil_date_to_millis(&day).ok_or(TemporalArgError::InvalidDay { input: day })?;
    Ok(match bound {
        DayBound::Start => midnight,
        DayBound::End => midnight + MILLIS_PER_DAY - 1,
    })
}
