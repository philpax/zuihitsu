//! The typed errors the Lua interface raises — calendar arguments, memory-search failures, handle
//! resolution, and block-consistency invariants. Each carries the offending value and the constraint,
//! and converts to `mlua::Error::RuntimeError` via its `Display`, so the agent-facing wording lives in
//! one place alongside the structured context (CONTRIBUTING: structured error types). The
//! agent-facing teachable messages are deliberately unprefixed prose — the agent reads them, not an
//! operator — while the delegating variants (search, embed) nest their inner error's own prefix.

use mlua::Error as LuaError;
use ulid::DecodeError as UlidError;

use crate::{
    ids::{EntryId, MemoryName, NamespacedMemoryName},
    memory::search::SearchError,
    model::ModelError,
    store::StoreError,
};

/// A bad argument to a `calendar.*` constructor or a date-arithmetic method.
#[derive(Debug)]
pub(super) enum CalendarError {
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
pub(super) enum TemporalArgError {
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

/// A failure running `memory.search` — the embedder/vector backends, or the absence of retrieval on
/// the instance. The delegating variants nest their inner error's own `model:`/`search (…):` prefix.
#[derive(Debug)]
pub(super) enum MemorySearchError {
    /// No embedding endpoint is configured, so semantic search is unavailable.
    NoRetrieval,
    /// The query was empty or whitespace — semantic search needs something to match on. Caught
    /// before the embedder is called, so a degenerate "enumerate a namespace" query fails fast and
    /// teachably instead of embedding the empty string and grinding through every memory.
    EmptyQuery,
    Embed(ModelError),
    NoVector,
    Settings(StoreError),
    Search(SearchError),
}

impl std::fmt::Display for MemorySearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemorySearchError::EmptyQuery => write!(
                f,
                "memory.search needs a query to match on — an empty search cannot list a namespace. \
                 Search for what you are actually after in natural language, narrowing with the \
                 namespace option if you want to stay within one prefix, e.g. \
                 memory.search(\"deploy schedule\", {{ namespace = \"topic/\" }})."
            ),
            MemorySearchError::NoRetrieval => write!(
                f,
                "semantic search is unavailable on this instance (no embedding endpoint configured)"
            ),
            MemorySearchError::Embed(error) => {
                write!(f, "embedding the search query failed: {error}")
            }
            MemorySearchError::NoVector => write!(f, "the embedder returned no vector"),
            MemorySearchError::Settings(error) => {
                write!(f, "could not read the search settings: {error}")
            }
            // `SearchError` already carries a `search (…):` layer prefix.
            MemorySearchError::Search(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MemorySearchError {}

impl From<MemorySearchError> for LuaError {
    fn from(error: MemorySearchError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A failure resolving a `convo.turn` transcript link. `InvalidTurnId`, `AudienceMismatch`, and
/// `NotFound` are teachable (the agent reads them and adapts); `Store` is an infrastructure read
/// failure surfaced to terminate the block cleanly, as `memory.search` does — turn resolution is
/// read-only, so a failure corrupts nothing. The two refusals are deliberately distinct:
/// `AudienceMismatch` says the id names a real moment the present audience did not all share (safe to
/// confirm, ULIDs being unguessable) and points at memory as the visibility-filtered channel;
/// `NotFound` says no such moment exists.
#[derive(Debug)]
pub(super) enum TurnResolveError {
    /// `convo.turn` was given a string that is not a ULID.
    InvalidTurnId { id: String, source: UlidError },
    /// The id names a real turn, but someone present here was not in that moment's audience.
    AudienceMismatch { id: String },
    /// No turn with this id exists anywhere in the log.
    NotFound { id: String },
    /// The event store could not be read.
    Store(StoreError),
}

impl std::fmt::Display for TurnResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnResolveError::InvalidTurnId { id, source } => {
                write!(f, "invalid turn id {id:?}: {source}")
            }
            TurnResolveError::AudienceMismatch { id } => write!(
                f,
                "turn {id:?} is a real moment, but its audience did not include everyone present \
                 here — so it cannot be replayed to this room. If its substance is worth relaying, \
                 recall it through memory — reconstruct it from every memory the moment plausibly \
                 touched, not the first hit — and honor each entry's own audience as you speak — \
                 the visibility rule governs what you say, not just what you look up."
            ),
            TurnResolveError::NotFound { id } => write!(
                f,
                "no turn {id:?} exists — the id must be one you were given (the value inside a \
                 [turn:<id>] token)"
            ),
            TurnResolveError::Store(error) => {
                write!(f, "could not read the conversation transcript: {error}")
            }
        }
    }
}

impl std::error::Error for TurnResolveError {}

impl From<TurnResolveError> for LuaError {
    fn from(error: TurnResolveError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A bad argument to `memory.list`. The prefix is required and must be non-empty: an empty or
/// whitespace stem would list the whole graph, which is not what discovery-by-stem is for, so it is a
/// teachable error naming the shape and pointing at `memory.search` for recall-by-meaning.
#[derive(Debug)]
pub(super) enum ListError {
    /// `memory.list` was called with no prefix, or a blank one.
    EmptyPrefix,
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListError::EmptyPrefix => write!(
                f,
                "list finds handles by stem — pass a name prefix like \"person/\" or \"person/dav\"; \
                 to recall by meaning, memory.search"
            ),
        }
    }
}

impl std::error::Error for ListError {}

impl From<ListError> for LuaError {
    fn from(error: ListError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A bad handle or link target passed to a memory operation.
#[derive(Debug)]
pub(super) enum HandleError {
    /// A memory handle's `id` is not a ULID.
    InvalidMemoryHandle { id: String, source: UlidError },
    /// An entry handle's `id` is not a ULID.
    InvalidEntryHandle { id: String, source: UlidError },
    /// `mem:retract` was given a value that is neither an entry handle nor an entry-id string.
    WrongEntryType { type_name: &'static str },
    /// `links.create`/`links.remove` was given a name string — in the subject or the object
    /// position — that is not a known memory.
    UnknownLinkTarget { name: String },
    /// `links.create`/`links.remove` was given a value — in the subject or the object position —
    /// that is neither a handle nor a name string.
    WrongLinkTargetType { type_name: &'static str },
    /// An append's `told_by` was given a name string that is not a known memory.
    UnknownTeller { name: String },
    /// An append's `told_by` was given a value that is neither a memory handle nor a name string.
    WrongTellerType { type_name: &'static str },
    /// An `exclude` list named a memory that does not exist — the party to withhold the entry from
    /// could not be resolved.
    UnknownExcludee { name: String },
    /// An `exclude` entry was a value that is neither a memory handle nor a name string.
    WrongExcludeeType { type_name: &'static str },
    /// `memory.get`/`get_or_create` was given a handle whose id resolves to no memory.
    UnknownMemoryHandle { id: String },
    /// `memory.get`/`get_or_create` was given a value that is neither a name string nor a memory
    /// handle.
    WrongGetArgType { type_name: &'static str },
    /// A handle method was reached with a dot (`memory.append(...)`) rather than a colon
    /// (`memory:append(...)`), so the first argument bound to `self` — a string or number where the
    /// handle was wanted. Raised at the `self` extractor (the method's leftmost argument, converted
    /// first), so the agent sees this fix rather than mlua's opaque "error converting Lua string to
    /// table".
    MethodCalledWithDot { type_name: &'static str },
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandleError::InvalidMemoryHandle { id, source } => {
                write!(f, "invalid memory handle id {id:?}: {source}")
            }
            HandleError::InvalidEntryHandle { id, source } => {
                write!(f, "invalid entry handle id {id:?}: {source}")
            }
            HandleError::WrongEntryType { type_name } => write!(
                f,
                "retract's entry must be an entry object (from <memory>:entries or \
                 <memory>:history) or an entry-id string, got {type_name}"
            ),
            HandleError::UnknownLinkTarget { name } => {
                // The operator anchor is minted by the imprint, never by the agent, so the generic
                // "create it first" advice would steer a write at a reserved handle. Teach the real
                // shape instead: operator facts and links belong on the operator's actual profile.
                if name == MemoryName::from(NamespacedMemoryName::operator()).as_str() {
                    write!(
                        f,
                        "person/operator does not exist yet — it is a provisional anchor minted \
                         when an operator imprints, never created directly. Link to the operator's \
                         real person/<name> profile instead; if you do not know who the operator \
                         is, there is nothing to link"
                    )
                } else {
                    write!(
                        f,
                        "no memory named \"{name}\" — create it first, or check the casing"
                    )
                }
            }
            HandleError::WrongLinkTargetType { type_name } => write!(
                f,
                "a link's subject and object must each be a memory handle (from memory.get/create) \
                 or a memory name, got {type_name}"
            ),
            HandleError::UnknownTeller { name } => write!(
                f,
                "no memory named \"{name}\" to attribute this entry to — told_by names the \
                 participant who told you the fact (a person handle, or their memory name). When \
                 you yourself are the source, omit told_by and pass by_agent = true instead. For a \
                 real teller, create their memory first, or check the casing"
            ),
            HandleError::WrongTellerType { type_name } => write!(
                f,
                "told_by must be a person handle (from memory.get/create) or a memory name, \
                 got {type_name}"
            ),
            HandleError::UnknownExcludee { name } => write!(
                f,
                "no memory named \"{name}\" to exclude — exclude names the parties to withhold this \
                 from (each a person handle from memory.get/create, or their memory name). Create \
                 their memory first — a bare memory.create(\"person/<name>\") stub suffices — or \
                 check the casing"
            ),
            HandleError::WrongExcludeeType { type_name } => write!(
                f,
                "exclude takes a list of person handles (from memory.get/create) or memory names, \
                 e.g. exclude = {{ \"person/dave\" }}; one entry was {type_name}"
            ),
            HandleError::UnknownMemoryHandle { id } => write!(
                f,
                "this memory handle (id {id:?}) resolves to no memory — it may name one that no \
                 longer exists"
            ),
            HandleError::WrongGetArgType { type_name } => write!(
                f,
                "memory.get takes a memory name or an existing memory handle (from memory.list, \
                 memory.create, or a prior memory.get), got {type_name}"
            ),
            HandleError::MethodCalledWithDot { type_name } => write!(
                f,
                "this is a method — call it with a colon (handle:method(...)), not a dot \
                 (handle.method(...)); the colon passes the handle as self, but the dot bound \
                 {type_name} there instead"
            ),
        }
    }
}

impl std::error::Error for HandleError {}

impl From<HandleError> for LuaError {
    fn from(error: HandleError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// How many characters of an entry's text a `find_entry` ambiguity candidate line shows, so the agent
/// can tell the matches apart without the message running long.
const FIND_ENTRY_SNIPPET_CHARS: usize = 60;

/// A `mem:find_entry` call that cannot resolve to a single entry. The needle folds case and diacritics
/// and matches as a substring against the memory's live entries; a lone match returns that entry and no
/// match returns `nil`, so the only failures are a needle that names nothing distinctly enough. Both
/// are teachable — the agent reads them and reissues — so they are unprefixed prose.
#[derive(Debug)]
pub(super) enum FindEntryError {
    /// The needle was empty or whitespace. A match-anything needle is a scan, not a find, so it is
    /// refused pointing at a distinctive phrase.
    EmptyNeedle,
    /// The needle matched more than one live entry. Silently taking the first is the correct-the-wrong-
    /// entry hazard, so the ambiguity surfaces with each candidate's id and a snippet, so the agent
    /// narrows the phrase or addresses one by its id.
    Ambiguous {
        needle: String,
        candidates: Vec<(EntryId, String)>,
    },
}

impl std::fmt::Display for FindEntryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FindEntryError::EmptyNeedle => write!(
                f,
                "find_entry needs some text to match on — an empty needle would match every entry. \
                 Pass a distinctive phrase from the entry you mean, e.g. \
                 mem:find_entry(\"leads the volcano project\")"
            ),
            FindEntryError::Ambiguous { needle, candidates } => {
                write!(
                    f,
                    "the text {needle:?} matches more than one entry on this memory; use a longer \
                     phrase, or address one by its id:"
                )?;
                for (id, snippet) in candidates {
                    write!(f, "\n  {} — {}", id.0, find_entry_snippet(snippet))?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for FindEntryError {}

impl From<FindEntryError> for LuaError {
    fn from(error: FindEntryError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A one-line snippet of an entry's text for a `find_entry` ambiguity candidate — clipped to
/// [`FIND_ENTRY_SNIPPET_CHARS`] with an ellipsis so the message stays compact.
fn find_entry_snippet(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= FIND_ENTRY_SNIPPET_CHARS {
        return text.to_owned();
    }
    let clipped: String = text.chars().take(FIND_ENTRY_SNIPPET_CHARS).collect();
    format!("{clipped}…")
}

/// A bad argument to `table.concat`, reworded from Luau's opaque native error. Stock Luau `table.concat`
/// joins only strings and numbers, so a reader's handle list — `mem:entries()`, `hub:links()` — fails
/// it with the unhelpful "invalid value (table) at index … in table for 'concat'". The thin wrapper in
/// [`crate::agent::lua::runtime::install_table_concat`] keeps stock semantics (it delegates the join untouched) and
/// only replaces that error: these two variants redirect the two observed slips — a whole reader
/// *method* passed in place of its result, and a handle list that has no join, which now points at
/// string interpolation as the way to compose text.
#[derive(Debug)]
pub(super) enum ConcatError {
    /// The list argument was not a table at all — most often a reader *method* passed in place of its
    /// result (`hub.links`, a function, rather than `hub:links()`).
    NotAList { type_name: &'static str },
    /// The list held a value `table.concat` cannot join — a handle list, most likely. Composing text
    /// from handles is interpolation's job now, not concat's.
    NonJoinable,
}

impl std::fmt::Display for ConcatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConcatError::NotAList { type_name } => write!(
                f,
                "table.concat joins a list, but its first argument is {type_name}, not a table. If \
                 you meant a memory reader like hub:links() or mem:entries(), call it with a colon \
                 and parentheses — hub.links is the method itself, hub:links() is the list it returns"
            ),
            ConcatError::NonJoinable => write!(
                f,
                "table.concat joins only strings and numbers, but this list holds values it cannot — \
                 a handle list like mem:entries() or hub:links(), most likely. To see the list, \
                 print(list) already renders each handle on its own line — no join needed. To compose \
                 text from handles, interpolate one into a backtick string — `latest: {{es[1]}}` \
                 renders it as its text — or, since a handle concatenates directly (\"- \" .. e \
                 renders one), build the string with .. in a loop."
            ),
        }
    }
}

impl std::error::Error for ConcatError {}

impl From<ConcatError> for LuaError {
    fn from(error: ConcatError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// Luau's incomplete-statement syntax error, reworded to teach the explicit `return`. A block that
/// ends in a bare trailing expression — `results` on its own last line, as if the VM echoed input like
/// a REPL — fails Luau's parser with "Incomplete statement: expected assignment or a function call",
/// which never points at the fix: a block yields its value with an explicit `return`. The rewrite keeps
/// Luau's own position info (the chunk is named `block`, so no host path leaks) and appends the lesson;
/// the script is never re-parsed or mutated.
#[derive(Debug)]
pub(super) struct MissingReturnError {
    /// Luau's original message, carrying the `[string "block"]:line:col:` position.
    pub message: String,
}

impl std::fmt::Display for MissingReturnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "syntax error: {}. A block does not echo a trailing expression the way a REPL does — \
             yield its value with an explicit return (e.g. `return results` on the last line).",
            self.message
        )
    }
}

impl std::error::Error for MissingReturnError {}

impl From<MissingReturnError> for LuaError {
    fn from(error: MissingReturnError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// An attempt to assign to a field on a read-only handle (a memory, an entry, a date, or a search
/// result). A handle is a view, not a mutable record, so a field assignment silently did nothing —
/// the footgun behind the stale-date thrash, where `entry.occurred_at = ...` looked like it landed a
/// date but stored nothing. The message names the operation that actually persists the change.
#[derive(Debug)]
pub(super) enum HandleAssignmentError {
    /// Assigning `occurred_at` — a fact's date lives on an entry and is set when it is recorded, not
    /// by mutating a handle's field.
    OccurredAt { kind: HandleKind },
    /// Assigning any other field.
    Other { kind: HandleKind, field: String },
}

/// Which read-only handle an assignment was attempted on, for the assignment error's wording.
#[derive(Debug, Clone, Copy)]
pub(super) enum HandleKind {
    Memory,
    Entry,
    Date,
    SearchResult,
}

impl HandleKind {
    fn label(self) -> &'static str {
        match self {
            HandleKind::Memory => "memory handle",
            HandleKind::Entry => "entry",
            HandleKind::Date => "date object",
            HandleKind::SearchResult => "search result",
        }
    }
}

impl std::fmt::Display for HandleAssignmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandleAssignmentError::OccurredAt { kind } => write!(
                f,
                "occurred_at is not assignable on a {}: a fact's date is set when you record it, not \
                 by writing the field. Append the dated entry with occurred_at in its opts \
                 (mem:append(text, {{ occurred_at = calendar.date(\"YYYY-MM-DD\") }})), or revise the \
                 dated entry (mem:revise(entry, text, {{ occurred_at = ... }}))",
                kind.label()
            ),
            HandleAssignmentError::Other { kind, field } => write!(
                f,
                "{field} is not assignable: a {} is a read-only view, so writing its field does \
                 nothing. To change what is stored, use the memory methods (mem:append, mem:revise, \
                 mem:supersede, mem:rename, …) — a field assignment does not persist",
                kind.label()
            ),
        }
    }
}

impl std::error::Error for HandleAssignmentError {}

impl From<HandleAssignmentError> for LuaError {
    fn from(error: HandleAssignmentError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A free-text argument that carries a literal `{ident}`-shaped placeholder — string-format syntax
/// (`mem:append("Full text: {content}")`) that a plain quoted string never interpolates, so the
/// uninterpolated braces would be stored (or searched) as fact. Raised at the Lua argument boundary,
/// where the script's own text crosses into the API, so it catches the slip at its point of failure
/// and points at the backtick string that does interpolate — the same vocabulary the [`ConcatError`]
/// teachable error uses. Genesis and console writes never pass through a script, so they may carry
/// literal braces (the scaffold's `{es[1]}` examples among them).
#[derive(Debug)]
pub(super) struct PlaceholderError {
    /// The argument the offending text was passed as, for the error's wording ("entry text",
    /// "memory name", …).
    pub what: &'static str,
    /// The matched placeholder including its braces (e.g. `"{content}"`).
    pub placeholder: String,
}

impl std::fmt::Display for PlaceholderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let PlaceholderError { what, placeholder } = self;
        write!(
            f,
            "the {what} contains \"{placeholder}\" as literal characters — a plain quoted string does \
             not interpolate. To insert a value, use a backtick string, which does: \
             `Full text: {{content}}` renders the variable content in place. If the braces are meant \
             literally, rephrase so the braces do not wrap a bare identifier"
        )
    }
}

impl std::error::Error for PlaceholderError {}

impl From<PlaceholderError> for LuaError {
    fn from(error: PlaceholderError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A write attempted through a `memory.search` hit the query did not name — the fuzzy-write guard.
/// A hit carries the query it came from; when a content write (`:append`, `:revise`, `:supersede`) or
/// a `links.create` endpoint goes through a hit whose query does not name the handle it landed on
/// (searching "Davina", landing person/david, then writing her role onto him), the write is refused
/// rather than committed to the wrong referent. The message names the query and the handle it names
/// instead, and points at the three-way confirmation: fetch the handle if it really is them, list the
/// shared stem to see who else it could be, or create a new memory if they are new. `list_arg` and
/// `create_handle` are precomputed by the guard so the message can suggest concrete calls.
#[derive(Debug)]
pub(super) struct SearchWriteError {
    /// The query the hit came from.
    pub query: String,
    /// The handle the hit landed on (the memory the query did not name).
    pub name: String,
    /// The stem to `memory.list` — the query and the handle's shared prefix, or the namespace when
    /// they share none — so the agent sees who else shares it.
    pub list_arg: String,
    /// The handle `memory.create` would mint for the searched name, offered for when they are new.
    pub create_handle: String,
}

impl std::fmt::Display for SearchWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let SearchWriteError {
            query,
            name,
            list_arg,
            create_handle,
        } = self;
        write!(
            f,
            "this handle came from a search for {query:?} but names {name} — a search hit is a \
             candidate, not a match, so writing to it here would record against the wrong memory. \
             Confirm who you mean first: memory.get(\"{name}\") if this really is them, \
             memory.list(\"{list_arg}\") to see who else shares the stem, or \
             memory.create(\"{create_handle}\") if they are new"
        )
    }
}

impl std::error::Error for SearchWriteError {}

impl From<SearchWriteError> for LuaError {
    fn from(error: SearchWriteError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A write to a memory this block's search surfaced without the query naming it — the block-scoped
/// taint guard (distinct from [`SearchWriteError`], which gates the search hit's own handle). The
/// launder it closes: composing one block that searches "Davina", then in an else-branch writes to the
/// mismatched person/david hit through a provenance-free `memory.get(hits[1].name)` handle, which the
/// hit-handle guard never sees. Because the whole block is written before its searches run, the branch
/// on the hit is a guess with no judgement behind it, so the write is refused and the practice — decide
/// at the block boundary, where the results are finally visible — is taught. `create_handle` is the
/// handle `memory.create` would mint for the searched name, precomputed so the message can suggest it.
#[derive(Debug)]
pub(super) struct TaintedWriteError {
    /// The query the mismatched search came from.
    pub query: String,
    /// The memory the query surfaced but did not name — the write's refused target.
    pub name: String,
    /// The handle `memory.create` would mint for the searched name, offered for when they are new.
    pub create_handle: String,
}

impl std::fmt::Display for TaintedWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let TaintedWriteError {
            query,
            name,
            create_handle,
        } = self;
        write!(
            f,
            "a search in this block for {query:?} surfaced {name} without naming it — a hit is a \
             candidate, not a match, and this block was written before the results were visible. \
             Finish this block by returning what you found, then decide in your next block: \
             memory.get(\"{name}\") if it is really them, or memory.create(\"{create_handle}\") if \
             they are new"
        )
    }
}

impl std::error::Error for TaintedWriteError {}

impl From<TaintedWriteError> for LuaError {
    fn from(error: TaintedWriteError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// A block-consistency invariant: an entry the block just buffered could not be read back. A bug,
/// not the agent's doing — surfaced as a catchable error rather than a panic so a misbehaving build
/// does not tear the whole session down.
#[derive(Debug)]
pub(super) enum BlockConsistencyError {
    AppendedEntryMissing,
    RevisedEntryMissing,
}

impl std::fmt::Display for BlockConsistencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockConsistencyError::AppendedEntryMissing => {
                write!(f, "the appended entry was not found in the block buffer")
            }
            BlockConsistencyError::RevisedEntryMissing => {
                write!(f, "the revised entry was not found in the block buffer")
            }
        }
    }
}

impl std::error::Error for BlockConsistencyError {}

impl From<BlockConsistencyError> for LuaError {
    fn from(error: BlockConsistencyError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}
