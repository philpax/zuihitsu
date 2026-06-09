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
        Authority, BlockContext, BlockOutcome, CaptureLevel, ConversationId, Engine, Graph,
        ManualClock, MemoryId, MemoryStore, ModelClient, PromptTemplateName, Session, Teller, Turn,
        TurnId,
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

    impl Harness {
        pub fn new() -> Harness {
            Harness::default()
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
                buffer: &[],
                template: PromptTemplateName::Scaffold,
                authority: Authority::Platform,
                max_steps,
                block_timeout: TEST_BLOCK_TIMEOUT,
                max_block_attempts: TEST_MAX_BLOCK_ATTEMPTS,
                capture,
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
