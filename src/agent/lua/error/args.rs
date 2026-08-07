//! Argument-validation errors for the Lua interface: the wrongly-shaped-argument reword, the
//! `calendar.*` constructor and date-arithmetic failures, and the temporal argument parsing errors.
//! Each names the offending value and the accepted shape, and converts to a runtime error via its
//! `Display`.

use mlua::Error as LuaError;

use crate::agent::{body_of, render_placeholders};

/// A wrongly-shaped argument to a Lua API function — a table where a string was wanted, or the
/// reverse — caught at the argument boundary and reworded from mlua's raw "error converting Lua table
/// to String" (which names neither the function nor the fix) into a teachable message. It names the
/// function, what the position expected, what arrived, and the correct one-line call, so a shape slip
/// teaches the signature at its point of failure rather than leaving the agent to guess. Raised by the
/// [`arg`](crate::agent::lua::runtime::arg) helper, which delegates to the real `FromLua` conversion
/// and only rewords its failure, so Luau's own string/number coercion is preserved.
#[derive(Debug)]
pub(in crate::agent::lua) struct ArgError {
    /// The function's agent-facing name, e.g. `"memory.search"` or `"mem:append"`.
    pub function: &'static str,
    /// What the argument position expects, in the agent's words, e.g. `"a query string"`.
    pub expected: &'static str,
    /// The Luau type that arrived instead, e.g. `"table"`.
    pub got: &'static str,
    /// The correct call, shown so the agent reissues it directly, e.g.
    /// `"pass the search text directly, memory.search(\"dave\")"`.
    pub hint: &'static str,
}

impl std::fmt::Display for ArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ArgError {
            function,
            expected,
            got,
            hint,
        } = self;
        // Read "got a table" for a value in the wrong shape, but "got nil" for an omitted argument, so
        // the wording stays natural whichever way the call was malformed.
        let arrived = if *got == "nil" {
            "nil".to_owned()
        } else {
            format!("a {got}")
        };
        f.write_str(&render_placeholders(
            body_of(include_str!("prose/args/arg_error.md")),
            &[
                ("function", function),
                ("expected", expected),
                ("arrived", &arrived),
                ("hint", hint),
            ],
        ))
    }
}

impl std::error::Error for ArgError {}

impl From<ArgError> for LuaError {
    fn from(error: ArgError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A bad argument to a `calendar.*` constructor or a date-arithmetic method.
#[derive(Debug)]
pub(in crate::agent::lua) enum CalendarError {
    /// `calendar.next` was given a string that is not a full weekday name.
    NotAWeekday { input: String },
    /// `calendar.in_days`/`in_weeks` shifted past the representable date range.
    DateOutOfRange { days: i64 },
    /// `calendar.date` was given a string that is not `YYYY-MM-DD`.
    InvalidDate { input: String },
    /// A date object's `day` field could not be interpreted — only reachable if one was corrupted,
    /// since the constructors validate before minting a date.
    InvalidDay { input: String },
    /// `calendar.upcoming`/`overdue` was given a window that is neither a duration string, an opts
    /// table, nor nil.
    NotAWindow { type_name: &'static str },
}

impl std::fmt::Display for CalendarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalendarError::NotAWeekday { input } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/not_a_weekday.md")),
                &[("input", &format!("{input:?}"))],
            )),
            CalendarError::DateOutOfRange { days } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/date_out_of_range.md")),
                &[("days", &days.to_string())],
            )),
            CalendarError::InvalidDate { input } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/invalid_date.md")),
                &[("input", &format!("{input:?}"))],
            )),
            CalendarError::InvalidDay { input } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/invalid_day.md")),
                &[("input", &format!("{input:?}"))],
            )),
            CalendarError::NotAWindow { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/not_a_window.md")),
                &[("type_name", type_name)],
            )),
        }
    }
}

impl std::error::Error for CalendarError {}

impl From<CalendarError> for LuaError {
    fn from(error: CalendarError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A bad date value handed to a temporal surface — `calendar.on`, or the `occurred_at` option's `day`
/// and range positions — where a date object (from `calendar.today()` and its siblings) or a
/// `"YYYY-MM-DD"` string was wanted. Raised at the parsing seam that every `occurred_at` taker passes
/// through, so a date object stands in for a date string uniformly.
#[derive(Debug)]
pub(in crate::agent::lua) enum TemporalArgError {
    /// A value that is neither a date object nor a date string where a day was expected.
    NotADate { type_name: &'static str },
    /// A date string (or a date object's `day`) that is not a valid `YYYY-MM-DD` calendar date.
    InvalidDay { input: String },
    /// An `occurred_at` option that names no occurrence at all — neither a bare `"YYYY-MM-DD"` string,
    /// a date object, nor a recognized tagged table. `got` describes the offending value. Names the
    /// accepted shapes so the agent reissues with one, rather than reading serde's raw enum-variant
    /// list (`unknown variant, expected instant/day/range/…`).
    UnknownOccurrence { got: String },
}

impl std::fmt::Display for TemporalArgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TemporalArgError::NotADate { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/not_a_date.md")),
                &[("type_name", type_name)],
            )),
            TemporalArgError::InvalidDay { input } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/invalid_date.md")),
                &[("input", &format!("{input:?}"))],
            )),
            TemporalArgError::UnknownOccurrence { got } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/args/unknown_occurrence.md")),
                &[("got", got)],
            )),
        }
    }
}

impl std::error::Error for TemporalArgError {}

impl From<TemporalArgError> for LuaError {
    fn from(error: TemporalArgError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}
