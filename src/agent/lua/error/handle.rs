//! Handle-resolution and assignment errors for the Lua interface: bad handles and link targets,
//! `find_entry` ambiguity, read-only-handle field assignment, and literal placeholder text. Each is a
//! teachable message that names the fix at its point of failure.

use mlua::Error as LuaError;
use ulid::DecodeError as UlidError;

use crate::ids::{EntryId, MemoryName, NamespacedMemoryName};

/// A bad handle or link target passed to a memory operation.
#[derive(Debug)]
pub(in crate::agent::lua) enum HandleError {
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
pub(in crate::agent::lua) enum FindEntryError {
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

/// An attempt to assign to a field on a read-only handle (a memory, an entry, a date, or a search
/// result). A handle is a view, not a mutable record, so a field assignment silently did nothing —
/// the footgun behind the stale-date thrash, where `entry.occurred_at = ...` looked like it landed a
/// date but stored nothing. The message names the operation that actually persists the change.
#[derive(Debug)]
pub(in crate::agent::lua) enum HandleAssignmentError {
    /// Assigning `occurred_at` — a fact's date lives on an entry and is set when it is recorded, not
    /// by mutating a handle's field.
    OccurredAt { kind: HandleKind },
    /// Assigning any other field.
    Other { kind: HandleKind, field: String },
}

/// Which read-only handle an assignment was attempted on, for the assignment error's wording.
#[derive(Debug, Clone, Copy)]
pub(in crate::agent::lua) enum HandleKind {
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
pub(in crate::agent::lua) struct PlaceholderError {
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
