//! Block execution — the transactional loop that runs a Lua script, handles timeouts and retries,
//! and commits the buffered effects or a terminal cause.

use std::{sync::Arc, time::Instant};

use parking_lot::Mutex;

use crate::{
    engine::Engine,
    event::{EventSource, TerminalCause},
    ids::MemoryId,
    memory::memory_block::{BlockEffects, MemoryBlock},
};

use super::{
    BlockOutcome, LuaError, Session,
    runtime::{
        BlockApi, LockSet, combine_output, eval_block, release_locks, render, timed_out_cause,
    },
};

impl Session {
    /// Execute one block as a transaction. On a clean run, the buffered side effects plus a
    /// `LuaExecuted` commit together; on error or abort, only a `LuaExecuted` recording the terminal
    /// cause is written. The graph is brought up to log-head afterward either way.
    ///
    /// The block acquires the lock on each memory it touches and holds it to block end (spec
    /// §Concurrency → per-memory mutual exclusion), so a concurrent block in another conversation
    /// serializes on a shared memory. If the block outruns its time budget — stuck on slow external
    /// I/O or on a lock-wait — it aborts, releases its locks, and **retries from scratch**, bounded by
    /// `max_block_attempts`; the exception is a block that has already made an MCP call, whose external
    /// effect cannot be rolled back, so its timeout surfaces as a terminal error with no retry (spec
    /// §645). A retried block re-runs the Lua from scratch; the VM globals (the scratchpad) persist and
    /// are not transactional, so a non-idempotent scratchpad write is observable across attempts.
    pub async fn execute(
        &self,
        engine: &Arc<Engine>,
        context: &super::super::BlockContext,
        script: &str,
    ) -> Result<BlockOutcome, LuaError> {
        let manager = engine.memory_locks.clone();
        let max_attempts = context.max_block_attempts.max(1);
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            // Each attempt is a fresh transaction over a fresh lock set, bundled with the infra-error
            // slot and the lock registry as the one [`BlockApi`] seam the install helpers and their
            // `'static` async closures share. The block owns the buffer and the write invariants;
            // `lock_set` holds the owned per-memory guards until block end.
            let api = BlockApi {
                block: Arc::new(Mutex::new(MemoryBlock::new(
                    engine.clone(),
                    context.teller.clone(),
                    context.authority,
                    self.conversation,
                    Some(context.turn_id),
                    context.present_set.clone(),
                    context.max_entry_chars,
                )?)),
                infra: Arc::new(Mutex::new(None)),
                lock_set: Arc::new(Mutex::new(LockSet::default())),
                manager: manager.clone(),
                printed: Arc::new(Mutex::new(String::new())),
            };

            // The handle metatable and its methods table back every memory handle the API mints; the
            // entry metatable backs the addressable content-entry handles that `mem:append` /
            // `mem:entries` / `mem:history` return (text-rendering, so reading stays ergonomic).
            let methods = self.lua.create_table().map_err(LuaError::Vm)?;
            let metatable = self.lua.create_table().map_err(LuaError::Vm)?;
            // `__index` is wired in `install_block_api`: it resolves `handle.name` / `handle.description`
            // lazily from the id and otherwise dispatches to `methods`.
            let entry_metatable =
                super::tables::entry_metatable(&self.lua).map_err(LuaError::Vm)?;

            // Reset the per-attempt "made an MCP call" latch, so the no-retry decision below reflects
            // this attempt only.
            self.begin_mcp_block();

            // Installing the API is our-side setup: a failure here is a bug, not an agent-visible
            // outcome.
            super::tables::install_block_api(
                &self.lua,
                &api,
                &methods,
                &metatable,
                &entry_metatable,
                &self.features,
            )
            .map_err(LuaError::Vm)?;

            // The agent-visible outcome: the rendered final value, or the runtime error/abort that
            // ended the script, bounded by the block's time budget. The block's memory functions only
            // hold their parking_lot guards transiently, never across this suspension point. `started`
            // times this attempt's eval for the console's turn timeline (the final attempt's, since a
            // retry restarts it).
            let started = Instant::now();
            let timed =
                tokio::time::timeout(context.block_timeout, eval_block(&self.lua, script)).await;

            let Ok(evaluated) = timed else {
                // Timed out. Release the locks (so a retry, or another conversation, can take them) and
                // drop the in-flight MCP instance — its session-side state is now undefined.
                release_locks(&api.lock_set);
                self.drop_in_flight_mcp();
                if self.block_made_mcp_call() {
                    // An external effect happened this attempt; surface the timeout, do not retry.
                    let cause = timed_out_cause(context.block_timeout, None);
                    return self
                        .commit_terminal(engine, context, script, &api.block, cause, started);
                }
                if attempt >= max_attempts {
                    let cause = timed_out_cause(context.block_timeout, Some(attempt));
                    return self
                        .commit_terminal(engine, context, script, &api.block, cause, started);
                }
                // Abort-and-retry: the buffer emitted nothing, so a fresh attempt is the only trace.
                continue;
            };
            // The agent-visible result is the rendered final value, prefixed by anything the block
            // printed (so an agent that prints a query result instead of returning it still sees it).
            let evaluated = evaluated.map(|value| {
                let rendered = render(&self.lua, &value);
                let printed = std::mem::take(&mut *api.printed.lock());
                combine_output(printed, rendered)
            });

            // An infrastructure failure during the block (a graph read) takes precedence over the
            // script's apparent outcome: it bubbles up, discarding the buffer and releasing the locks,
            // rather than reaching the agent.
            if let Some(graph_error) = api.infra.lock().take() {
                release_locks(&api.lock_set);
                return Err(LuaError::Graph(graph_error));
            }

            // Drain the effects through the lock and commit. Locks are held through the commit so a
            // concurrent block sees consistent state, then released once the block is done.
            let BlockEffects {
                events,
                touched,
                aborted,
            } = api.block.lock().take_effects();
            let outcome = match evaluated {
                Ok(result) => {
                    // A dry run discards the whole buffer — including the `LuaExecuted` record — and
                    // commits nothing; the operator gets the rendered result over a clean log.
                    if context.dry_run {
                        Ok(BlockOutcome::Committed { result })
                    } else {
                        let mut events = events;
                        // Tell the agent what the block actually changed, so it sees its writes landed
                        // and does not re-issue them next turn for want of confirmation. Folded into the
                        // result the agent reads and the `LuaExecuted` record, so the log shows what was
                        // shown.
                        let result = super::commit::with_commit_summary(
                            result,
                            super::commit::summarize_committed(engine, &events),
                        );
                        events.push(self.lua_executed(
                            context.turn_id,
                            script,
                            Some(result.clone()),
                            touched,
                            None,
                            started.elapsed().as_millis() as u64,
                        ));
                        self.finish(engine, events, BlockOutcome::Committed { result })
                    }
                }
                Err(error) => {
                    // Discard the buffer; record only what the agent saw — the terminal cause.
                    let cause = match aborted {
                        Some(reason) => TerminalCause::Aborted(reason),
                        None => TerminalCause::Error(error.to_string()),
                    };
                    if context.dry_run {
                        Ok(BlockOutcome::Terminated(cause))
                    } else {
                        let event = self.lua_executed(
                            context.turn_id,
                            script,
                            None,
                            touched,
                            Some(cause.clone()),
                            started.elapsed().as_millis() as u64,
                        );
                        self.finish(engine, vec![event], BlockOutcome::Terminated(cause))
                    }
                }
            };
            release_locks(&api.lock_set);
            return outcome;
        }
    }

    /// Commit a block's terminal record (a discarded buffer, the touched set kept for the audit) and
    /// bring the graph to head — the shared tail of the timeout-give-up and no-retry-after-MCP paths.
    fn commit_terminal(
        &self,
        engine: &Engine,
        context: &super::super::BlockContext,
        script: &str,
        block: &Arc<Mutex<MemoryBlock>>,
        cause: TerminalCause,
        started: Instant,
    ) -> Result<BlockOutcome, LuaError> {
        let BlockEffects { touched, .. } = block.lock().take_effects();
        // A dry run commits nothing — not even the terminal record.
        if context.dry_run {
            return Ok(BlockOutcome::Terminated(cause));
        }
        let event = self.lua_executed(
            context.turn_id,
            script,
            None,
            touched,
            Some(cause.clone()),
            started.elapsed().as_millis() as u64,
        );
        self.finish(engine, vec![event], BlockOutcome::Terminated(cause))
    }

    fn lua_executed(
        &self,
        turn_id: crate::ids::TurnId,
        script: &str,
        result: Option<String>,
        touched: Vec<MemoryId>,
        terminal_cause: Option<TerminalCause>,
        duration_ms: u64,
    ) -> crate::event::EventPayload {
        crate::event::EventPayload::LuaExecuted {
            conversation: self.conversation,
            turn_id,
            script: script.to_owned(),
            result,
            touched,
            terminal_cause,
            duration_ms,
        }
    }

    /// Append the block's events (the durable commit point), bring the graph up to head, and return.
    fn finish(
        &self,
        engine: &Engine,
        events: Vec<crate::event::EventPayload>,
        outcome: BlockOutcome,
    ) -> Result<BlockOutcome, LuaError> {
        let now = engine.clock.now();
        engine
            .store
            .lock()
            .append(now, EventSource::Agent, events)?;
        // Two guards at once: graph (written) before store (read), per the lock-ordering rule.
        let mut graph = engine.graph.lock();
        graph.materialize_from(engine.store.lock().as_ref())?;
        Ok(outcome)
    }
}
