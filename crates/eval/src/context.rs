//! The run context — a fresh, booted agent per run, with the helpers a scenario drives it through
//! (route a turn, advance the clock, catch the index up) and the run's event log afterwards. Each run
//! is independent (its own in-memory store, graph, and — when retrieval is configured — vector index),
//! which is what lets runs parallelize.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::broadcast::{Receiver, error::RecvError};
use zuihitsu::progress::{ProgressKind, TurnProgress};

use zuihitsu::{
    CheckpointTrigger, ConversationLocator, Embedder, Event, EventPayload, FakeWebFetcher, Graph,
    InstanceFeatures, LinkPosture, LinkSource, ManualClock, MemoryId, MemoryStore, ModelClient,
    PersonId, RelationName, SeedSelf, Seq, Server, SqliteVectorIndex, Store, Timestamp,
    TurnOutcome, Visibility,
};

use crate::{error::EvalError, fetch_fixture::FIXTURE_MAX_MARKDOWN_CHARS};

/// The fixed clock anchor every run starts at — midnight UTC, 8 June 2026 — so scenario timing is
/// reproducible; scenarios advance from here.
pub(crate) fn run_start() -> Timestamp {
    civil_timestamp(2026, 6, 8)
}

/// The named-civil-date constructor (`civil_timestamp(2026, 10, 3)`), re-exported from core so a
/// scenario states a fixed calendar date as a date rather than as an epoch literal or a day-offset
/// sum.
pub(crate) use zuihitsu::time::civil_timestamp;

/// The shared day/hour units every scenario expresses its clock advances and windows in, re-exported
/// from core so the derivation lives in one place rather than being redefined per scenario module.
pub(crate) use zuihitsu::time::{MILLIS_PER_DAY, MILLIS_PER_HOUR, MILLIS_PER_SECOND};

/// A human's pause before sending a message — applied before each inbound turn so consecutive turns in
/// a busy room are spaced apart, not stacked at one instant. Small against the day-scale advances a
/// scheduling scenario makes, so it does not perturb those.
const HUMAN_PAUSE_MS: i64 = 10_000;

/// How long [`RunContext::interrupted_turn`] waits for turn A's first generation frame before delivering
/// the interrupt regardless. Prefill on a local model is slow, so this is generous: if A produces no
/// frame within it — or completes with none — the interrupt is delivered anyway. The step never hangs on
/// the race, and the log records whatever happened for the oracles to judge.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(120);

/// The human beat between noticing the agent has started replying and sending the correction — the clock
/// advance [`RunContext::interrupted_turn`] applies after the first generation frame (or the timeout) and
/// before the interrupt is delivered, so the interrupt's arrival sits a realistic moment after the first
/// message's.
const INTERRUPT_PAUSE_MS: i64 = 3_000;

/// The shared, build-once inputs every run needs: the model, — when an embedding endpoint is
/// configured — the embedder and its dimensionality (a fresh vector index is built per run), and the
/// fixture web fetcher backing `web.markdown` (pure in-memory, so it is connected to every run
/// unconditionally, standing in for what the serving host builds from config).
#[derive(Clone)]
pub struct RunDeps {
    pub model: Arc<dyn ModelClient>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub dimensions: usize,
    pub web: Arc<FakeWebFetcher>,
}

/// One message of an [`RunContext::interrupted_turn`] burst, resolved to its sender stub and text — the
/// executor builds the two the platform delivers, keeping the method's argument list within house-style
/// bounds while reading as "the first delivery" and "the interrupt".
pub(crate) struct BurstDelivery<'a> {
    pub sender: &'a PersonId,
    pub text: &'a str,
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
    /// Birth a fresh agent from `seed` — its name, charter persona, and seed disposition entries. Most
    /// runs pass [`default_seed`]; an onboarding scenario overrides it (via [`Scenario::seed`]) to give
    /// the agent a rich, specific charter the imprint has real material to reason about.
    pub async fn new(
        deps: &RunDeps,
        features: InstanceFeatures,
        seed: &SeedSelf,
    ) -> Result<RunContext, EvalError> {
        let clock = ManualClock::new(run_start());
        let server = assemble(deps, features, &clock, Box::new(MemoryStore::new())).await?;
        // A fresh run is born: genesis writes the birth events into the empty log.
        server.control().create_agent(seed)?;
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
    /// rather than resetting to [`run_start`]. Genesis already sits in the restored log, so this
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
            .map(|event| event.recorded_at.as_millisecond())
            .unwrap_or(run_start().as_millisecond());
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
        sender: &PersonId,
        text: &str,
        present: &[PersonId],
    ) -> Result<TurnOutcome, EvalError> {
        self.clock.advance_millis(HUMAN_PAUSE_MS);
        let locator = ConversationLocator::new(platform, scope);
        let started = Instant::now();
        let response = self
            .server
            .platform()
            .route_message(self.model.as_ref(), &locator, sender, text, present)
            .await?;
        self.clock
            .advance_millis(started.elapsed().as_millis() as i64);
        Ok(response.outcome)
    }

    /// Drive a two-message burst into one room where the second message lands mid-generation, so the
    /// platform's per-conversation supersession cancels the in-flight generation and answers once with
    /// both messages in context. The concurrency is contained entirely within this call: turn A (the
    /// `first` message) is not spawned — [`Server`] is not `Clone` — so its future is pinned and driven
    /// here under `select!`/`join!`, and the whole burst reads as one journal step.
    ///
    /// The pacing mirrors [`RunContext::turn`] but counts once for the burst rather than once per
    /// message: one human pause before the first message, a short "noticed and corrected" beat before the
    /// interrupt, and one elapsed-think-time advance at the end (calling `turn` twice would double-count
    /// the per-call pauses). Phase 1 waits for A to begin generating (its first `Reasoning`/`Reply`
    /// frame), to complete early, or a generous timeout — whichever comes first; all three proceed. Phase
    /// 2 delivers the interrupt: if A already completed, B is simply awaited; otherwise both are driven
    /// concurrently so the interrupt lands while A generates. Every race outcome proceeds — the step never
    /// hangs and never fails on the race — and both outcomes are returned for the caller to inspect (the
    /// executor discards them; the event log is the assessed product).
    pub(crate) async fn interrupted_turn(
        &self,
        platform: &str,
        scope: &str,
        first: BurstDelivery<'_>,
        interrupt: BurstDelivery<'_>,
        present: &[PersonId],
    ) -> Result<(TurnOutcome, TurnOutcome), EvalError> {
        // Subscribe before launching A, so no frame A emits between launch and the phase-1 select is
        // missed — the run is quiescent when the burst begins, so the first generation frame is A's.
        let mut progress = self.subscribe_progress();

        // One human pause for the whole burst, and one elapsed-think advance at the end: driving the two
        // messages through `turn` would advance the clock twice, double-counting the pacing.
        self.clock.advance_millis(HUMAN_PAUSE_MS);
        let locator = ConversationLocator::new(platform, scope);
        let started = Instant::now();
        let platform = self.server.platform();

        // Turn A opens the turn. Pinned rather than spawned (no `'static`), so it is polled by the
        // phase-1 select and, if it does not complete there, the phase-2 join.
        let a_future = platform.route_message(
            self.model.as_ref(),
            &locator,
            first.sender,
            first.text,
            present,
        );
        tokio::pin!(a_future);

        // Phase 1: proceed as soon as A starts generating, completes early, or the timeout elapses.
        let mut a_outcome: Option<TurnOutcome> = None;
        tokio::select! {
            result = &mut a_future => a_outcome = Some(result?.outcome),
            () = first_generation_frame(&mut progress) => {}
            () = tokio::time::sleep(FIRST_FRAME_TIMEOUT) => {}
        }

        // The human beat between noticing the reply and sending the correction.
        self.clock.advance_millis(INTERRUPT_PAUSE_MS);

        // Phase 2: deliver the interrupt. If A already completed, just await B; otherwise drive both so
        // the interrupt lands mid-generation and supersedes A.
        let b_future = platform.route_message(
            self.model.as_ref(),
            &locator,
            interrupt.sender,
            interrupt.text,
            present,
        );
        let (a_outcome, b_outcome) = match a_outcome {
            Some(a) => (a, b_future.await?.outcome),
            None => {
                let (a, b) = tokio::join!(&mut a_future, b_future);
                (a?.outcome, b?.outcome)
            }
        };

        self.clock
            .advance_millis(started.elapsed().as_millis() as i64);
        Ok((a_outcome, b_outcome))
    }

    /// Pin the per-conversation supersession window (`TurnSettings::supersede_window_seconds`) so a
    /// scripted burst lands inside it — a real turn can outlast the 60s default, so a supersession
    /// scenario widens the window and the interrupt reliably cancels rather than queueing. Mirrors
    /// [`RunContext::tighten_compaction`], leaving the rest of the settings as seeded.
    pub(crate) fn tune_supersession(&self, window_seconds: i64) -> Result<(), EvalError> {
        let mut settings = self.server.control().settings()?;
        settings.turn.supersede_window_seconds = window_seconds;
        self.server.control().set_settings(settings)?;
        Ok(())
    }

    /// Drive one operator imprint-interview turn — the `operator/imprint` channel, under operator
    /// authority (the only path that may write `self`), distinct from the participant turns `turn`
    /// drives. Paces the clock like `turn`, so an `advance` between imprint turns crosses the idle gap
    /// into a fresh session just as it does for participants.
    pub(crate) async fn imprint(&self, text: &str) -> Result<TurnOutcome, EvalError> {
        self.clock.advance_millis(HUMAN_PAUSE_MS);
        let started = Instant::now();
        let response = self
            .server
            .control()
            .imprint(self.model.as_ref(), text)
            .await?;
        self.clock
            .advance_millis(started.elapsed().as_millis() as i64);
        Ok(response.outcome)
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
    /// directly. This is the one path to a merge — a proposal pends until the operator acts on it. Drives
    /// the operator confirmation a proposal surfaces for, so a scenario can assess what the agent does
    /// once identity is confirmed.
    pub(crate) fn operator_merge(&self, from: MemoryId, to: MemoryId) -> Result<(), EvalError> {
        self.seed_events(vec![EventPayload::link_created(
            from,
            to,
            RelationName::SameAs,
            LinkPosture {
                source: LinkSource::Operator,
                told_by: None,
                told_in: None,
                visibility: Visibility::Public,
            },
        )])
    }

    /// Advance the run's clock by `delta_ms` — to cross a recurrence instance or an idle gap.
    pub(crate) fn advance(&self, delta_ms: i64) {
        self.clock.advance_millis(delta_ms);
    }

    /// Advance the run's clock just past the configured idle gap (plus a 1-second epsilon), so the
    /// next turn opens a fresh session. Reads the live `idle_gap_seconds` setting rather than baking
    /// the default into a compile-time constant — a default change cannot silently break the cross.
    pub(crate) fn advance_past_idle_gap(&self) {
        let idle_gap_ms = self
            .server
            .control()
            .settings()
            .expect("settings read during eval")
            .compaction
            .idle_gap_seconds
            * MILLIS_PER_SECOND;
        self.clock.advance_millis(idle_gap_ms + MILLIS_PER_SECOND);
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

    /// Tune the checkpoint gates so a scripted two-room exchange trips them: the substance threshold,
    /// the cooldown, and whether a fresh session open flushes the other rooms first (`flush_on_open`),
    /// leaving the enable flag and the rest of the settings as seeded. A timer-path scenario disables
    /// `flush_on_open` so the open trigger does not pre-empt the explicit sweep it drives.
    pub(crate) fn tune_checkpoint(
        &self,
        min_delta_chars: i64,
        cooldown_seconds: i64,
        flush_on_open: bool,
    ) -> Result<(), EvalError> {
        let mut settings = self.server.control().settings()?;
        settings.checkpoint.min_delta_chars = min_delta_chars;
        settings.checkpoint.cooldown_seconds = cooldown_seconds;
        settings.checkpoint.flush_on_open = flush_on_open;
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
            .checkpoint_live_sessions(self.model.as_ref(), CheckpointTrigger::Timer)
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

    /// Run the maintenance passes — consolidation, canonicalize, and link cleanup — so a scenario
    /// that asserts on consolidated entries or canonical profiles drives them explicitly.
    pub(crate) async fn maintenance_catch_up(&self) -> Result<(), EvalError> {
        self.server
            .consolidation_catch_up(self.model.as_ref())
            .await?;
        self.server
            .canonicalize_catch_up(self.model.as_ref())
            .await?;
        self.server
            .link_cleanup_catch_up(self.model.as_ref())
            .await?;
        Ok(())
    }

    /// The run's whole event log — the record the harness embeds and assessment reads.
    pub(crate) fn events(&self) -> Result<Vec<Event>, EvalError> {
        Ok(self.server.control().events()?)
    }

    /// Subscribe to the booted instance's ephemeral turn-progress feed, so the harness can forward
    /// the deliberation's tokens into the live stream as [`crate::live::LiveEvent::RunProgress`].
    pub(crate) fn subscribe_progress(
        &self,
    ) -> tokio::sync::broadcast::Receiver<zuihitsu::progress::TurnProgress> {
        self.server.subscribe_progress()
    }

    /// The run's events recorded at or after `from` — for streaming a run's deliberation live as it
    /// drives, reading only what is new since the last poll.
    pub(crate) fn events_from(&self, from: Seq) -> Result<Vec<Event>, EvalError> {
        Ok(self.server.control().events_from(from)?)
    }
}

/// Await the first progress frame signalling that turn A has begun generating — a `Reasoning` or `Reply`
/// fragment. `Lagged` (the lossy broadcast dropped frames under load) is skipped and the wait continues;
/// `Closed` (every sender dropped) behaves like the timeout, returning so phase 2 proceeds rather than
/// hanging. The run is quiescent when the burst begins, so the first such frame is unambiguously A's.
async fn first_generation_frame(progress: &mut Receiver<TurnProgress>) {
    loop {
        match progress.recv().await {
            Ok(frame) if matches!(frame.kind, ProgressKind::Reasoning | ProgressKind::Reply) => {
                return;
            }
            Ok(_) => continue,
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        }
    }
}

/// Build a server around `store` and `clock` with the scenario's feature set, connect the fixture web
/// fetcher before boot so sessions opened during the run can fetch, and boot. Shared by
/// [`RunContext::new`] (which then births a fresh agent) and [`RunContext::restored`] (which boots into
/// a restored log): boot materializes the graph from whatever the store already holds, so it serves
/// both the empty-log birth and the existing-log restart.
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
    // The `FakeWebFetcher` is pure in-memory — no subprocess, no network — so it is connected to every
    // run. The real `HttpFetcher` never reaches the eval.
    server.connect_web(deps.web.clone(), FIXTURE_MAX_MARKDOWN_CHARS);
    server.boot()?;
    Ok(server)
}

/// Append `events` to a fresh store preserving each event's `recorded_at` and `source`. Consecutive
/// events sharing both a timestamp and an authority ride in one batch (the store stamps a batch with a
/// single `recorded_at` and `source`), so the seqs regenerate `1..=N` in the recorded order while
/// every event keeps its original recorded time and author.
fn restore_verbatim(store: &mut MemoryStore, events: &[Event]) -> Result<(), EvalError> {
    let mut index = 0;
    while index < events.len() {
        let recorded_at = events[index].recorded_at;
        let source = events[index].source.clone();
        let mut batch = Vec::new();
        while index < events.len()
            && events[index].recorded_at == recorded_at
            && events[index].source == source
        {
            batch.push(events[index].payload.clone());
            index += 1;
        }
        store
            .append(recorded_at, source, batch)
            .map_err(server_error)?;
    }
    Ok(())
}

/// Wrap a raw [`StoreError`](zuihitsu::StoreError) as a server error, so a restore failure reads under
/// the same context as the rest of the run's store operations.
fn server_error(error: zuihitsu::StoreError) -> EvalError {
    EvalError::Server(Box::new(error.into()))
}

/// The agent every run is born as, unless the scenario overrides it via [`Scenario::seed`].
pub(crate) fn default_seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}
