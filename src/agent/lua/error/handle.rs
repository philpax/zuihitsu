//! Handle-resolution and assignment errors for the Lua interface: bad handles and link targets,
//! `find_entry` ambiguity, read-only-handle field assignment, and literal placeholder text. Each is a
//! teachable message that names the fix at its point of failure.

use mlua::Error as LuaError;
use ulid::DecodeError as UlidError;

use crate::{
    agent::{body_of, render_placeholders},
    ids::{EntryId, MemoryName, NamespacedMemoryName},
};

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
    /// position — that is not a known memory, where minting one is not on offer: an unlink (which
    /// never creates its target), or a name `links.create` declines to mint (a machinery-owned
    /// handle, or one in no recognised namespace).
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
            HandleError::WrongEntryType { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/wrong_entry_type.md")),
                &[("type_name", type_name)],
            )),
            HandleError::UnknownLinkTarget { name } => {
                // The operator anchor is minted by the imprint, never by the agent, so the generic
                // "create it first" advice would steer a write at a reserved handle. Teach the real
                // shape instead: operator facts and links belong on the operator's actual profile.
                if name == MemoryName::from(NamespacedMemoryName::operator()).as_str() {
                    f.write_str(&body_of(include_str!(
                        "prose/handle/unknown_link_target_operator.md"
                    )))
                } else {
                    f.write_str(&render_placeholders(
                        body_of(include_str!("prose/handle/unknown_link_target.md")),
                        &[("name", name)],
                    ))
                }
            }
            HandleError::WrongLinkTargetType { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/wrong_link_target_type.md")),
                &[("type_name", type_name)],
            )),
            HandleError::UnknownTeller { name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/unknown_teller.md")),
                &[("name", name)],
            )),
            HandleError::WrongTellerType { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/wrong_teller_type.md")),
                &[("type_name", type_name)],
            )),
            HandleError::UnknownExcludee { name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/unknown_excludee.md")),
                &[("name", name)],
            )),
            HandleError::WrongExcludeeType { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/wrong_excludee_type.md")),
                &[("type_name", type_name)],
            )),
            HandleError::UnknownMemoryHandle { id } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/unknown_memory_handle.md")),
                &[("id", &format!("{id:?}"))],
            )),
            HandleError::WrongGetArgType { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/wrong_get_arg_type.md")),
                &[("type_name", type_name)],
            )),
            HandleError::MethodCalledWithDot { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/method_called_with_dot.md")),
                &[("type_name", type_name)],
            )),
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
            FindEntryError::EmptyNeedle => {
                f.write_str(&body_of(include_str!("prose/handle/find_entry_empty.md")))
            }
            FindEntryError::Ambiguous { needle, candidates } => {
                f.write_str(&render_placeholders(
                    body_of(include_str!("prose/handle/find_entry_ambiguous.md")),
                    &[("needle", &format!("{needle:?}"))],
                ))?;
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
            HandleAssignmentError::OccurredAt { kind } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/assign_occurred_at.md")),
                &[("kind", kind.label())],
            )),
            HandleAssignmentError::Other { kind, field } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/handle/assign_other.md")),
                &[("field", field), ("kind", kind.label())],
            )),
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
        f.write_str(&render_placeholders(
            body_of(include_str!("prose/handle/placeholder_error.md")),
            &[("what", what), ("placeholder", placeholder)],
        ))
    }
}

impl std::error::Error for PlaceholderError {}

impl From<PlaceholderError> for LuaError {
    fn from(error: PlaceholderError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}
