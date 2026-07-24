//! Argument-validation errors for the Lua interface: the wrongly-shaped-argument reword, the
//! `calendar.*` constructor and date-arithmetic failures, and the temporal argument parsing errors.
//! Each names the offending value and the accepted shape, and converts to a runtime error via its
//! `Display`.

use mlua::Error as LuaError;

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
        write!(f, "{function}: expected {expected}, got {arrived} — {hint}")
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
            CalendarError::NotAWeekday { input } => write!(
                f,
                "{input:?} is not a weekday; use a full name like \"monday\" or \"friday\""
            ),
            CalendarError::DateOutOfRange { days } => write!(
                f,
                "the date {days} days from today is out of range; use a smaller offset"
            ),
            CalendarError::InvalidDate { input } => write!(
                f,
                "{input:?} is not a valid date; use YYYY-MM-DD, e.g. \"2026-06-03\""
            ),
            CalendarError::InvalidDay { input } => write!(f, "{input:?} is not a valid day"),
            CalendarError::NotAWindow { type_name } => write!(
                f,
                "the window is a duration — pass it directly (\"31 days\", \"2 weeks\") or as \
                 {{ within = \"…\" }}, or omit it for the default; got {type_name}"
            ),
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
            TemporalArgError::NotADate { type_name } => write!(
                f,
                "expected a date object (from calendar.today(), calendar.next(...), …) or a \
                 \"YYYY-MM-DD\" string, got {type_name}"
            ),
            TemporalArgError::InvalidDay { input } => write!(
                f,
                "{input:?} is not a valid date; use YYYY-MM-DD, e.g. \"2026-06-03\""
            ),
            TemporalArgError::UnknownOccurrence { got } => write!(
                f,
                "occurred_at does not name an occurrence ({got}). Pass a bare \"YYYY-MM-DD\" string \
                 or a date object (calendar.today(), calendar.date(\"…\")) for a single day, or a \
                 tagged table for a richer occurrence: {{ day = \"YYYY-MM-DD\" }}, \
                 {{ instant = <epoch ms> }}, {{ range = {{ start = …, [\"end\"] = … }} }}, \
                 {{ approx = {{ center = …, fuzz_days = N }} }}, {{ recurring = \"FREQ=WEEKLY\" }}, \
                 or {{ before_after = {{ dir = \"before\" | \"after\", anchor = \"<memory name>\" }} }}"
            ),
        }
    }
}

impl std::error::Error for TemporalArgError {}

impl From<TemporalArgError> for LuaError {
    fn from(error: TemporalArgError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}
