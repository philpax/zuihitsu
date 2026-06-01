//! Shared integration-test helpers. Included via `mod common;` from a test file; the directory
//! form keeps it from being compiled as its own test binary.

// Helpers are used by some test binaries and not others; that's expected for a shared module.
#![allow(dead_code)]

#[cfg(feature = "lua")]
pub use harness::Harness;

#[cfg(feature = "lua")]
mod harness {
    use zuihitsu::{
        BlockOutcome, ConversationId, Graph, ManualClock, MemoryStore, Session, Timestamp, TurnId,
    };

    /// A complete agent backed entirely in memory: an in-memory event log, an in-memory graph, a
    /// manual clock, and one Lua session. Each `run` executes a block as its own turn.
    pub struct Harness {
        pub store: MemoryStore,
        pub graph: Graph,
        pub clock: ManualClock,
        pub session: Session,
    }

    impl Default for Harness {
        fn default() -> Self {
            Harness {
                store: MemoryStore::new(),
                graph: Graph::open_in_memory().unwrap(),
                clock: ManualClock::new(Timestamp::from_millis(1_000)),
                session: Session::new(ConversationId::generate()),
            }
        }
    }

    impl Harness {
        pub fn new() -> Harness {
            Harness::default()
        }

        /// Execute one Lua block against the harness's store and graph, as a fresh turn.
        pub fn run(&mut self, script: &str) -> BlockOutcome {
            self.session
                .execute(
                    &mut self.store,
                    &mut self.graph,
                    &self.clock,
                    TurnId::generate(),
                    script,
                )
                .unwrap()
        }
    }
}
