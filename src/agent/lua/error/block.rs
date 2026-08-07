//! Block-assembly and write-consistency invariants for the Lua interface: the `table.concat` reword,
//! the missing-`return` syntax lesson, the fuzzy-write and tainted-write guards, and the block-buffer
//! consistency checks. Each converts to a runtime error via its `Display`.

use mlua::Error as LuaError;

use crate::agent::{body_of, render_placeholders};

/// A bad argument to `table.concat`, reworded from Luau's opaque native error. Stock Luau `table.concat`
/// joins only strings and numbers, so a reader's handle list — `mem:entries()`, `hub:links()` — fails
/// it with the unhelpful "invalid value (table) at index … in table for 'concat'". The thin wrapper in
/// [`crate::agent::lua::runtime::install_table_concat`] keeps stock semantics (it delegates the join untouched) and
/// only replaces that error: these two variants redirect the two observed slips — a whole reader
/// *method* passed in place of its result, and a handle list that has no join, which now points at
/// string interpolation as the way to compose text.
#[derive(Debug)]
pub(in crate::agent::lua) enum ConcatError {
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
            ConcatError::NotAList { type_name } => f.write_str(&render_placeholders(
                body_of(include_str!("prose/block/not_a_list.md")),
                &[("type_name", type_name)],
            )),
            ConcatError::NonJoinable => {
                f.write_str(&body_of(include_str!("prose/block/non_joinable.md")))
            }
        }
    }
}

impl std::error::Error for ConcatError {}

impl From<ConcatError> for LuaError {
    fn from(error: ConcatError) -> Self {
        LuaError::RuntimeError(error.to_string())
    }
}

/// Append a lesson to a block's terminal message when Luau's own wording names a slip we can teach.
/// Applied where a block's runtime error becomes its terminal cause, so a lesson lands wherever the
/// slip happened rather than only at the surfaces that route through a typed error.
///
/// Only `..` against a table is recognized. The read surfaces already give a single handle a
/// `__concat`, so a table reaching here is a collection — most often a handle list from
/// `mem:entries()` or `hub:links()`. The lesson is worded as the likely case rather than a diagnosis,
/// since the predicate cannot tell a handle list from any other table the script built, and a
/// confidently wrong explanation is worse than Luau's bare one. The reword is additive: the original
/// message, position and all, is kept ahead of the lesson.
pub(in crate::agent::lua) fn with_lesson(message: String) -> String {
    let lesson = body_of(include_str!("prose/block/concat_lesson.md"));
    if message.contains("attempt to concatenate")
        && message.contains("table")
        && !message.contains(&lesson)
    {
        return format!("{message}\n{lesson}");
    }
    message
}

/// Luau's incomplete-statement syntax error, reworded to teach the explicit `return`. A block that
/// ends in a bare trailing expression — `results` on its own last line, as if the VM echoed input like
/// a REPL — fails Luau's parser with "Incomplete statement: expected assignment or a function call",
/// which never points at the fix: a block yields its value with an explicit `return`. The rewrite keeps
/// Luau's own position info (the chunk is named `block`, so no host path leaks) and appends the lesson;
/// the script is never re-parsed or mutated.
#[derive(Debug)]
pub(in crate::agent::lua) struct MissingReturnError {
    /// Luau's original message, carrying the `[string "block"]:line:col:` position.
    pub message: String,
}

impl std::fmt::Display for MissingReturnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&render_placeholders(
            body_of(include_str!("prose/block/missing_return.md")),
            &[("message", &self.message)],
        ))
    }
}

impl std::error::Error for MissingReturnError {}

impl From<MissingReturnError> for LuaError {
    fn from(error: MissingReturnError) -> Self {
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
pub(in crate::agent::lua) struct SearchWriteError {
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
        f.write_str(&render_placeholders(
            body_of(include_str!("prose/block/search_write.md")),
            &[
                ("query", &format!("{query:?}")),
                ("name", name),
                ("list_arg", list_arg),
                ("create_handle", create_handle),
            ],
        ))
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
pub(in crate::agent::lua) struct TaintedWriteError {
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
        f.write_str(&render_placeholders(
            body_of(include_str!("prose/block/tainted_write.md")),
            &[
                ("query", &format!("{query:?}")),
                ("name", name),
                ("create_handle", create_handle),
            ],
        ))
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
// Each variant names the operation whose just-buffered entry could not be read back; the shared
// `EntryMissing` suffix is the point (they are the same class of bug), not redundant naming.
#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub(in crate::agent::lua) enum BlockConsistencyError {
    AppendedEntryMissing,
    RevisedEntryMissing,
    AttestedEntryMissing,
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
            BlockConsistencyError::AttestedEntryMissing => {
                write!(f, "the attested entry could not be read back")
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
