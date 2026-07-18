//! Control actions: agent creation, the imprint interview, Lua console runs, and prompt registration.

use std::time::Duration;

use crate::{
    agent::{
        BlockContext, InboundMessage, TurnOutcome,
        api_doc::ApiEntry,
        genesis::{self, Rollout, SeedSelf},
        lua::{self, BlockOutcome, Session},
        templates,
    },
    event::{
        EventPayload, EventSource, LinkPosture, LinkSource, PromptTemplateName, Teller,
        TerminalCause, Visibility,
    },
    ids::{ConversationLocator, EntryId, MemoryId, MemoryName, TurnId},
    instance::{
        InstanceError,
        control::{
            Control, DesignateOutcome, LuaConsoleOutcome, RetractOutcome, SelfEditOutcome,
            UnmergeOutcome,
        },
        session::RoutedTurn,
    },
    memory::{
        identity,
        memory_block::{AppendOptions, Authority, MemoryBlock, MemoryError, VisibilityChoice},
    },
    model::ModelClient,
    settings::Settings,
    vocabulary::RelationName,
};
use zuihitsu_platform_connector_types::PlatformResponse;

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
    ) -> Result<PlatformResponse, InstanceError> {
        let operator = self.server.resolve_or_mint_operator()?;
        let conversation = {
            // Graph before store, per the lock-ordering rule (this resolve holds both at once).
            let graph = self.server.engine.graph.lock();
            identity::resolve_or_mint_conversation(
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

        // Serialize and supersede the imprint's own conversation, like a platform batch (spec
        // §Concurrency → per-conversation supersession): two rapid imprints serialize and the newer
        // supersedes the in-flight one instead of racing the shared VM. The window is read per call
        // from the same store-backed setting, before `arrive`, so a failed settings read never leaves
        // an arrival that bumped the supersede watch without ever admitting.
        let window_seconds = Settings::from_store(self.server.engine.store.lock().as_ref())?
            .turn
            .supersede_window_seconds;
        let window = Duration::from_secs(window_seconds.max(0) as u64);
        let ticket = self
            .server
            .turns
            .arrive(conversation, self.server.engine.clock.now());
        let mut admission = ticket.admit(window).await;

        // The imprint runs the model too, so it takes a stream permit like any other turn (spec
        // §Concurrency), held across the interview turn below and taken only after slot admission so a
        // waiting imprint never camps on a permit.
        let _stream = self
            .server
            .streams
            .acquire()
            .await
            .expect("the stream semaphore is never closed");
        // Build the inbound batch and turn ids; the participant turn is recorded inside
        // `run_session_turn` after the session opens (so the session's `start_seq` precedes it).
        let participant_turn_id = TurnId::generate();
        let inbound = vec![InboundMessage {
            participant: operator,
            text: text.to_owned(),
        }];
        let participant_turn_ids = vec![participant_turn_id];
        let (report, _buffer) = self
            .server
            .run_session_turn(
                model,
                &RoutedTurn {
                    conversation,
                    present_set: &[operator],
                    inbound: &inbound,
                    participant_turn_ids: &participant_turn_ids,
                    template: PromptTemplateName::Imprint,
                    authority: Authority::Operator,
                },
                Some(admission.supersession()),
            )
            .await?;
        // A superseded imprint lost its slot to a newer one: leave its arrival anchoring the burst for
        // the successor rather than pruning it, matching the platform path.
        if matches!(report.outcome, TurnOutcome::Superseded) {
            admission.mark_superseded();
        }
        Ok(PlatformResponse {
            outcome: report.outcome,
            participant_turn_ids: report
                .participant_turn_ids
                .iter()
                .map(|id| id.0.to_string())
                .collect(),
        })
    }

    /// Run an ad-hoc operator Lua block in a no-commit sandbox (spec §Observability → the operator Lua
    /// console). The block executes against the live graph — reads see real memory — but its buffered
    /// effects, including any `LuaExecuted` record, are discarded, so nothing persists and the run is
    /// invisible to the log. It runs under operator authority on a throwaway VM bound to a dedicated
    /// `console/lua` conversation. Outward reach is **off** by default: `allow_mcp` opts into MCP calls
    /// (when a host is connected) and `allow_web` into `web.markdown` (when a fetcher is connected).
    /// Both perform real external I/O that no sandbox can roll back, even though the block's memory
    /// writes are discarded, so each is an explicit opt-in rather than the default.
    pub async fn run_lua(
        &self,
        script: &str,
        allow_mcp: bool,
        allow_web: bool,
    ) -> Result<LuaConsoleOutcome, InstanceError> {
        // The block may embed (`memory.search`) and, with MCP, reach outward, so it takes a stream
        // permit like any model-driving operation (spec §Concurrency), held across the run below.
        let _stream = self
            .server
            .streams
            .acquire()
            .await
            .expect("the stream semaphore is never closed");

        // Bring the graph to head so the block reads current state, then run against no conversation:
        // the console is a throwaway sandbox with no room, so it neither resolves nor mints one (which
        // would persist a `console/lua` conversation and context memory), keeping the run invisible to
        // the log. Writes are discarded anyway, and `context.current` is nil — matching the other
        // conversation-less operator paths (self-edit, retraction).
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

        // A throwaway VM isolated from live sessions; each outward projection installed only when opted
        // in and its dependency is connected. `with_web(None)` is a no-op, so a missing fetcher simply
        // leaves `web` absent even with the opt-in set.
        let base = match (allow_mcp, self.server.mcp.as_ref()) {
            (true, Some(runtime)) => Session::with_mcp(
                None,
                runtime.host.clone(),
                runtime.catalogue.clone(),
                self.server.features,
            ),
            _ => Session::new(None, self.server.features),
        };
        let session = if allow_web {
            base.with_web(self.server.web.clone())
        } else {
            base
        };

        let settings = Settings::from_store(self.server.engine.store.lock().as_ref())?;
        let turn = settings.turn;
        let context = BlockContext {
            teller: Teller::Agent,
            authority: Authority::Operator,
            turn_id: TurnId::generate(),
            block_timeout: Duration::from_secs(turn.block_timeout_seconds.max(0) as u64),
            max_block_attempts: turn.max_block_attempts.max(1) as u32,
            max_entry_chars: settings.memory.max_entry_chars.max(1) as usize,
            present_set: Vec::new(),
            dry_run: true,
        };

        let outcome = session
            .execute(&self.server.engine, &context, script)
            .await
            .map_err(|error| InstanceError::Lua {
                conversation: None,
                error,
            })?;
        session.shutdown_mcp().await;

        Ok(match outcome {
            BlockOutcome::Committed { result } => LuaConsoleOutcome {
                result: Some(result),
                error: None,
            },
            BlockOutcome::Skipped(reason) => LuaConsoleOutcome {
                result: None,
                error: Some(format!("turn skipped: {}", reason.unwrap_or_default())),
            },
            BlockOutcome::Terminated(TerminalCause::Error(message)) => LuaConsoleOutcome {
                result: None,
                error: Some(message),
            },
            BlockOutcome::Terminated(TerminalCause::Aborted(message)) => LuaConsoleOutcome {
                result: None,
                error: Some(format!("aborted: {message}")),
            },
            BlockOutcome::Terminated(TerminalCause::Skipped(reason)) => LuaConsoleOutcome {
                result: None,
                error: Some(format!("turn skipped: {}", reason.unwrap_or_default())),
            },
        })
    }

    /// The Lua API as the structured catalogue the console renders into a reference guide — the same
    /// entries projected into the agent's system prompt (spec §What you can do). The hand-written API
    /// is build-derived and needs no engine access; the MCP tools are appended from the connected host's
    /// probed catalogue, so the reference matches what a turn's `full_api_reference` shows rather than
    /// omitting the servers. Each MCP entry carries its `allow_mcp` gate for the console to mark.
    pub fn lua_api(&self) -> Vec<ApiEntry> {
        let mut entries = lua::api_reference(&self.server.features);
        if let Some(runtime) = self.server.mcp.as_ref() {
            entries.extend(runtime.catalogue.api_entries());
        }
        entries
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
            EventSource::Operator,
            vec![EventPayload::prompt_template_registered(
                name,
                version,
                body.to_owned(),
            )],
        )?;
        Ok(())
    }

    /// Append to — or revise an entry on — `self`, the agent's own profile, from the console under
    /// operator authority (spec §Imprint interview → the operator owns `self`). The direct counterpart
    /// to [`Control::imprint`]: where the imprint writes `self` by running the model conversationally,
    /// this writes it outright, the same operator authority that lets the console edit every other
    /// prompt-shaping surface — the scaffold via [`Control::register_prompt`], the behaviour via
    /// [`Control::set_settings`]. (Distinct from the agent writing its *own* `self`, which stays barred
    /// from a conversation.)
    ///
    /// The write runs through a real operator-authority [`MemoryBlock`], so `guard_self` passes for the
    /// operator exactly as a platform turn's is rejected — the guard is honoured, not weakened — and the
    /// edit reuses the block's length validation, visibility resolution, and (for a revision) its
    /// live-entry checks rather than authoring raw events. A `self` entry is charter content: authored
    /// in the agent's own voice (`Teller::Agent`) and `Public`, so it feeds the system prompt's identity
    /// verbatim and the describer's regeneration. With `supersedes` set the write is a revision — the new
    /// entry replaces the named one, which drops from every live surface while remaining in history; with
    /// none, it is a plain append.
    ///
    /// No description rerun is triggered here, and none is needed: the append advances `self`'s content
    /// watermark, so the background describer regenerates its description on its next pass, and the
    /// identity the system prompt reads is drawn from the entries verbatim regardless of the description.
    /// The operator-input cases (an empty edit, an unknown `supersedes` id, over-long text, or an agent
    /// not yet born) return as [`SelfEditOutcome`] variants the console renders; only a genuinely
    /// unexpected block failure escalates to [`InstanceError`].
    /// Edit the agent's `self` profile under operator authority (the console counterpart to the
    /// imprint interview, and the operator side of self-editing). Appends a charter entry, or revises
    /// one when `supersedes` names a live entry. The edit's provenance is carried by a dedicated
    /// `console/self` conversation — minted atomically with the entry write, so a failed edit (an
    /// empty text, an unknown `supersedes` id, over-long text, or an unborn agent) leaves no orphaned
    /// context memory behind.
    pub fn edit_self(
        &self,
        text: &str,
        supersedes: Option<EntryId>,
    ) -> Result<SelfEditOutcome, InstanceError> {
        if text.trim().is_empty() {
            return Ok(SelfEditOutcome::EmptyText);
        }
        let self_id = {
            let graph = self.server.engine.graph.lock();
            match graph.self_memory()?.map(|memory| memory.id) {
                Some(id) => id,
                None => return Ok(SelfEditOutcome::NotBorn),
            }
        };

        // Operator self-edits are not bound by `max_entry_chars` — the limit guards against the agent
        // pasting source content into memory, not against the operator authoring a persona. Genesis
        // writes directly to the store and is likewise unbounded, so a self-edit revising a genesis
        // persona entry can replace it with text of comparable length. No conversation is attributed
        // — provenance is carried by `EventSource::Operator` and `Authority::Operator`.
        let mut block = MemoryBlock::new(
            self.server.engine.clone(),
            Teller::Agent,
            Authority::Operator,
            None,
            None,
            Vec::new(),
            usize::MAX,
        )?;
        // Charter content is `Public` — the identity surface the system prompt reads verbatim and
        // the describer regenerates its description from.
        let opts = AppendOptions {
            visibility: Some(VisibilityChoice::Public),
            ..AppendOptions::default()
        };
        let entry_id = match supersedes {
            Some(old) => block.revise(self_id, old, text, opts),
            None => block.append(self_id, text, opts),
        };
        let entry_id = match entry_id {
            Ok(entry_id) => entry_id,
            Err(MemoryError::UnknownEntry(_)) => {
                // Only `revise`'s supersede leg raises this, and it is driven by `supersedes`, so the
                // unknown entry is exactly that operator-supplied id.
                return Ok(SelfEditOutcome::UnknownEntry(supersedes.expect(
                    "UnknownEntry arises only from a revise, which passes an id",
                )));
            }
            Err(MemoryError::ContentTooLong { length, limit }) => {
                return Ok(SelfEditOutcome::TooLong { length, limit });
            }
            Err(error) => return Err(InstanceError::Memory(error)),
        };

        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            EventSource::Operator,
            block.into_effects().events,
        )?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = self.server.engine.graph.lock();
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(SelfEditOutcome::Applied(entry_id))
    }

    /// Retract a live entry from any memory under operator authority (spec §Visibility → the operator
    /// withdraws a fact outright). The entry is tombstoned — it drops from every live surface while
    /// remaining in history with its reason. The operator supplies the reason; an empty reason is
    /// rejected, because an unexplained retraction is unauditable. Like `edit_self`, the conversation
    /// that carries the retraction's provenance is minted atomically with the write.
    pub fn retract_entry(
        &self,
        memory: &str,
        entry: EntryId,
        reason: &str,
    ) -> Result<RetractOutcome, InstanceError> {
        if reason.trim().is_empty() {
            return Ok(RetractOutcome::EmptyReason);
        }
        let memory_id = {
            let graph = self.server.engine.graph.lock();
            match graph
                .memory_by_name(MemoryName::new(memory))?
                .map(|memory| memory.id)
            {
                Some(id) => id,
                None => return Ok(RetractOutcome::UnknownMemory),
            }
        };

        // Retraction carries no conversation — provenance is `EventSource::Operator`. The entry
        // limit is irrelevant (retraction buffers no content), but the block requires a value.
        let max_entry_chars = Settings::from_store(self.server.engine.store.lock().as_ref())?
            .memory
            .max_entry_chars
            .max(1) as usize;
        let mut block = MemoryBlock::new(
            self.server.engine.clone(),
            Teller::Agent,
            Authority::Operator,
            None,
            None,
            Vec::new(),
            max_entry_chars,
        )?;
        match block.retract(memory_id, entry, reason) {
            Ok(()) => {}
            Err(MemoryError::UnknownEntry(_)) => return Ok(RetractOutcome::UnknownEntry(entry)),
            Err(MemoryError::RetractionReasonRequired) => return Ok(RetractOutcome::EmptyReason),
            Err(error) => return Err(InstanceError::Memory(error)),
        }

        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            EventSource::Operator,
            block.into_effects().events,
        )?;
        let mut graph = self.server.engine.graph.lock();
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(RetractOutcome::Retracted)
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
        // Baseline the link-inference cursor past genesis so a synchronous caller does not re-run that
        // pass over the seeded state. The describer needs no baseline call: the `GenesisCompleted`
        // handler already marked the seeded `self` described in the graph materialization above, so the
        // first describe pass over it regenerates nothing.
        self.server.baseline_link_inference_cursor()?;
        Ok(outcome)
    }

    /// Confirm a pending cross-platform merge proposal as the operator would from the console (spec
    /// §Cross-platform identity → operator-asserted merge): author the merging `same_as` link directly
    /// (`LinkSource::Operator`) — the console-only path to a merge, the same operator authority that
    /// lets the console assert identity the agent's own `links.create(a, "same_as", b)` may not — and
    /// re-materialize so a subsequent read reflects the merge. There is no decline counterpart: a
    /// proposal has no recorded settlement of its own, so an unconvinced operator simply leaves it
    /// pending, and nothing merges until they confirm.
    pub fn confirm_merge(&self, from: MemoryId, to: MemoryId) -> Result<(), InstanceError> {
        let now = self.server.engine.clock.now();
        let event = EventPayload::link_created(
            from,
            to,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                // No teller behind it: the operator authored this from the console, not a participant.
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        );
        self.server
            .engine
            .store
            .lock()
            .append(now, EventSource::Operator, vec![event])?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = self.server.engine.graph.lock();
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(())
    }

    /// Retract an operator-asserted `same_as` merge, splitting the two identities back into their own
    /// visibility classes (spec §Cross-platform identity → operator-asserted merge). The console-only
    /// undo of a wrong merge and the mirror of `confirm_merge`: it authors a `LinkRemoved` on
    /// the `same_as` edge between the two stubs — the operator authority the agent's own turn is denied,
    /// since a `same_as` retraction is operator-only (see `change_link`) — then re-materializes so the
    /// classes split on the next read. Only a *direct* edge between exactly this pair is retractable: an
    /// id that names no live memory, or a pair joined only transitively through a third member, is
    /// refused (`UnknownMemory`/`NotMerged`) rather than authoring an event that would delete nothing.
    /// The removal carries no provenance of its own — `LinkRemoved` records no source — so the operator's
    /// authorship lives in this control path, not on the event.
    pub fn unmerge(&self, from: MemoryId, to: MemoryId) -> Result<UnmergeOutcome, InstanceError> {
        {
            let graph = self.server.engine.graph.lock();
            if graph.memory_by_id(from)?.is_none() {
                return Ok(UnmergeOutcome::UnknownMemory(from));
            }
            if graph.memory_by_id(to)?.is_none() {
                return Ok(UnmergeOutcome::UnknownMemory(to));
            }
            // `links(from)` returns every canonical edge touching `from`, so a `same_as` edge whose other
            // endpoint is `to` is the direct merge — regardless of which way the canonical pair is stored.
            let directly_merged = graph.links(from)?.iter().any(|link| {
                link.relation == RelationName::SameAs && (link.from == to || link.to == to)
            });
            if !directly_merged {
                return Ok(UnmergeOutcome::NotMerged);
            }
        }
        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            EventSource::Operator,
            vec![EventPayload::link_removed(from, to, RelationName::SameAs)],
        )?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = self.server.engine.graph.lock();
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(UnmergeOutcome::Removed)
    }

    /// Designate (or release) a `same_as` class's primary stub — the id class-level facts and reads
    /// resolve through (spec §Cross-platform identity). Without a designation the primary is the class's
    /// earliest member by ULID, so a throwaway stub minted before the real handle wins by age; pinning
    /// the operator's canonical stub overrides that. `designated` is `true` to pin and `false` to
    /// release back to the earliest-ULID rule. The choice is recorded as a `ClassPrimaryDesignated` on
    /// the memory, so it persists on the log and survives the stub's later unmerge into another class.
    /// An id that names no live memory is refused (`UnknownMemory`) rather than authoring an inert event.
    pub fn designate_primary(
        &self,
        memory: MemoryId,
        designated: bool,
    ) -> Result<DesignateOutcome, InstanceError> {
        {
            let graph = self.server.engine.graph.lock();
            if graph.memory_by_id(memory)?.is_none() {
                return Ok(DesignateOutcome::UnknownMemory(memory));
            }
        }
        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            EventSource::Operator,
            vec![EventPayload::class_primary_designated(memory, designated)],
        )?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = self.server.engine.graph.lock();
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(DesignateOutcome::Designated)
    }
}
