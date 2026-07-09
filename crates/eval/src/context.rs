//! The run context — a fresh, booted agent per run, with the helpers a scenario drives it through
//! (route a turn, advance the clock, catch the index up) and the run's event log afterwards. Each run
//! is independent (its own in-memory store, graph, and — when retrieval is configured — vector index),
//! which is what lets runs parallelize.

use std::{collections::BTreeMap, sync::Arc, time::Instant};

use zuihitsu::{
    ConversationLocator, Embedder, Event, EventPayload, FakeMcpHost, Graph, InstanceFeatures,
    LinkSource, ManualClock, McpServerConfig, MemoryId, MemoryStore, ModelClient, RelationName,
    SeedSelf, Seq, Server, SqliteVectorIndex, Store, Timestamp, TurnOutcome, Visibility,
};

use crate::error::EvalError;

/// The fixed clock anchor every run starts at (2026-06-08T00:00:00Z), so scenario timing is
/// reproducible; scenarios advance from here.
pub(crate) const RUN_START_MS: i64 = 1_780_876_800_000;

/// The shared day/hour units every scenario expresses its clock advances and windows in, re-exported
/// from core so the derivation lives in one place rather than being redefined per scenario module.
pub(crate) use zuihitsu::time::{MILLIS_PER_DAY, MILLIS_PER_HOUR};

/// A human's pause before sending a message — applied before each inbound turn so consecutive turns in
/// a busy room are spaced apart, not stacked at one instant. Small against the day-scale advances a
/// scheduling scenario makes, so it does not perturb those.
const HUMAN_PAUSE_MS: i64 = 10_000;

/// Just past the default idle gap (1800s), so the next turn after an [`RunContext::advance`] of this
/// much opens a fresh session. Shared by the scenarios that cross the idle seam without a day-scale
/// advance (an operator imprint lapsing, a room going quiet between sessions).
pub(crate) const PAST_IDLE_GAP_MS: i64 = 1_801 * 1_000;

/// The shared, build-once inputs every run needs: the model, — when an embedding endpoint is
/// configured — the embedder and its dimensionality (a fresh vector index is built per run), and —
/// when a test MCP host is configured — the fake server catalogue a scenario's `needs_mcp()` run
/// depends on (a fresh host is connected per run, returning canned results deterministically).
#[derive(Clone)]
pub struct RunDeps {
    pub model: Arc<dyn ModelClient>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub dimensions: usize,
    pub mcp: Option<Arc<FakeMcpHost>>,
}

/// One run's booted agent and the clock it runs against.
pub struct RunContext {
    server: Server,
    model: Arc<dyn ModelClient>,
    clock: ManualClock,
}

impl RunContext {
    /// Build, boot, and birth a fresh agent for one run, with the scenario's feature set narrowing the
    /// agent's API surface (so a scenario like `InfersLinkFromContent` can disable `linking` and test
    /// the inference pass as the sole path to a link).
    pub async fn new(deps: &RunDeps, features: InstanceFeatures) -> Result<RunContext, EvalError> {
        let clock = ManualClock::new(Timestamp::from_millis(RUN_START_MS));
        let server = assemble(deps, features, &clock, Box::new(MemoryStore::new())).await?;
        // A fresh run is born: genesis writes the birth events into the empty log.
        server.control().create_agent(&seed())?;
        Ok(RunContext {
            server,
            model: deps.model.clone(),
            clock,
        })
    }

    /// Restore a recorded run's log verbatim and boot the agent around it, without birthing a new one —
    /// the resume path's counterpart to [`RunContext::new`]. The `events` (a recorded run's log up to a
    /// chosen step's watermark, genesis included) are appended to a fresh [`MemoryStore`] preserving
    /// each event's `recorded_at`, so the seqs regenerate `1..=N` in their recorded order and the
    /// materialized graph rebuilds at exactly the point the recording held. The clock starts at the last
    /// restored event's `recorded_at`, so the continuation's timestamps continue the recorded timeline
    /// rather than resetting to [`RUN_START_MS`]. Genesis already sits in the restored log, so this
    /// boots into an existing log — the deployment restart path — rather than creating an agent.
    pub async fn restored(
        deps: &RunDeps,
        features: InstanceFeatures,
        events: &[Event],
    ) -> Result<RunContext, EvalError> {
        let mut store = MemoryStore::new();
        restore_verbatim(&mut store, events)?;
        // The restored head must equal the recording's watermark: the seqs regenerated `1..=N` in order,
        // so a mismatch means the append reordered or dropped events — a restore that cannot be trusted.
        let restored_head = store.head().map_err(server_error)?;
        let expected_head = events.last().map(|event| event.seq).unwrap_or(Seq::ZERO);
        if restored_head != expected_head {
            return Err(EvalError::Replay(format!(
                "restored log head is seq {} but the recorded watermark is seq {}",
                restored_head.0, expected_head.0
            )));
        }
        // The continuation's clock continues the recorded timeline from the last restored event.
        let last_ms = events
            .last()
            .map(|event| event.recorded_at.as_millis())
            .unwrap_or(RUN_START_MS);
        let clock = ManualClock::new(Timestamp::from_millis(last_ms));
        let server = assemble(deps, features, &clock, Box::new(store)).await?;
        Ok(RunContext {
            server,
            model: deps.model.clone(),
            clock,
        })
    }

    /// Route one inbound message and run the agent's turn, returning what it said. Advances the run
    /// clock so turns sit on a realistic timescale: a human pause before the message, then the agent's
    /// actual think time after — so the recorded timestamps reflect how the conversation paced (legible
    /// especially in the multi-party rooms), rather than stacking every turn at one frozen instant. The
    /// executor resolves the step's `present` set and text before calling.
    pub(crate) async fn turn(
        &self,
        platform: &str,
        scope: &str,
        sender: &str,
        text: &str,
        present: &[&str],
    ) -> Result<TurnOutcome, EvalError> {
        self.clock.advance_millis(HUMAN_PAUSE_MS);
        let locator = ConversationLocator::new(platform, scope);
        let started = Instant::now();
        let outcome = self
            .server
            .platform()
            .route_message(self.model.as_ref(), &locator, sender, text, present)
            .await?;
        self.clock
            .advance_millis(started.elapsed().as_millis() as i64);
        Ok(outcome)
    }

    /// Drive one operator imprint-interview turn — the `operator/imprint` channel, under operator
    /// authority (the only path that may write `self`), distinct from the participant turns `turn`
    /// drives. Paces the clock like `turn`, so an `advance` between imprint turns crosses the idle gap
    /// into a fresh session just as it does for participants.
    pub(crate) async fn imprint(&self, text: &str) -> Result<TurnOutcome, EvalError> {
        self.clock.advance_millis(HUMAN_PAUSE_MS);
        let started = Instant::now();
        let outcome = self
            .server
            .control()
            .imprint(self.model.as_ref(), text)
            .await?;
        self.clock
            .advance_millis(started.elapsed().as_millis() as i64);
        Ok(outcome)
    }

    /// Append raw events to the store and materialize the graph, for scenarios that set up
    /// deterministic state directly — no agent or Lua in the loop. The caller constructs the exact
    /// events, so a scenario controls precisely what state exists.
    pub(crate) fn seed_events(&self, events: Vec<EventPayload>) -> Result<(), EvalError> {
        self.server.control().seed_events(events)?;
        Ok(())
    }

    /// Confirm a cross-platform merge as the operator would from the console (spec §Cross-platform
    /// identity → operator-asserted merge): author the `same_as` link between two `person/*` stubs
    /// directly, the one path to a merge that does not run through the adjudicator. Drives the operator
    /// confirmation a proposal surfaces for, so a scenario can assess what the agent does once identity
    /// is confirmed.
    pub(crate) fn operator_merge(&self, from: MemoryId, to: MemoryId) -> Result<(), EvalError> {
        self.seed_events(vec![EventPayload::link_created(
            from,
            to,
            RelationName::SameAs,
            LinkSource::Operator,
            None,
            None,
            Visibility::Public,
        )])
    }

    /// Advance the run's clock by `delta_ms` — to cross a recurrence instance or an idle gap.
    pub(crate) fn advance(&self, delta_ms: i64) {
        self.clock.advance_millis(delta_ms);
    }

    /// Tighten the compaction trigger so a short scripted session crosses the token budget and flushes
    /// before the cut (the fixture-22/23 setup). Sets the budget and the flush floor, leaving the rest
    /// of the settings as seeded.
    pub(crate) fn tighten_compaction(
        &self,
        token_budget: i64,
        flush_min_turns: i64,
    ) -> Result<(), EvalError> {
        let mut settings = self.server.control().settings()?;
        settings.compaction.token_budget = token_budget;
        settings.compaction.flush_min_turns = flush_min_turns;
        self.server.control().set_settings(settings)?;
        Ok(())
    }

    /// Force a compaction of the open session in `platform`/`scope` right now, through the same path
    /// the organic token-budget trigger drives (the pre-compaction flush, the carryover staging, and a
    /// fresh session seeded from that carryover on the next turn). This states the cut point directly,
    /// so a scenario probing survival across several seams forces its cuts rather than sizing a token
    /// budget so the trigger *happens* to fire the right number of times. Returns whether a live
    /// session was actually compacted.
    pub(crate) async fn force_compaction(
        &self,
        platform: &str,
        scope: &str,
    ) -> Result<bool, EvalError> {
        let locator = ConversationLocator::new(platform, scope);
        Ok(self
            .server
            .platform()
            .force_compaction(self.model.as_ref(), &locator)
            .await?)
    }

    /// Tune the checkpoint gates so a scripted two-room exchange trips them: the substance threshold
    /// and the cooldown, leaving the enable flag and the rest of the settings as seeded.
    pub(crate) fn tune_checkpoint(
        &self,
        min_delta_chars: i64,
        cooldown_seconds: i64,
    ) -> Result<(), EvalError> {
        let mut settings = self.server.control().settings()?;
        settings.checkpoint.min_delta_chars = min_delta_chars;
        settings.checkpoint.cooldown_seconds = cooldown_seconds;
        self.server.control().set_settings(settings)?;
        Ok(())
    }

    /// Run one checkpoint sweep over the live sessions — the mid-session flush the background
    /// checkpoint sweeper drives on a timer (spec §Compaction → checkpoint flush), driven explicitly
    /// so a scenario controls exactly where the flush lands between turns. Returns how many sessions
    /// flushed.
    pub(crate) async fn checkpoint_sweep(&self) -> Result<usize, EvalError> {
        Ok(self
            .server
            .checkpoint_live_sessions(self.model.as_ref())
            .await?)
    }

    /// Catch the vector index up to the log, so a fact written this run is searchable next turn (the
    /// same catch-up the background indexer runs).
    pub(crate) async fn index_catch_up(&self) -> Result<(), EvalError> {
        self.server.index_catch_up().await?;
        Ok(())
    }

    /// Let both background synthesis passes settle: the describer (descriptions, arbitration, temporal
    /// extraction) and then the vector indexer, in that order. This is the pair scenarios run together
    /// after a turn that wrote content — before advancing the clock across a gap or asserting on the
    /// synthesized-and-searchable state — folded into one call. A scenario that needs only one of the
    /// two (no retrieval, or a description-only probe) calls the specific catch-up directly.
    pub(crate) async fn settle(&self) -> Result<(), EvalError> {
        self.describe_catch_up().await?;
        self.index_catch_up().await?;
        Ok(())
    }

    /// Regenerate descriptions, belief arbitration, and temporal extraction for everything written so
    /// far — the off-hot-path synthesis the background describer runs, driven explicitly (spec §Write
    /// path). A scenario that asserts on a synthesized description, an arbitration, or a resolved
    /// occurrence calls this after the turn that wrote it, before its log is assessed.
    pub(crate) async fn describe_catch_up(&self) -> Result<(), EvalError> {
        self.server.describe_catch_up(self.model.as_ref()).await?;
        Ok(())
    }

    /// Adjudicate the merges proposed so far — the off-hot-path pass the background adjudicator runs,
    /// driven explicitly (spec §Cross-platform identity → adjudicated merge). A scenario that proposes a
    /// merge calls this before its log is assessed, so the verdict (and any `same_as`) is recorded.
    pub(crate) async fn adjudicate_catch_up(&self) -> Result<(), EvalError> {
        self.server.adjudicate_catch_up(self.model.as_ref()).await?;
        Ok(())
    }

    /// Infer links from the content written so far — the off-hot-path pass the background
    /// link-inference worker runs, driven explicitly (spec §Write path → link inference). A scenario
    /// that asserts on an inferred link calls this after the turn that wrote the content, before its
    /// log is assessed.
    pub(crate) async fn link_inference_catch_up(&self) -> Result<(), EvalError> {
        self.server
            .link_inference_catch_up(self.model.as_ref())
            .await?;
        Ok(())
    }

    /// The run's whole event log — the record the harness embeds and assessment reads.
    pub(crate) fn events(&self) -> Result<Vec<Event>, EvalError> {
        Ok(self.server.control().events()?)
    }

    /// The run's events recorded at or after `from` — for streaming a run's deliberation live as it
    /// drives, reading only what is new since the last poll.
    pub(crate) fn events_from(&self, from: Seq) -> Result<Vec<Event>, EvalError> {
        Ok(self.server.control().events_from(from)?)
    }
}

/// Build a server around `store` and `clock` with the scenario's feature set, connect the test MCP host
/// (if any) before boot so sessions opened during the run get the projected tool catalogue, and boot.
/// Shared by [`RunContext::new`] (which then births a fresh agent) and [`RunContext::restored`] (which
/// boots into a restored log): boot materializes the graph from whatever the store already holds, so it
/// serves both the empty-log birth and the existing-log restart.
async fn assemble(
    deps: &RunDeps,
    features: InstanceFeatures,
    clock: &ManualClock,
    store: Box<dyn Store>,
) -> Result<Server, EvalError> {
    let mut server = match &deps.embedder {
        Some(embedder) => Server::with_retrieval_features(
            store,
            Graph::open_in_memory()?,
            Box::new(clock.clone()),
            embedder.clone(),
            Box::new(SqliteVectorIndex::open_in_memory(deps.dimensions)?),
            features,
        ),
        None => Server::with_features(
            store,
            Graph::open_in_memory()?,
            Box::new(clock.clone()),
            features,
        ),
    };
    // The `FakeMcpHost` is pure in-memory — no subprocess, no network — so it is safe to always connect.
    // Real MCP servers (`config.mcp`) never reach the eval.
    if let Some(host) = &deps.mcp {
        let configs: BTreeMap<String, McpServerConfig> =
            BTreeMap::from([("fetch".to_owned(), McpServerConfig::default())]);
        server.connect_mcp(host.clone(), configs).await?;
    }
    server.boot()?;
    Ok(server)
}

/// Append `events` to a fresh store preserving each event's `recorded_at`. Consecutive events sharing a
/// timestamp ride in one batch (the store stamps a batch with a single `recorded_at`), so the seqs
/// regenerate `1..=N` in the recorded order while every event keeps its original recorded time.
fn restore_verbatim(store: &mut MemoryStore, events: &[Event]) -> Result<(), EvalError> {
    let mut index = 0;
    while index < events.len() {
        let recorded_at = events[index].recorded_at;
        let mut batch = Vec::new();
        while index < events.len() && events[index].recorded_at == recorded_at {
            batch.push(events[index].payload.clone());
            index += 1;
        }
        store.append(recorded_at, batch).map_err(server_error)?;
    }
    Ok(())
}

/// Wrap a raw [`StoreError`](zuihitsu::StoreError) as a server error, so a restore failure reads under
/// the same context as the rest of the run's store operations.
fn server_error(error: zuihitsu::StoreError) -> EvalError {
    EvalError::Server(Box::new(error.into()))
}

/// The agent every run is born as.
fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}
