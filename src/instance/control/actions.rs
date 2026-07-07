//! Control actions: agent creation, the imprint interview, Lua console runs, and prompt registration.

use std::time::Duration;

use crate::{
    agent::{
        BlockContext, TurnOutcome,
        api_doc::ApiEntry,
        genesis::{self, Rollout, SeedSelf},
        lua::{self, BlockOutcome, Session},
        templates,
    },
    event::{EventPayload, EventSource, LinkSource, PromptTemplateName, Teller, TerminalCause},
    ids::{ConversationLocator, MemoryId, TurnId},
    memory::{identity::resolve_or_mint_conversation, memory_block::Authority},
    model::ModelClient,
    settings::Settings,
    vocabulary::RelationName,
};

use super::super::InstanceError;
use crate::instance::session::RoutedTurn;
use super::LuaConsoleOutcome;

impl super::Control<'_> {
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
    ) -> Result<TurnOutcome, InstanceError> {
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
    ) -> Result<LuaConsoleOutcome, InstanceError> {
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
                self.server.features,
            ),
            _ => Session::new(conversation, self.server.features),
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
            .await
            .map_err(|error| InstanceError::Lua {
                conversation: Some(conversation),
                error,
            })?;
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
        lua::api_reference(&self.server.features)
    }

    /// Register a new version of a prompt template — the operator edit path (spec §Initialization →
    /// prompt templates). Templates are read from the log as the highest version per name, so an
    /// edit is a fresh registration at the next version under operator source; old `produced_by`
    /// references keep resolving to the version they named. No projection — templates are not
    /// materialized into the graph.
    pub fn register_prompt(
        &self,
        name: PromptTemplateName,
        body: &str,
    ) -> Result<(), InstanceError> {
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
    pub fn create_agent(&self, seed: &SeedSelf) -> Result<Rollout, InstanceError> {
        let outcome = genesis::rollout(
            self.server.engine.store.lock().as_mut(),
            self.server.engine.clock.as_ref(),
            seed,
            self.server.model_context_length,
            &self.server.features,
        )?;
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;
        // Baseline the adjudicator and link-inference cursors past genesis so a synchronous caller does
        // not re-run those passes over the seeded state. The describer needs no baseline call: the
        // `GenesisCompleted` handler already marked the seeded `self` described in the graph
        // materialization above, so the first describe pass over it regenerates nothing.
        self.server.baseline_adjudicator_cursor()?;
        self.server.baseline_link_inference_cursor()?;
        Ok(outcome)
    }

    /// Resolve a pending cross-platform merge proposal as the operator would from the console (spec
    /// §Cross-platform identity → operator-asserted merge). On `accept`, author the merging `same_as`
    /// link directly (`LinkSource::Operator`) — the console-only path to a merge that does not run
    /// through the adjudicator, the same operator authority that lets the console assert identity the
    /// agent's own `mem:link("same_as")` may not. On refusal, record a `MergeAdjudicated` decline (no
    /// `produced_by` — the operator decided, not a model) so the proposal reads as settled and the
    /// adjudicator does not weigh it again. Either way the graph is re-materialized so a subsequent read
    /// reflects the decision.
    pub fn resolve_merge(
        &self,
        from: MemoryId,
        to: MemoryId,
        accept: bool,
    ) -> Result<(), InstanceError> {
        let now = self.server.engine.clock.now();
        let event = if accept {
            EventPayload::LinkCreated {
                from,
                to,
                relation: RelationName::SameAs,
                source: LinkSource::Operator,
                // No teller behind it: the operator authored this from the console, not a participant.
                told_by: None,
            }
        } else {
            EventPayload::MergeAdjudicated {
                from,
                to,
                accepted: false,
                rationale: "declined by the operator".to_owned(),
                produced_by: None,
            }
        };
        self.server.engine.store.lock().append(now, vec![event])?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = self.server.engine.graph.lock();
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(())
    }
}
