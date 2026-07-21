//! `arg`: the argument-shape helper for the Lua API seams. It converts one raw `Value` to the typed
//! argument a function wants and, where mlua's `FromLua` would surface its opaque "error converting
//! Lua table to String", rewords the failure into a teachable [`ArgError`] naming the function, the
//! expected shape, what arrived, and the correct one-line call. The real conversion is delegated
//! untouched, so Luau's own string/number coercion still applies — only its error message is replaced.

use mlua::{FromLua, Lua, Value};

use crate::agent::lua::error::ArgError;

/// Convert `value` to `T`, rewording a shape mismatch into a teachable [`ArgError`]. `function` names
/// the call (`"memory.search"`), `expected` says what the position wants (`"a query string"`), and
/// `hint` shows the correct call (`"pass the search text directly, memory.search(\"dave\")"`). The
/// underlying `FromLua` runs untouched, so a number handed where a string is wanted still coerces the
/// way Luau does; the reworded error only fires when the real conversion fails.
pub(crate) fn arg<T: FromLua>(
    lua: &Lua,
    value: Value,
    function: &'static str,
    expected: &'static str,
    hint: &'static str,
) -> mlua::Result<T> {
    let got = value.type_name();
    T::from_lua(value, lua).map_err(|_| {
        ArgError {
            function,
            expected,
            got,
            hint,
        }
        .into()
    })
}
