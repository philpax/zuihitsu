//! The run context — a fresh, booted agent per run, with the helpers a scenario drives it through
//! (route a turn, advance the clock, catch the index up) and the run's event log afterwards. Each run
//! is independent (its own in-memory store, graph, and — when retrieval is configured — vector index),
//! which is what lets runs parallelize.

use std::sync::Arc;

use zuihitsu::{
    ConversationLocator, Embedder, Event, Graph, ManualClock, MemoryStore, ModelClient, SeedSelf,
    Server, SqliteVectorIndex, Timestamp, TurnOutcome,
};

use crate::error::EvalError;

/// The fixed clock anchor every run starts at (2026-06-08T00:00:00Z), so scenario timing is
/// reproducible; scenarios advance from here.
const RUN_START_MS: i64 = 1_780_876_800_000;

/// The shared, build-once inputs every run needs: the model, and — when an embedding endpoint is
/// configured — the embedder and its dimensionality (a fresh vector index is built per run).
#[derive(Clone)]
pub struct RunDeps {
    pub model: Arc<dyn ModelClient>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub dimensions: usize,
}

/// One inbound message for [`RunContext::turn`], named so a call site reads as what it is rather than
/// as five positional strings. `present` is just the sender for now; a `with_present` override arrives
/// with the privacy scenarios, where who else is in the room changes what may surface.
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
}

/// One run's booted agent and the clock it runs against.
pub struct RunContext {
    server: Server,
    model: Arc<dyn ModelClient>,
    clock: ManualClock,
}

impl RunContext {
    /// Build, boot, and birth a fresh agent for one run.
    pub async fn new(deps: &RunDeps) -> Result<RunContext, EvalError> {
        let clock = ManualClock::new(Timestamp::from_millis(RUN_START_MS));
        let mut server = match &deps.embedder {
            Some(embedder) => Server::with_retrieval(
                Box::new(MemoryStore::new()),
                Graph::open_in_memory()?,
                Box::new(clock.clone()),
                embedder.clone(),
                Box::new(SqliteVectorIndex::open_in_memory(deps.dimensions)?),
            ),
            None => Server::new(
                Box::new(MemoryStore::new()),
                Graph::open_in_memory()?,
                Box::new(clock.clone()),
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

    /// Route one inbound message and run the agent's turn, returning what it said.
    pub async fn turn(&self, turn: Turn<'_>) -> Result<TurnOutcome, EvalError> {
        let locator = ConversationLocator::new(turn.platform, turn.scope);
        Ok(self
            .server
            .platform()
            .route_message(
                self.model.as_ref(),
                &locator,
                turn.sender,
                turn.text,
                &turn.present,
            )
            .await?)
    }

    /// Advance the run's clock by `delta_ms` — to cross a recurrence instance or an idle gap.
    pub fn advance(&self, delta_ms: i64) {
        self.clock.advance_millis(delta_ms);
    }

    /// Catch the vector index up to the log, so a fact written this run is searchable next turn (the
    /// same catch-up the background indexer runs).
    pub async fn index_catch_up(&self) -> Result<(), EvalError> {
        self.server.index_catch_up().await?;
        Ok(())
    }

    /// The run's whole event log — the record the harness embeds and assessment reads.
    pub fn events(&self) -> Result<Vec<Event>, EvalError> {
        Ok(self.server.control().events()?)
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
