//! The run context — a fresh, booted agent per run, with the helpers a scenario drives it through
//! (route a turn, advance the clock, catch the index up) and the run's event log afterwards. Each run
//! is independent (its own in-memory store, graph, and — when retrieval is configured — vector index),
//! which is what lets runs parallelize.

use std::{sync::Arc, time::Instant};

use zuihitsu::{
    ConversationLocator, Embedder, Event, EventPayload, Graph, InstanceFeatures, LinkSource,
    ManualClock, MemoryId, MemoryStore, ModelClient, RelationName, SeedSelf, Seq, Server,
    SqliteVectorIndex, Timestamp, TurnOutcome, Visibility,
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

/// The shared, build-once inputs every run needs: the model, and — when an embedding endpoint is
/// configured — the embedder and its dimensionality (a fresh vector index is built per run).
#[derive(Clone)]
pub struct RunDeps {
    pub model: Arc<dyn ModelClient>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub dimensions: usize,
}

/// One inbound message for [`RunContext::turn`], named so a call site reads as what it is rather than
/// as five positional strings. `present` defaults to just the sender; [`Turn::with_present`] overrides
/// it when others share the room — who else is present changes what the visibility predicate surfaces.
pub struct Turn<'a> {
    platform: &'a str,
    scope: &'a str,
    sender: &'a str,
    text: &'a str,
    present: Vec<&'a str>,
}

impl<'a> Turn<'a> {
    /// A turn from `sender` in `platform`/`scope`, with `sender` as the only one present.
    pub fn new(platform: &'a str, scope: &'a str, sender: &'a str, text: &'a str) -> Turn<'a> {
        Turn {
            platform,
            scope,
            sender,
            text,
            present: vec![sender],
        }
    }

    /// Override who is present for this turn (the default is the sender alone). The sender is always
    /// present, so it is added if the caller's set omits it.
    pub fn with_present(mut self, present: &[&'a str]) -> Turn<'a> {
        self.present = present.to_vec();
        if !self.present.contains(&self.sender) {
            self.present.push(self.sender);
        }
        self
    }
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
        let mut server = match &deps.embedder {
            Some(embedder) => Server::with_retrieval_features(
                Box::new(MemoryStore::new()),
                Graph::open_in_memory()?,
                Box::new(clock.clone()),
                embedder.clone(),
                Box::new(SqliteVectorIndex::open_in_memory(deps.dimensions)?),
                features,
            ),
            None => Server::with_features(
                Box::new(MemoryStore::new()),
                Graph::open_in_memory()?,
                Box::new(clock.clone()),
                features,
            ),
        };
        server.boot()?;
        server.control().create_agent(&seed())?;
        Ok(RunContext {
            server,
            model: deps.model.clone(),
            clock,
        })
    }

    /// Route one inbound message and run the agent's turn, returning what it said. Advances the run
    /// clock so turns sit on a realistic timescale: a human pause before the message, then the agent's
    /// actual think time after — so the recorded timestamps reflect how the conversation paced (legible
    /// especially in the multi-party rooms), rather than stacking every turn at one frozen instant.
    pub async fn turn(&self, turn: Turn<'_>) -> Result<TurnOutcome, EvalError> {
        self.clock.advance_millis(HUMAN_PAUSE_MS);
        let locator = ConversationLocator::new(turn.platform, turn.scope);
        let started = Instant::now();
        let outcome = self
            .server
            .platform()
            .route_message(
                self.model.as_ref(),
                &locator,
                turn.sender,
                turn.text,
                &turn.present,
            )
            .await?;
        self.clock
            .advance_millis(started.elapsed().as_millis() as i64);
        Ok(outcome)
    }

    /// Drive one operator imprint-interview turn — the `operator/imprint` channel, under operator
    /// authority (the only path that may write `self`), distinct from the participant turns `turn`
    /// drives. Paces the clock like `turn`, so an `advance` between imprint turns crosses the idle gap
    /// into a fresh session just as it does for participants.
    pub async fn imprint(&self, text: &str) -> Result<TurnOutcome, EvalError> {
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
    pub fn seed_events(&self, events: Vec<EventPayload>) -> Result<(), EvalError> {
        self.server.control().seed_events(events)?;
        Ok(())
    }

    /// Confirm a cross-platform merge as the operator would from the console (spec §Cross-platform
    /// identity → operator-asserted merge): author the `same_as` link between two `person/*` stubs
    /// directly, the one path to a merge that does not run through the adjudicator. Drives the operator
    /// confirmation a proposal surfaces for, so a scenario can assess what the agent does once identity
    /// is confirmed.
    pub fn operator_merge(&self, from: MemoryId, to: MemoryId) -> Result<(), EvalError> {
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
    pub fn advance(&self, delta_ms: i64) {
        self.clock.advance_millis(delta_ms);
    }

    /// Tighten the compaction trigger so a short scripted session crosses the token budget and flushes
    /// before the cut (the fixture-22/23 setup). Sets the budget and the flush floor, leaving the rest
    /// of the settings as seeded.
    pub fn tighten_compaction(
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
    pub async fn force_compaction(&self, platform: &str, scope: &str) -> Result<bool, EvalError> {
        let locator = ConversationLocator::new(platform, scope);
        Ok(self
            .server
            .platform()
            .force_compaction(self.model.as_ref(), &locator)
            .await?)
    }

    /// Tune the checkpoint gates so a scripted two-room exchange trips them: the substance threshold
    /// and the cooldown, leaving the enable flag and the rest of the settings as seeded.
    pub fn tune_checkpoint(
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
    pub async fn checkpoint_sweep(&self) -> Result<usize, EvalError> {
        Ok(self
            .server
            .checkpoint_live_sessions(self.model.as_ref())
            .await?)
    }

    /// Catch the vector index up to the log, so a fact written this run is searchable next turn (the
    /// same catch-up the background indexer runs).
    pub async fn index_catch_up(&self) -> Result<(), EvalError> {
        self.server.index_catch_up().await?;
        Ok(())
    }

    /// Let both background synthesis passes settle: the describer (descriptions, arbitration, temporal
    /// extraction) and then the vector indexer, in that order. This is the pair scenarios run together
    /// after a turn that wrote content — before advancing the clock across a gap or asserting on the
    /// synthesized-and-searchable state — folded into one call. A scenario that needs only one of the
    /// two (no retrieval, or a description-only probe) calls the specific catch-up directly.
    pub async fn settle(&self) -> Result<(), EvalError> {
        self.describe_catch_up().await?;
        self.index_catch_up().await?;
        Ok(())
    }

    /// Regenerate descriptions, belief arbitration, and temporal extraction for everything written so
    /// far — the off-hot-path synthesis the background describer runs, driven explicitly (spec §Write
    /// path). A scenario that asserts on a synthesized description, an arbitration, or a resolved
    /// occurrence calls this after the turn that wrote it, before its log is assessed.
    pub async fn describe_catch_up(&self) -> Result<(), EvalError> {
        self.server.describe_catch_up(self.model.as_ref()).await?;
        Ok(())
    }

    /// Adjudicate the merges proposed so far — the off-hot-path pass the background adjudicator runs,
    /// driven explicitly (spec §Cross-platform identity → adjudicated merge). A scenario that proposes a
    /// merge calls this before its log is assessed, so the verdict (and any `same_as`) is recorded.
    pub async fn adjudicate_catch_up(&self) -> Result<(), EvalError> {
        self.server.adjudicate_catch_up(self.model.as_ref()).await?;
        Ok(())
    }

    /// Infer links from the content written so far — the off-hot-path pass the background
    /// link-inference worker runs, driven explicitly (spec §Write path → link inference). A scenario
    /// that asserts on an inferred link calls this after the turn that wrote the content, before its
    /// log is assessed.
    pub async fn link_inference_catch_up(&self) -> Result<(), EvalError> {
        self.server
            .link_inference_catch_up(self.model.as_ref())
            .await?;
        Ok(())
    }

    /// The run's whole event log — the record the harness embeds and assessment reads.
    pub fn events(&self) -> Result<Vec<Event>, EvalError> {
        Ok(self.server.control().events()?)
    }

    /// The run's events recorded at or after `from` — for streaming a run's deliberation live as it
    /// drives, reading only what is new since the last poll.
    pub fn events_from(&self, from: Seq) -> Result<Vec<Event>, EvalError> {
        Ok(self.server.control().events_from(from)?)
    }
}

/// The agent every run is born as.
fn seed() -> SeedSelf {
    SeedSelf {
        agent_name: "Kestrel".to_owned(),
        persona: "A thoughtful, discreet companion with a long memory.".to_owned(),
        seed_entries: vec!["I keep what people tell me in confidence.".to_owned()],
    }
}
