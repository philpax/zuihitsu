//! Value rendering: the `inspect` global, `table.concat` wrapper, and the render pipeline that turns
//! a block's final value into agent-visible text.

use mlua::{Lua, Table, Value};

use super::super::error::ConcatError;

/// Render a script's final value to the text the agent sees back (REPL-style).
/// Fold a block's `print` output and its final-value rendering into the one agent-visible result.
/// When nothing was printed this is just the value (the common `return …` case, unchanged). When the
/// block printed but returned nothing meaningful (a `for … print(x) end` loop, whose value is `nil`),
/// the printed output stands alone rather than being buried under a bare `nil`.
pub(crate) fn combine_output(printed: String, value: String) -> String {
    let printed = printed.trim_end_matches('\n');
    if printed.is_empty() {
        value
    } else if value.is_empty() || value == "nil" {
        printed.to_owned()
    } else {
        format!("{printed}\n{value}")
    }
}

pub(crate) fn render(lua: &Lua, value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_owned(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string_lossy(),
        // A table with a `__tostring` metamethod (an entry handle) renders through it, so a returned
        // entry — or a list of them — reads as its text rather than `<table>`. `coerce_string` would
        // not do this (it ignores `__tostring`), so call the `tostring` builtin, which honors it.
        Value::Table(t) => match tostring_via_metamethod(lua, value, t) {
            Some(text) => text,
            None => render_table(lua, value, t),
        },
        other => format!("<{}>", other.type_name()),
    }
}

/// Render a table through its `__tostring` metamethod, if it has one — the entry-handle case. `None`
/// for a plain table (no metamethod), so the caller falls back to the array rendering.
pub(crate) fn tostring_via_metamethod(lua: &Lua, value: &Value, table: &Table) -> Option<String> {
    let has_tostring = table
        .metatable()
        .is_some_and(|mt| mt.contains_key("__tostring").unwrap_or(false));
    if !has_tostring {
        return None;
    }
    lua.globals()
        .get::<mlua::Function>("tostring")
        .and_then(|tostring| tostring.call::<String>(value.clone()))
        .ok()
}

/// Render an undecorated table (no `__tostring`): a sequence as its elements joined by newlines (a
/// list of entry handles or search results), otherwise its structure via [`inspect_table`] — so a map
/// table the agent built, or one we have not given a `__tostring`, reads back as its fields rather than
/// an opaque `<table>` the model cannot act on.
fn render_table(lua: &Lua, value: &Value, table: &Table) -> String {
    let items: Vec<String> = table
        .clone()
        .sequence_values::<Value>()
        .filter_map(Result::ok)
        .map(|value| render(lua, &value))
        .collect();
    if items.is_empty() {
        inspect_table(lua, value)
    } else {
        items.join("\n")
    }
}

/// Pretty-print a table's structure through the vendored `inspect` global (loaded by
/// [`install_inspect`]). This is the fallback for a table with neither a `__tostring` nor a sequence
/// part; `inspect` only ever sees plain tables here, so its default options render clean
/// `{ key = value }` structure with no metatable noise. Falls back to the bare token if the global is
/// somehow absent.
fn inspect_table(lua: &Lua, value: &Value) -> String {
    lua.globals()
        .get::<mlua::Function>("inspect")
        .and_then(|inspect| inspect.call::<String>(value.clone()))
        .unwrap_or_else(|_| "<table>".to_owned())
}

/// The vendored `inspect.lua` pretty-printer (MIT-licensed, kikito/inspect.lua; see
/// `vendor/inspect.lua/VENDOR.md` for the exact commit). Loaded once per VM and exposed as the
/// `inspect` global, it backs [`render`]'s fallback for an undecorated table so the agent never
/// receives an opaque `<table>` it cannot read.
const INSPECT_LUA: &str = include_str!("../../../../vendor/inspect.lua/inspect.lua");

/// Evaluate `inspect.lua` and bind a wrapped pretty-printer as the `inspect` global. The wrapper
/// passes a `process` hook that keeps the structural dump legible where it matters most: a nested
/// table carrying a `__tostring` — a memory handle, an entry, a search hit — renders as its own text
/// rather than as a bare id-and-metatable blob (a handle's name and description read lazily through
/// `__index`, so the raw structure shows neither), and metatable entries are omitted outright, since
/// `<metatable> = { __concat = <function 1>, … }` is noise the agent cannot act on. The root value is
/// left to the hook's callers ([`render`] tries `__tostring` first, so a decorated root never reaches
/// the inspector). Done once at VM construction, like the MCP projection.
pub(crate) fn install_inspect(lua: &Lua) -> mlua::Result<()> {
    let module: Table = lua.load(INSPECT_LUA).set_name("inspect.lua").eval()?;
    let wrapper: mlua::Function = lua
        .load(
            r#"
            local inspect = ...
            return function(value)
                return inspect(value, {
                    process = function(item, path)
                        if path[#path] == inspect.METATABLE then
                            return nil
                        end
                        if type(item) == "table" and #path > 0 then
                            local mt = getmetatable(item)
                            if mt and mt.__tostring then
                                return tostring(item)
                            end
                        end
                        return item
                    end,
                })
            end
            "#,
        )
        .set_name("inspect-wrapper")
        .call(&module)?;
    lua.globals().set("inspect", wrapper)?;
    Ok(())
}

/// Wrap stock `table.concat` so a reader's handle list fails *legibly*. Stock Luau `table.concat` joins
/// only strings and numbers, so a handle list — `mem:entries()`, `hub:links()` — fails it with the
/// opaque "invalid value (table) at index … in table for 'concat'", one of the recurring recall
/// confusions. This shell keeps stock semantics exactly — it delegates the join to the original
/// function untouched, so an ordinary `table.concat(names, ",")` over a list the agent built joins as
/// before — and only rewrites the error, redirecting the two observed slips to teachable messages (see
/// [`ConcatError`]): the whole list argument being a reader *method* rather than its result, and a
/// handle list, which now points at string interpolation (a backtick string stringifies a handle) as
/// the way to compose text from entries, links, and dates. Installed before `Lua::sandbox(true)` freezes
/// the `table` library read-only, so the override is part of the frozen surface.
pub(crate) fn install_table_concat(lua: &Lua) -> mlua::Result<()> {
    let table_lib: Table = lua.globals().get("table")?;
    let stock: mlua::Function = table_lib.get("concat")?;
    table_lib.set(
        "concat",
        lua.create_function(move |_, args: mlua::Variadic<Value>| {
            let list_type = args.first().map(Value::type_name).unwrap_or("nil");
            match stock.call::<Value>(args) {
                Ok(joined) => Ok(joined),
                // A table that stock concat rejected holds a non-joinable element (a handle list);
                // any other first argument is not a list at all (a reader method, most often).
                Err(_) if list_type == "table" => Err(ConcatError::NonJoinable.into()),
                Err(_) => Err(ConcatError::NotAList {
                    type_name: list_type,
                }
                .into()),
            }
        })?,
    )?;
    Ok(())
}

/// Render a value to its text for entry-handle `__concat`: an entry handle yields its `text`; any
/// other value coerces as Lua's `tostring` would (strings and numbers directly, otherwise empty).
pub(crate) fn value_text(lua: &Lua, value: &Value) -> mlua::Result<String> {
    if let Value::Table(table) = value
        && let Ok(text) = table.get::<String>("text")
    {
        return Ok(text);
    }
    Ok(lua
        .coerce_string(value.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

/// The `__concat` metamethod shared by the read-surface handles (a memory, a link result, a search
/// result): each operand renders through its own `__tostring`, so `"Topic: " .. topic` and
/// `"- " .. link` compose the same text printing already shows, rather than erroring as a bare
/// table — the join the agent actually writes when assembling a reply. A plain string or number
/// operand coerces as Lua's `tostring` would.
pub(crate) fn concat_via_tostring(lua: &Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(|lua, (left, right): (Value, Value)| {
        Ok(format!(
            "{}{}",
            tostring_text(lua, &left)?,
            tostring_text(lua, &right)?
        ))
    })
}

/// One `__concat` operand's text: a table with a `__tostring` renders through it; everything else
/// coerces as Lua's `tostring` would (strings and numbers directly, otherwise empty).
fn tostring_text(lua: &Lua, value: &Value) -> mlua::Result<String> {
    if let Value::Table(table) = value
        && let Some(text) = tostring_via_metamethod(lua, value, table)
    {
        return Ok(text);
    }
    Ok(lua
        .coerce_string(value.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}

/// Render a value for a date handle's `__concat`: a date handle (a `{ day = "…" }` table) yields its
/// ISO day, and any other operand coerces as Lua's `tostring` would — so both `"on " .. friday` and
/// `friday .. " it is"` read the date while the surrounding text coerces normally.
pub(crate) fn date_text(lua: &Lua, value: &Value) -> mlua::Result<String> {
    if let Value::Table(table) = value
        && let Ok(day) = table.get::<String>("day")
    {
        return Ok(day);
    }
    Ok(lua
        .coerce_string(value.clone())?
        .map(|s| s.to_string_lossy())
        .unwrap_or_default())
}
