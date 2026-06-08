//! Shared integration-test helpers. Included via `mod common;` from a test file; the directory
//! form keeps it from being compiled as its own test binary.

// Helpers are used by some test binaries and not others; that's expected for a shared module.
#![allow(dead_code)]

#[cfg(feature = "lua")]
pub use harness::Harness;

#[cfg(feature = "lua")]
mod harness {
    use zuihitsu::{
        Authority, BlockContext, BlockOutcome, ConversationId, Engine, Graph, ManualClock,
        MemoryId, MemoryStore, ModelClient, PromptTemplateName, Session, Teller, Timestamp, Turn,
        TurnId,
    };

    /// A realistic, non-epoch test clock (2026-06-08T00:00:00Z). Starting near the Unix epoch made
    /// model-gated runs resolve relative phrases like "last Tuesday" into 1969/1970 and risked the
    /// model overfitting to that period; a present-day base keeps the declared "now" lifelike.
    const TEST_NOW: Timestamp = Timestamp(1_780_876_800_000);

    /// A complete agent backed entirely in memory: an in-memory event log, an in-memory graph, a
    /// manual clock, and one Lua session. Each `run` executes a block as its own turn.
    pub struct Harness {
        pub store: MemoryStore,
        pub graph: Graph,
        pub clock: ManualClock,
        pub session: Session,
        /// The stand-in inbound participant a turn is attributed to.
        pub participant: MemoryId,
    }

    impl Default for Harness {
        fn default() -> Self {
            Harness {
                store: MemoryStore::new(),
                graph: Graph::open_in_memory().unwrap(),
                clock: ManualClock::new(TEST_NOW),
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
        pub fn as_turn<'a>(
            &'a mut self,
            model: &'a dyn ModelClient,
            inbound: &'a str,
            max_steps: usize,
        ) -> Turn<'a> {
            Turn {
                session: &self.session,
                model,
                engine: Engine {
                    store: &mut self.store,
                    graph: &mut self.graph,
                    clock: &self.clock,
                },
                inbound,
                inbound_participant: self.participant,
                brief: "",
                buffer: &[],
                template: PromptTemplateName::Scaffold,
                authority: Authority::Platform,
                max_steps,
            }
        }

        /// Execute one Lua block against the harness's store and graph, as a fresh agent-authored
        /// turn (the teller is the agent; see the conversation tests for participant-attributed
        /// writes).
        pub fn run(&mut self, script: &str) -> BlockOutcome {
            self.session
                .execute(
                    &mut Engine {
                        store: &mut self.store,
                        graph: &mut self.graph,
                        clock: &self.clock,
                    },
                    &BlockContext {
                        teller: Teller::Agent,
                        authority: Authority::Platform,
                        turn_id: TurnId::generate(),
                    },
                    script,
                )
                .unwrap()
        }
    }
}
