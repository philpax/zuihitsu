//! The agent server: the single writer that owns the event log, the materialized graph, and the
//! clock, and exposes its API split by client authority (spec §Clients and the server boundary).
//!
//! Authority is a property of the client's role, enforced here — never of where the client runs.
//! The operator-authority surface is [`Control`] (agent creation and read-only inspection; its
//! writes are authored as source `Debugger`). The platform-authority surface — delivering
//! participant turns via `route_message` — arrives with the agent loop in Stage 4 as a sibling
//! facet that structurally lacks Control's creation and inspection methods, which is what makes
//! "the operator has no platform identity" enforceable.

use crate::{
    clock::Clock,
    event::{ConfigValue, EventPayload},
    genesis::{self, GenesisStatus, Rollout, SeedSelf},
    graph::{Graph, GraphError, MemoryView},
    ids::Seq,
    store::{MemoryStore, Store, StoreError},
};

pub struct Server {
    store: Box<dyn Store>,
    graph: Graph,
    clock: Box<dyn Clock>,
}

impl Server {
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Server {
        Server {
            store,
            graph,
            clock,
        }
    }

    /// A server backed entirely in memory (in-memory store and graph), for tests.
    pub fn in_memory(clock: Box<dyn Clock>) -> Result<Server, ServerError> {
        Ok(Server::new(
            Box::new(MemoryStore::new()),
            Graph::open_in_memory()?,
            clock,
        ))
    }

    /// Catch the graph up to log-head — reconciling a graph left stale or half-applied by a crash
    /// in the commit window — and classify the log for the caller to act on. The single-writer log
    /// lock is acquired when the (file-backed) store is opened, before the server is constructed.
    pub fn boot(&mut self) -> Result<GenesisStatus, ServerError> {
        let applied = self.graph.materialize_from(self.store.as_ref())?;
        let status = genesis::status(self.store.as_ref())?;
        tracing::info!(?status, applied, "server booted");
        Ok(status)
    }

    /// The operator-authority API facet.
    pub fn control(&mut self) -> Control<'_> {
        Control { server: self }
    }
}

/// Operator-authority operations: agent creation and read-only inspection. A platform client can
/// never obtain one of these.
pub struct Control<'a> {
    server: &'a mut Server,
}

impl Control<'_> {
    /// Create the agent — or resume an interrupted genesis — then project the new events so reads
    /// see them. Idempotent: calling it on a born agent is a no-op.
    pub fn create_agent(&mut self, seed: &SeedSelf) -> Result<Rollout, ServerError> {
        let outcome =
            genesis::rollout(self.server.store.as_mut(), self.server.clock.as_ref(), seed)?;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;
        Ok(outcome)
    }

    pub fn genesis_status(&self) -> Result<GenesisStatus, ServerError> {
        Ok(genesis::status(self.server.store.as_ref())?)
    }

    /// Inspect a live memory by name (e.g. `"self"`).
    pub fn memory(&self, name: &str) -> Result<Option<MemoryView>, ServerError> {
        Ok(self.server.graph.memory_by_name(name)?)
    }

    /// The current value of a behavioral tunable: the latest `ConfigSet` for `key`.
    pub fn config(&self, key: &str) -> Result<Option<ConfigValue>, ServerError> {
        let events = self.server.store.read_from(Seq::ZERO)?;
        let mut current = None;
        for event in events {
            if let EventPayload::ConfigSet {
                key: set_key,
                value,
                ..
            } = event.payload
                && set_key == key
            {
                current = Some(value);
            }
        }
        Ok(current)
    }
}

/// A server-side failure, delegating its message to the underlying store or graph error.
#[derive(Debug)]
pub enum ServerError {
    Store(StoreError),
    Graph(GraphError),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Store(error) => write!(f, "server: {error}"),
            ServerError::Graph(error) => write!(f, "server: {error}"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Store(error) => Some(error),
            ServerError::Graph(error) => Some(error),
        }
    }
}

impl From<StoreError> for ServerError {
    fn from(error: StoreError) -> Self {
        ServerError::Store(error)
    }
}

impl From<GraphError> for ServerError {
    fn from(error: GraphError) -> Self {
        ServerError::Graph(error)
    }
}
