//! Shared integration-test helpers. Included via `mod common;` from a test file; the directory
//! form keeps it from being compiled as its own test binary.

// Helpers (and the `Harness` re-export) are used by some test binaries and not others; that's
// expected for a shared module.
#![allow(dead_code, unused_imports)]

pub mod time;

pub use harness::Harness;

mod harness {
    use std::{sync::Arc, time::Duration};

    use zuihitsu::{
        Authority, BlockContext, BlockOutcome, CaptureLevel, ConversationId, Embedder, Engine,
        FakeEmbedder, Graph, InMemoryVectorIndex, ManualClock, MemoryId, MemoryStore, ModelClient,
        PromptTemplateName, Session, Teller, Turn, TurnId, TurnView, VectorIndex,
        model::index::{apply_batch, embed_batch},
    };

    use super::time::TEST_NOW;

    /// A block-duration budget generous enough that no in-memory test block ever trips it; the
    /// timeout's firing path is exercised directly in the MCP tests with a deliberately slow server.
    const TEST_BLOCK_TIMEOUT: Duration = Duration::from_secs(30);
    /// The per-block lock-wait retry bound for tests.
    const TEST_MAX_BLOCK_ATTEMPTS: u32 = 3;

    /// A complete agent backed entirely in memory: an in-memory event log, an in-memory graph, a
    /// manual clock, and one Lua session. The `engine` is the same shared handle the turn writes
    /// through, so a `run` and a subsequent `h.engine.graph.lock()` read observe each other. The
    /// `clock` field is a separate handle sharing the engine clock's atomic, for tests to read. Each
    /// `run` executes a block as its own turn.
    pub struct Harness {
        pub engine: Arc<Engine>,
        pub clock: ManualClock,
        pub session: Session,
        /// The stand-in inbound participant a turn is attributed to.
        pub participant: MemoryId,
    }

    impl Default for Harness {
        fn default() -> Self {
            let clock = ManualClock::new(TEST_NOW);
            Harness {
                engine: Engine::new(
                    Box::new(MemoryStore::new()),
                    Graph::open_in_memory().unwrap(),
                    Box::new(clock.clone()),
                ),
                clock,
                session: Session::new(ConversationId::generate()),
                participant: MemoryId::generate(),
            }
        }
    }

    /// The embedding dimensionality the retrieval-backed harness uses (the fake embedder's size).
    const TEST_EMBED_DIMS: usize = 16;

    impl Harness {
        pub fn new() -> Harness {
            Harness::default()
        }

        /// A harness whose engine has semantic retrieval attached (a fake embedder and in-memory
        /// vector index), for exercising `memory.search`. Drive [`Harness::index`] after a write to
        /// embed it before searching.
        pub fn with_retrieval() -> Harness {
            let clock = ManualClock::new(TEST_NOW);
            let embedder: Arc<dyn Embedder> = Arc::new(FakeEmbedder::new(TEST_EMBED_DIMS));
            let vectors: Box<dyn VectorIndex> = Box::new(InMemoryVectorIndex::new());
            Harness {
                engine: Engine::with_retrieval(
                    Box::new(MemoryStore::new()),
                    Graph::open_in_memory().unwrap(),
                    Box::new(clock.clone()),
                    embedder,
                    vectors,
                ),
                clock,
                session: Session::new(ConversationId::generate()),
                participant: MemoryId::generate(),
            }
        }

        /// Catch the harness's vector index up to its log — embed everything committed since the last
        /// call, so a subsequent `memory.search` can find it. Panics if the harness has no retrieval.
        pub async fn index(&self) {
            let retrieval = self.engine.retrieval.as_ref().expect("retrieval attached");
            let from = retrieval.vectors.lock().cursor().unwrap().next();
            let events = self.engine.store.lock().read_from(from).unwrap();
            let batch = embed_batch(retrieval.embedder.as_ref(), &events)
                .await
                .unwrap();
            apply_batch(&mut **retrieval.vectors.lock(), batch).unwrap();
        }

        /// Borrow the harness as a [`Turn`] over `model` for `inbound`, ready to hand to `run_turn`.
        /// Captures the full model-interaction record, the production default.
        pub fn as_turn<'a>(
            &'a self,
            model: &'a dyn ModelClient,
            inbound: &'a str,
            max_steps: usize,
        ) -> Turn<'a> {
            self.as_turn_capturing(model, inbound, max_steps, CaptureLevel::Full)
        }

        /// As [`Harness::as_turn`], but with an explicit model-interaction capture level — for tests
        /// that exercise the `Digest`/`Off` paths.
        pub fn as_turn_capturing<'a>(
            &'a self,
            model: &'a dyn ModelClient,
            inbound: &'a str,
            max_steps: usize,
            capture: CaptureLevel,
        ) -> Turn<'a> {
            Turn {
                session: &self.session,
                model,
                engine: self.engine.clone(),
                inbound,
                inbound_participant: self.participant,
                brief: "",
                session_started_at: self.engine.clock.now(),
                buffer: &[],
                template: PromptTemplateName::Scaffold,
                authority: Authority::Platform,
                present_set: &[],
                max_steps,
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                capture,
            }
        }

        /// As [`Harness::as_turn`], but replaying `buffer` as the prior conversation — for multi-turn
        /// scenarios where a later turn must see what the agent said and did earlier (build it with
        /// `buffer_turns` over the recorded turns).
        pub fn as_turn_buffered<'a>(
            &'a self,
            model: &'a dyn ModelClient,
            inbound: &'a str,
            max_steps: usize,
            buffer: &'a [TurnView],
        ) -> Turn<'a> {
            Turn {
                session: &self.session,
                model,
                engine: self.engine.clone(),
                inbound,
                inbound_participant: self.participant,
                brief: "",
                session_started_at: self.engine.clock.now(),
                buffer,
                template: PromptTemplateName::Scaffold,
                authority: Authority::Platform,
                present_set: &[],
                max_steps,
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                capture: CaptureLevel::Full,
            }
        }

        /// Execute one Lua block against the harness's engine, as a fresh agent-authored turn (the
        /// teller is the agent; see the conversation tests for participant-attributed writes).
        pub async fn run(&self, script: &str) -> BlockOutcome {
            self.session
                .execute(
                    &self.engine,
                    &BlockContext {
                        teller: Teller::Agent,
                        authority: Authority::Platform,
                        turn_id: TurnId::generate(),
                        present_set: Vec::new(),
                        block_timeout: TEST_BLOCK_TIMEOUT,
                        max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                    },
                    script,
                )
                .await
                .unwrap()
        }
    }
}
