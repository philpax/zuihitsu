//! The `calendar` global: `upcoming`, `overdue`, `on`, `recurring`, and date construction.

use crate::agent::lua::tables::modules::{metatables::*, *};

/// The `calendar` global: `upcoming`, `overdue`, `on`, and `recurring`, each returning a list of memory
/// handles, soonest first. Unlike the brief's `<upcoming/>` block these are the agent's own
/// queries and are not visibility-filtered (like `mem:entries`, the agent sees its whole memory).
/// Strict locking: each returned memory is locked, since the query read (and touched) it.
pub(in crate::agent::lua) fn calendar_table(
    lua: &Lua,
    api: &BlockApi,
    metatable: &Table,
) -> mlua::Result<Table> {
    let calendar = lua.create_table()?;
    calendar.set(
        "upcoming",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, opts: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let within = within_arg(opts)?;
                    let ids = api
                        .block
                        .lock()
                        .upcoming(within.as_deref())
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;
    calendar.set(
        "overdue",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, opts: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let within = within_arg(opts)?;
                    let ids = api
                        .block
                        .lock()
                        .overdue(within.as_deref())
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;
    calendar.set(
        "on",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, date: Value| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    // Accept a date object (`calendar.today()`, `calendar.next(...)`) as readily as a
                    // `"YYYY-MM-DD"` string, so the calendar's own return value feeds its sibling.
                    let date = day_string(&date)?;
                    let ids = api
                        .block
                        .lock()
                        .on(&date)
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;
    calendar.set(
        "recurring",
        lua.create_async_function({
            let api = api.clone();
            let metatable = metatable.clone();
            move |lua, ()| {
                let api = api.clone();
                let metatable = metatable.clone();
                async move {
                    let ids = api
                        .block
                        .lock()
                        .recurring()
                        .map_err(|error| route_error(error, &mut api.infra.lock()))?;
                    api.lock_all(ids.iter().copied()).await;
                    make_handle_list(&lua, ids, &metatable)
                }
            }
        })?,
    )?;

    // Date construction: the agent names a relative date and the runtime computes it, so a date is
    // never arithmetic the model carries in its head. Each returns a date object (see
    // `date_metatable`) that doubles as an `occurred_at` value. Synchronous — they read the clock and
    // do pure date math, touching no memory, so they need no lock.
    let date_metatable = date_metatable(lua)?;
    calendar.set("today", {
        let api = api.clone();
        let dmt = date_metatable.clone();
        lua.create_function(move |lua, ()| {
            let now = api.block.lock().now();
            make_date(lua, time::today(now), &dmt)
        })?
    })?;
    calendar.set("next", {
        let api = api.clone();
        let dmt = date_metatable.clone();
        lua.create_function(move |lua, weekday: String| {
            let now = api.block.lock().now();
            let day = time::next_weekday(now, &weekday)
                .ok_or(CalendarError::NotAWeekday { input: weekday })?;
            make_date(lua, day, &dmt)
        })?
    })?;
    for (name, per) in [("in_days", 1i64), ("in_weeks", 7)] {
        let api = api.clone();
        let dmt = date_metatable.clone();
        calendar.set(
            name,
            lua.create_function(move |lua, count: i64| {
                let now = api.block.lock().now();
                let day = time::add_days(&time::today(now), count.saturating_mul(per)).ok_or(
                    CalendarError::DateOutOfRange {
                        days: count.saturating_mul(per),
                    },
                )?;
                make_date(lua, day, &dmt)
            })?,
        )?;
    }
    calendar.set("date", {
        let dmt = date_metatable.clone();
        lua.create_function(move |lua, day: String| {
            if time::civil_date_to_millis(&day).is_none() {
                return Err(CalendarError::InvalidDate { input: day }.into());
            }
            make_date(lua, day, &dmt)
        })?
    })?;
    Ok(calendar)
}

/// The window argument `calendar.upcoming` and `calendar.overdue` share: a bare duration string
/// ("31 days", "2 weeks") stands for the window directly — the shape the agent naturally writes —
/// while `{ within = "…" }` and `nil` (the default window) keep working. Anything else is a teachable
/// [`CalendarError`] rather than an opaque conversion failure; an unparseable duration string still
/// errors downstream where the duration is parsed, with its own teachable message.
pub(in crate::agent::lua) fn within_arg(opts: Value) -> mlua::Result<Option<String>> {
    match opts {
        Value::Nil => Ok(None),
        Value::String(within) => Ok(Some(within.to_string_lossy())),
        Value::Table(table) => Ok(table.get("within")?),
        other => Err(CalendarError::NotAWindow {
            type_name: other.type_name(),
        }
        .into()),
    }
}
