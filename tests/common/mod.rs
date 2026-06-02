//! Shared integration-test helpers. Included via `mod common;` from a test file; the directory
//! form keeps it from being compiled as its own test binary.

// Helpers are used by some test binaries and not others; that's expected for a shared module.
#![allow(dead_code)]

#[cfg(feature = "lua")]
pub use harness::Harness;

#[cfg(feature = "lua")]
mod harness {
    use zuihitsu::{
        BlockOutcome, ConversationId, Graph, ManualClock, MemoryId, MemoryStore, ModelClient,
        Session, Teller, Timestamp, Turn, TurnId,
    };

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
                clock: ManualClock::new(Timestamp::from_millis(1_000)),
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
                store: &mut self.store,
                graph: &mut self.graph,
                clock: &self.clock,
                inbound,
                inbound_participant: self.participant,
                max_steps,
            }
        }

        /// Execute one Lua block against the harness's store and graph, as a fresh agent-authored
        /// turn (the teller is the agent; see the conversation tests for participant-attributed
        /// writes).
        pub fn run(&mut self, script: &str) -> BlockOutcome {
            self.session
                .execute(
                    &mut self.store,
                    &mut self.graph,
                    &self.clock,
                    Teller::Agent,
                    TurnId::generate(),
                    script,
                )
                .unwrap()
        }
    }
}
