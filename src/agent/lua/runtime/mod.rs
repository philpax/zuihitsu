//! The block-scoped seam ([`BlockApi`]), the per-block lock set, and the free helpers shared between
//! the lifecycle (`mod.rs`) and the Lua-table builders (`tables.rs`): lock acquisition and release,
//! handle minting, error routing, the `memory.search` runner, and value rendering.

mod handles;
mod inspect;
mod search;
mod temporal;

use std::{collections::HashMap, sync::Arc};

use mlua::{Lua, Value};
use parking_lot::Mutex;
use tokio::sync::OwnedMutexGuard;

use crate::{
    engine::MemoryLocks,
    event::TerminalCause,
    graph::GraphError,
    ids::MemoryId,
    memory::memory_block::{MemoryBlock, MemoryError},
};

use super::error::MissingReturnError;

pub(crate) use handles::{
    HandleSelf, entry_handle_id, get_argument_name, handle_id, link_target_id,
    make_capped_handle_list, make_entry_handle, make_entry_handle_list, make_handle,
    make_handle_list, make_link_handle_list, make_relation_result, readonly_newindex,
    render_details, render_neighborhood, render_salient_relations,
};
pub(crate) use inspect::{
    combine_output, concat_via_tostring, date_text, install_inspect, install_table_concat, render,
    value_text,
};
pub(crate) use search::{SearchOpts, run_memory_search};
pub(crate) use temporal::{append_options_from_lua, day_string, make_date};

/// The chunk name given to an agent block, so a syntax or runtime error names the block rather than
/// leaking mlua's default (the Rust caller's `file:line`, which the agent should never see).
pub(super) const BLOCK_CHUNK_NAME: &str = "block";

/// The block-scoped handles every memory-API closure captures: the transaction (`block`), the
/// infrastructure-error slot (`infra`), the per-block lock set (`lock_set`), and the server-wide lock
/// registry (`manager`). Bundled so the install helpers pass one seam rather than four parallel
/// arguments, and the `'static` async closures clone one value. `Clone` clones the inner `Arc`s.
#[derive(Clone)]
pub(super) struct BlockApi {
    pub(super) block: Arc<Mutex<MemoryBlock>>,
    pub(super) infra: Arc<Mutex<Option<GraphError>>>,
    pub(super) lock_set: Arc<Mutex<LockSet>>,
    pub(super) manager: Arc<MemoryLocks>,
    /// What the block wrote with `print(...)`, accumulated across the script and folded into the
    /// block's agent-visible result. Without this, `print` output is lost, so an agent that inspects a
    /// query by printing it (rather than returning it) sees nothing come back — the recall failure mode.
    pub(super) printed: Arc<Mutex<String>>,
}

impl BlockApi {
    /// Acquire `id`'s lock (unless already held), holding the owned guard in the lock set to block end.
    pub(super) async fn lock(&self, id: MemoryId) {
        ensure_locked(&self.lock_set, &self.manager, [id]).await;
    }

    /// Acquire the locks for `ids` (skipping any already held) — the multi-memory operations (a link's
    /// two endpoints, a calendar query's whole result set).
    pub(super) async fn lock_all(&self, ids: impl IntoIterator<Item = MemoryId>) {
        ensure_locked(&self.lock_set, &self.manager, ids).await;
    }

    /// Lock the whole `same_as` class of `id` (plus `id` itself) before a traversing read, so a
    /// concurrent write to a sibling stub cannot tear the merged view (spec §Concurrency → class-wide
    /// locking). The class membership is read lock-free through the block; a graph failure routes to
    /// `infra`. The class boundary is read-then-locked, so a concurrent operator merge can shift it —
    /// an accepted edge the timeout backstops (a platform turn cannot merge).
    pub(super) async fn lock_class(&self, id: MemoryId) -> mlua::Result<()> {
        let members = self
            .block
            .lock()
            .class_members(id)
            .map_err(|error| route_error(error, &mut self.infra.lock()))?;
        ensure_locked(
            &self.lock_set,
            &self.manager,
            std::iter::once(id).chain(members),
        )
        .await;
        Ok(())
    }
}

/// The per-memory locks a block holds, keyed by memory and released together at block end (spec
/// §Concurrency → lifetime is the code block). The owned guards live here, not in the closures, so
/// [`release_locks`] can drop them deterministically at the end of `execute`.
#[derive(Default)]
pub(super) struct LockSet {
    held: HashMap<MemoryId, OwnedMutexGuard<()>>,
}

impl LockSet {
    fn holds(&self, id: MemoryId) -> bool {
        self.held.contains_key(&id)
    }

    fn insert(&mut self, id: MemoryId, guard: OwnedMutexGuard<()>) {
        self.held.insert(id, guard);
    }

    fn take(&mut self) -> Vec<OwnedMutexGuard<()>> {
        std::mem::take(&mut self.held).into_values().collect()
    }
}

/// Acquire the registry lock for each id not already held by `lock_set`, recording each owned guard.
/// The `lock_set` `parking_lot` guard is taken only to test membership and to insert, never held across
/// the acquire `.await`; the only long-held locks are the per-memory ones, so two blocks acquiring in
/// opposite orders deadlock only until the per-block timeout breaks and retries them (spec §Concurrency
/// → timeout-and-retry, not an ordering protocol). Within one block the calls are sequential (Lua runs
/// one operation at a time), so the membership test is race-free and never double-acquires an id.
async fn ensure_locked(
    lock_set: &Arc<Mutex<LockSet>>,
    manager: &Arc<MemoryLocks>,
    ids: impl IntoIterator<Item = MemoryId>,
) {
    for id in ids {
        if lock_set.lock().holds(id) {
            continue;
        }
        let guard = manager.acquire(id).await;
        lock_set.lock().insert(id, guard);
    }
}

/// Drain and drop the block's lock guards, releasing the per-memory locks so the next block (here or in
/// another conversation) can take them. The `'static` Lua closures still hold `Arc` clones of the
/// now-empty lock set, but no longer any guard — a leaked guard would deadlock the next block touching
/// that memory, so this is called on every exit path of `execute`.
pub(super) fn release_locks(lock_set: &Arc<Mutex<LockSet>>) {
    let guards = lock_set.lock().take();
    drop(guards);
}

/// The terminal cause for a block that blew its time budget: the budget in seconds, plus — when the
/// block exhausted its retries without an MCP call — the attempt count, so the give-up is auditable.
pub(super) fn timed_out_cause(budget: std::time::Duration, attempts: Option<u32>) -> TerminalCause {
    let secs = budget.as_secs();
    let message = match attempts {
        Some(attempts) => format!(
            "the block exceeded its time budget of {secs}s on each of {attempts} attempts and was aborted"
        ),
        None => format!("the block exceeded its time budget of {secs}s and was aborted"),
    };
    TerminalCause::Error(message)
}

/// Load and evaluate one agent block, naming its chunk `block` so a syntax or runtime error names the
/// block rather than leaking mlua's default caller location, and rewording Luau's incomplete-statement
/// syntax error into the teachable [`MissingReturnError`]. Every other load or runtime error passes
/// through untouched.
pub(super) async fn eval_block(lua: &Lua, script: &str) -> mlua::Result<Value> {
    lua.load(script)
        .set_name(BLOCK_CHUNK_NAME)
        .eval_async::<Value>()
        .await
        .map_err(teach_block_load_error)
}

/// Reword the incomplete-statement syntax error Luau raises when a block ends in a bare trailing
/// expression (`results` on its own last line, as if the VM echoed input like a REPL) into a teachable
/// one that names the fix: yield the value with an explicit `return`. A pure rewrite — the script is
/// never re-parsed or mutated — that touches only that syntax error; anything else is returned as-is.
fn teach_block_load_error(error: mlua::Error) -> mlua::Error {
    match &error {
        mlua::Error::SyntaxError { message, .. } if message.contains("Incomplete statement") => {
            MissingReturnError {
                message: message.clone(),
            }
            .into()
        }
        _ => error,
    }
}

/// Route a memory operation's error. A teachable violation (a duplicate name, an unknown relation)
/// becomes the Lua runtime error the agent sees as the block's terminal cause. A graph read failure
/// is infrastructure, not the agent's doing: it is stashed in the caller's `infra` slot for `execute`
/// to bubble up as a [`super::LuaError`], and the returned Lua error only serves to stop the script.
pub(super) fn route_error(error: MemoryError, infra: &mut Option<GraphError>) -> mlua::Error {
    match error {
        MemoryError::Graph(graph_error) => {
            *infra = Some(graph_error);
            mlua::Error::RuntimeError("internal graph error".to_owned())
        }
        teachable => mlua::Error::RuntimeError(teachable.to_string()),
    }
}
