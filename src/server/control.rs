//! The operator-authority facet: agent creation and read-only inspection. A platform client can
//! never obtain one of these, which is what keeps the operator surface off the platform boundary
//! (spec §Clients and the server boundary).

use serde::{Deserialize, Serialize};

use std::time::Duration;

use super::{RoutedTurn, Server, ServerError};
use crate::{
    agent::{
        BlockContext, TurnOutcome,
        api_doc::ApiEntry,
        genesis::{self, GenesisStatus, Rollout, SeedSelf},
        lua::{self, BlockOutcome, Session},
        templates,
    },
    event::{
        Event, EventPayload, EventSource, ModelPhase, PromptTemplateName, RequestRecord, Teller,
        TerminalCause,
    },
    graph::{EntryView, MemoryView, SessionView},
    ids::{ConversationId, ConversationLocator, MemoryName, Seq, TurnId},
    memory::{identity::resolve_or_mint_conversation, memory_block::Authority},
    model::{Completion, ModelClient, Usage},
    settings::Settings,
    time::Timestamp,
};

/// Operator-authority operations: agent creation and read-only inspection. A platform client can
/// never obtain one of these.
pub struct Control<'a> {
    pub(super) server: &'a Server,
}

/// One recorded belief arbitration: the memory it concerns and the reconciling statement the agent
/// wrote (spec §Write path). The operator/console view of "why does it believe X".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Arbitration {
    pub memory: MemoryName,
    pub statement: String,
}

/// One recorded model interaction — the console's view of a single model call (spec
/// §Observability). The `seq` and `recorded_at` of the `ModelCalled` event place the call on the
/// timeline; the rest mirrors the event. The `request` is delta-encoded (`Base`/`Continuation`); the
/// console reconstructs a full prompt by walking a `(turn_id, phase)` group, and `request_digest`
/// checks the reconstruction against the call actually sent.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCall {
    pub seq: Seq,
    pub recorded_at: Timestamp,
    pub conversation: ConversationId,
    pub turn_id: TurnId,
    pub phase: ModelPhase,
    pub request_digest: String,
    pub request: Option<RequestRecord>,
    pub completion: Completion,
    pub reasoning: Option<String>,
    pub finish_reason: Option<String>,
    pub usage: Usage,
    pub duration_ms: u64,
}

/// The result of one operator Lua console run (spec §Observability → the operator Lua console): the
/// rendered value of the block's final expression, or the error/abort that ended it. Exactly one is
/// `Some`. The run is a no-commit sandbox — nothing it writes persists — so it leaves no trace on the
/// log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LuaConsoleOutcome {
    pub result: Option<String>,
    pub error: Option<String>,
}

impl Control<'_> {
    /// Run one operator message of the imprint interview: the console conversation where the
    /// operator introduces themselves and the agent learns who they are and what it is for (spec
    /// §Imprint interview). It runs under operator authority, so the agent may write `self` — the
    /// only path that may — and authors its links as `Operator`. The operator is a stable
    /// `person/operator` stub (minted on first contact, no platform binding); the agent learns their
    /// real name, creates `person/<name>`, and merges the two with `same_as`. Multi-turn and
    /// re-runnable: each call delivers one operator message and runs the agent's response. No
    /// compaction — the interview is short, and its flush would run barred from `self`.
    pub async fn imprint(
        &self,
        model: &dyn ModelClient,
        text: &str,
    ) -> Result<TurnOutcome, ServerError> {
        // The imprint runs the model too, so it takes a stream permit like any other turn (spec
        // §Concurrency), held across the interview turn below.
        let _stream = self
            .server
            .streams
            .acquire()
            .await
            .expect("the stream semaphore is never closed");
        let operator = self.server.resolve_or_mint_operator()?;
        let conversation = {
            // Graph before store, per the lock-ordering rule (this resolve holds both at once).
            let graph = self.server.engine.graph.lock();
            resolve_or_mint_conversation(
                self.server.engine.store.lock().as_mut(),
                self.server.engine.clock.as_ref(),
                &graph,
                &ConversationLocator::new("operator", "imprint"),
            )?
        };
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;
        let (report, _buffer) = self
            .server
            .run_session_turn(
                model,
                &RoutedTurn {
                    conversation,
                    present_set: &[operator],
                    participant: operator,
                    inbound: text,
                    template: PromptTemplateName::Imprint,
                    authority: Authority::Operator,
                },
            )
            .await?;
        Ok(report.outcome)
    }

    /// Run an ad-hoc operator Lua block in a no-commit sandbox (spec §Observability → the operator Lua
    /// console). The block executes against the live graph — reads see real memory — but its buffered
    /// effects, including any `LuaExecuted` record, are discarded, so nothing persists and the run is
    /// invisible to the log. It runs under operator authority on a throwaway VM bound to a dedicated
    /// `console/lua` conversation. MCP is **off** unless `allow_mcp` is set and a host is connected:
    /// an MCP call is a real external effect that no sandbox can roll back, so reaching outward is an
    /// explicit opt-in (e.g. to exercise an input-leaning integration), never the default.
    pub async fn run_lua(
        &self,
        script: &str,
        allow_mcp: bool,
    ) -> Result<LuaConsoleOutcome, ServerError> {
        // The block may embed (`memory.search`) and, with MCP, reach outward, so it takes a stream
        // permit like any model-driving operation (spec §Concurrency), held across the run below.
        let _stream = self
            .server
            .streams
            .acquire()
            .await
            .expect("the stream semaphore is never closed");

        // A dedicated console conversation, minted once (graph before store, per the lock-ordering rule).
        let conversation = {
            let graph = self.server.engine.graph.lock();
            resolve_or_mint_conversation(
                self.server.engine.store.lock().as_mut(),
                self.server.engine.clock.as_ref(),
                &graph,
                &ConversationLocator::new("console", "lua"),
            )?
        };
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

        // A throwaway VM isolated from live sessions; MCP only when opted in and a host is connected.
        let session = match (allow_mcp, self.server.mcp.as_ref()) {
            (true, Some(runtime)) => Session::with_mcp(
                conversation,
                runtime.host.clone(),
                runtime.catalogue.clone(),
            ),
            _ => Session::new(conversation),
        };

        let turn = Settings::from_store(self.server.engine.store.lock().as_ref())?.turn;
        let context = BlockContext {
            teller: Teller::Agent,
            authority: Authority::Operator,
            turn_id: TurnId::generate(),
            block_timeout: Duration::from_secs(turn.block_timeout_seconds.max(0) as u64),
            max_block_attempts: turn.max_block_attempts.max(1) as u32,
            present_set: Vec::new(),
            dry_run: true,
        };

        let outcome = session
            .execute(&self.server.engine, &context, script)
            .await?;
        session.shutdown_mcp().await;

        Ok(match outcome {
            BlockOutcome::Committed { result } => LuaConsoleOutcome {
                result: Some(result),
                error: None,
            },
            BlockOutcome::Terminated(TerminalCause::Error(message)) => LuaConsoleOutcome {
                result: None,
                error: Some(message),
            },
            BlockOutcome::Terminated(TerminalCause::Aborted(message)) => LuaConsoleOutcome {
                result: None,
                error: Some(format!("aborted: {message}")),
            },
        })
    }

    /// The Lua API as the structured catalogue the console renders into a reference guide — the same
    /// build-derived entries projected into the agent's system prompt (spec §What you can do). Static,
    /// so it needs no engine access. MCP tools are excluded; they appear only when actually connected.
    pub fn lua_api(&self) -> Vec<ApiEntry> {
        lua::api_reference()
    }

    /// Register a new version of a prompt template — the operator edit path (spec §Initialization →
    /// prompt templates). Templates are read from the log as the highest version per name, so an
    /// edit is a fresh registration at the next version under operator source; old `produced_by`
    /// references keep resolving to the version they named. No projection — templates are not
    /// materialized into the graph.
    pub fn register_prompt(&self, name: PromptTemplateName, body: &str) -> Result<(), ServerError> {
        let current = templates::latest_template(self.server.engine.store.lock().as_ref(), name)?;
        let version = current.map_or(1, |template| template.version + 1);
        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            vec![EventPayload::prompt_template_registered(
                name,
                version,
                body.to_owned(),
                EventSource::Operator,
            )],
        )?;
        Ok(())
    }

    /// Create the agent — or resume an interrupted genesis — then project the new events so reads
    /// see them. Idempotent: calling it on a born agent is a no-op.
    pub fn create_agent(&self, seed: &SeedSelf) -> Result<Rollout, ServerError> {
        let outcome = genesis::rollout(
            self.server.engine.store.lock().as_mut(),
            self.server.engine.clock.as_ref(),
            seed,
        )?;
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;
        // Baseline the describer cursor past genesis: the seeded `self` has no synthesized description
        // yet, and nothing should try to regenerate it until real content is written (it would have no
        // public entries, and a synchronous caller — a scripted test or the open-time forcing guard —
        // must not block on it). The same baseline `boot` performs, here for the born-without-boot path.
        self.server.baseline_describer_cursor()?;
        self.server.baseline_adjudicator_cursor()?;
        Ok(outcome)
    }

    pub fn genesis_status(&self) -> Result<GenesisStatus, ServerError> {
        Ok(genesis::status(self.server.engine.store.lock().as_ref())?)
    }

    /// Inspect a live memory by name (e.g. `"self"`).
    pub fn memory(&self, name: &str) -> Result<Option<MemoryView>, ServerError> {
        Ok(self
            .server
            .engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))?)
    }

    /// Inspect the live memories in a namespace (e.g. `"person/"`), ordered by name.
    pub fn memories(&self, prefix: &str) -> Result<Vec<MemoryView>, ServerError> {
        Ok(self
            .server
            .engine
            .graph
            .lock()
            .memories_in_namespace(prefix)?)
    }

    /// Inspect the live memories carrying a `Recurring` occurrence — the operator's view of the
    /// agent's recurring calendar, the inspection parallel to the agent-facing `calendar.recurring()`.
    pub fn recurring(&self) -> Result<Vec<MemoryView>, ServerError> {
        Ok(self.server.engine.graph.lock().recurring_memories()?)
    }

    /// The belief arbitrations the agent has recorded, oldest first — for each, the memory it concerns
    /// and the reconciling statement. The audit surface for "why does it believe X" (spec §Write path);
    /// `BeliefArbitrated` is log-only, so this reads it from the log rather than the graph.
    pub fn arbitrations(&self) -> Result<Vec<Arbitration>, ServerError> {
        let mut out = Vec::new();
        let events = self.server.engine.store.lock().read_from(Seq::ZERO)?;
        for event in events {
            if let EventPayload::BeliefArbitrated {
                memory, resolution, ..
            } = event.payload
            {
                let name = self
                    .server
                    .engine
                    .graph
                    .lock()
                    .memory_by_id(memory)?
                    .map(|memory| memory.name)
                    .unwrap_or_else(|| MemoryName::new("<unknown>"));
                out.push(Arbitration {
                    memory: name,
                    statement: resolution.statement,
                });
            }
        }
        Ok(out)
    }

    /// The model interactions recorded on the log, oldest first — each call's request (delta-encoded),
    /// deliberation, token usage, and latency. The console's deliberation surface and the answer to
    /// "where did the turn's time go" (spec §Observability); `ModelCalled` is log-only, so this reads
    /// it from the log. Returns nothing under the `Off` capture level, since no events were written.
    pub fn model_calls(&self) -> Result<Vec<ModelCall>, ServerError> {
        let mut out = Vec::new();
        for event in self.server.engine.store.lock().read_from(Seq::ZERO)? {
            let seq = event.seq;
            let recorded_at = event.recorded_at;
            if let EventPayload::ModelCalled {
                conversation,
                turn_id,
                phase,
                request_digest,
                request,
                completion,
                reasoning,
                finish_reason,
                usage,
                duration_ms,
            } = event.payload
            {
                out.push(ModelCall {
                    seq,
                    recorded_at,
                    conversation,
                    turn_id,
                    phase,
                    request_digest,
                    request,
                    completion,
                    reasoning,
                    finish_reason,
                    usage,
                    duration_ms,
                });
            }
        }
        Ok(out)
    }

    /// The whole event log, oldest first — the raw record everything else is derived from (spec
    /// §Observability → the Events view). The eval harness embeds this per run, and the console
    /// reconstructs its views from it.
    pub fn events(&self) -> Result<Vec<Event>, ServerError> {
        self.events_from(Seq::ZERO)
    }

    /// The event log from `from` onward (every event with `seq >= from`), in order. The live
    /// console's catch-up and tail surface (spec §Observability → live phase): an initial
    /// `events_from(ZERO)` seeds the replica, then repeated `events_from(head)` polls the new tail.
    pub fn events_from(&self, from: Seq) -> Result<Vec<Event>, ServerError> {
        Ok(self.server.engine.store.lock().read_from(from)?)
    }

    /// Inspect a memory's local content entries by name — their text, teller, and visibility — for
    /// auditing what was written and how it is gated (e.g. that a private aside was not stored
    /// `Public`). Empty if the memory is unknown.
    pub fn entries(&self, name: &str) -> Result<Vec<EntryView>, ServerError> {
        let graph = self.server.engine.graph.lock();
        Ok(graph
            .memory_by_name(MemoryName::new(name))?
            .map(|m| graph.entries_local(m.id))
            .transpose()?
            .unwrap_or_default())
    }

    /// The agent's current behavioral settings: the latest `ConfigSet` snapshot.
    pub fn settings(&self) -> Result<Settings, ServerError> {
        Ok(Settings::from_store(
            self.server.engine.store.lock().as_ref(),
        )?)
    }

    /// Replace the agent's behavioral settings, logged as an operator `ConfigSet` (source
    /// `Operator`) — the read-modify-write the configuration design calls for (spec §Initialization →
    /// configuration). The new snapshot is the latest and takes effect on the next read; settings are
    /// read from the log, so no projection is involved.
    pub fn set_settings(&self, settings: Settings) -> Result<(), ServerError> {
        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            vec![EventPayload::config_set(settings, EventSource::Operator)],
        )?;
        Ok(())
    }

    /// The sessions of a conversation, addressed by its locator, oldest first — operator inspection
    /// of how the conversation segmented into sessions. Empty if the room has never been seen.
    pub fn sessions(&self, locator: &ConversationLocator) -> Result<Vec<SessionView>, ServerError> {
        let graph = self.server.engine.graph.lock();
        match graph.conversation_for_locator(locator)? {
            Some(conversation) => Ok(graph.sessions_in(conversation)?),
            None => Ok(Vec::new()),
        }
    }
}
