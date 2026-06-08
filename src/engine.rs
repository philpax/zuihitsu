//! The shared backends a turn threads as a unit: the append-only event log (`store`), the graph
//! projection it feeds (`graph`), and the clock that stamps writes (`clock`).
//!
//! They always travel together, so they ride as one value rather than three parallel arguments — the
//! shared shape behind [`crate::agent::Turn`], the pre-compaction flush, the step loop, and
//! [`crate::agent::lua::Session::execute`]. The whole bundle lives behind a single [`Arc`], so a turn
//! shares it with one pointer bump and the Lua block API can hold a `'static` handle to it across the
//! script's `eval_async` (the block API moved off mlua's borrowing `scope` to make that possible).
//!
//! Each backend is locked transiently for a single read or write; **nothing holds a guard across an
//! `.await`**. When two are needed at once — only `materialize_from`, which reads the store while
//! writing the graph, and the scheduler's `fire_due` — the **graph is locked before the store**, the
//! one ordering rule that keeps the (non-reentrant) locks deadlock-free once sessions run concurrently.

use std::sync::Arc;

use parking_lot::Mutex;

use crate::{clock::Clock, graph::Graph, store::Store};

/// The store, graph, and clock a turn operates over, bundled behind one [`Arc`] (see the module docs
/// for the locking discipline). Built once per agent and cloned cheaply for each turn.
pub struct Engine {
    pub store: Mutex<Box<dyn Store>>,
    pub graph: Mutex<Graph>,
    pub clock: Box<dyn Clock>,
}

impl Engine {
    /// Bundle the three backends behind a shared [`Arc`].
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Arc<Engine> {
        Arc::new(Engine {
            store: Mutex::new(store),
            graph: Mutex::new(graph),
            clock,
        })
    }
}
