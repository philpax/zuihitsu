//! Control inspection: read-only views of the agent's state — memories, entries, sessions, events,
//! model calls, settings, and metrics gauges.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    agent::genesis::{self, GenesisStatus},
    event::{Event, EventPayload, EventSource},
    graph::{EntryView, MemoryView, SessionView},
    ids::{ConversationLocator, MemoryId, MemoryName, Seq},
    metrics::{set_graph_counts, set_head_seq, set_lag, set_mcp, set_sessions_active},
    settings::Settings,
};

use super::super::InstanceError;
use super::{Arbitration, MergeProposal, ModelCall, canonical_pair};

impl super::Control<'_> {
    pub fn genesis_status(&self) -> Result<GenesisStatus, InstanceError> {
        Ok(genesis::status(self.server.engine.store.lock().as_ref())?)
    }

    /// Inspect a live memory by name (e.g. `"self"`).
    pub fn memory(&self, name: &str) -> Result<Option<MemoryView>, InstanceError> {
        Ok(self
            .server
            .engine
            .graph
            .lock()
            .memory_by_name(MemoryName::new(name))?)
    }

    /// Inspect the live memories in a namespace (e.g. `"person/"`), ordered by name.
    pub fn memories(&self, prefix: &str) -> Result<Vec<MemoryView>, InstanceError> {
        Ok(self
            .server
            .engine
            .graph
            .lock()
            .memories_in_namespace(prefix)?)
    }

    /// Inspect the live memories carrying a `Recurring` occurrence — the operator's view of the
    /// agent's recurring calendar, the inspection parallel to the agent-facing `calendar.recurring()`.
    pub fn recurring(&self) -> Result<Vec<MemoryView>, InstanceError> {
        Ok(self.server.engine.graph.lock().recurring_memories()?)
    }

    /// The belief arbitrations the agent has recorded, oldest first — for each, the memory it concerns
    /// and the reconciling statement. The audit surface for "why does it believe X" (spec §Write path);
    /// `BeliefArbitrated` is log-only, so this reads it from the log rather than the graph.
    pub fn arbitrations(&self) -> Result<Vec<Arbitration>, InstanceError> {
        let mut out = Vec::new();
        let events = self.server.engine.store.lock().read_from(Seq::ZERO)?;
        for event in events {
            if let EventPayload::BeliefArbitrated {
                memory, resolution, ..
            } = event.payload
            {
                let name = self
                    .server
                    .engine
                    .graph
                    .lock()
                    .memory_by_id(memory)?
                    .map(|memory| memory.name)
                    .unwrap_or_else(|| MemoryName::new("<unknown>"));
                out.push(Arbitration {
                    memory: name,
                    statement: resolution.statement,
                });
            }
        }
        Ok(out)
    }

    /// The cross-platform merge proposals still awaiting the operator, in first-proposal order (spec
    /// §Cross-platform identity → adjudicated merge). A proposal whose two stubs now share a `same_as`
    /// class has been merged (by the adjudicator or an operator) and drops off; every other proposal —
    /// unweighed, or weighed and refused — stays, so the operator's backstop never silently loses one.
    /// `MergeProposed`/`MergeAdjudicated` are log-only, so this reads them from the log and resolves the
    /// current class membership from the graph.
    pub fn merge_proposals(&self) -> Result<Vec<MergeProposal>, InstanceError> {
        let events = self.server.engine.store.lock().read_from(Seq::ZERO)?;
        // Track each pair by its canonical key (`same_as` is symmetric) for settlement matching, but
        // keep the original `(from, to)` order of the first proposal for a stable display direction.
        let mut order: Vec<(MemoryId, MemoryId)> = Vec::new();
        let mut source: BTreeMap<(MemoryId, MemoryId), super::MergeProposalSource> = BTreeMap::new();
        let mut refused: BTreeSet<(MemoryId, MemoryId)> = BTreeSet::new();
        for event in events {
            match event.payload {
                EventPayload::MergeProposed {
                    from,
                    to,
                    source: raised_by,
                    ..
                } => {
                    let pair = canonical_pair(from, to);
                    if source.insert(pair, raised_by).is_none() {
                        order.push((from, to));
                    }
                }
                EventPayload::MergeAdjudicated {
                    from, to, accepted, ..
                } => {
                    let pair = canonical_pair(from, to);
                    // The latest verdict wins: an accept clears a prior refusal, a refusal marks it.
                    if accepted {
                        refused.remove(&pair);
                    } else {
                        refused.insert(pair);
                    }
                }
                _ => {}
            }
        }

        let graph = self.server.engine.graph.lock();
        let mut out = Vec::new();
        for (from, to) in order {
            // A pair now in one class has been merged — nothing left for the operator to decide.
            let from_class = graph.class_id(from)?;
            if from_class.is_some() && from_class == graph.class_id(to)? {
                continue;
            }
            let name = |id| -> Result<MemoryName, InstanceError> {
                Ok(graph
                    .memory_by_id(id)?
                    .map(|memory| memory.name)
                    .unwrap_or_else(|| MemoryName::new("<unknown>")))
            };
            let pair = canonical_pair(from, to);
            out.push(MergeProposal {
                from: name(from)?,
                to: name(to)?,
                source: source[&pair],
                refused: refused.contains(&pair),
            });
        }
        Ok(out)
    }

    /// The model interactions recorded on the log, oldest first — each call's request (delta-encoded),
    /// deliberation, token usage, and latency. The console's deliberation surface and the answer to
    /// "where did the turn's time go" (spec §Observability); `ModelCalled` is log-only, so this reads
    /// it from the log. Returns nothing under the `Off` capture level, since no events were written.
    pub fn model_calls(&self) -> Result<Vec<ModelCall>, InstanceError> {
        let mut out = Vec::new();
        for event in self.server.engine.store.lock().read_from(Seq::ZERO)? {
            let seq = event.seq;
            let recorded_at = event.recorded_at;
            if let EventPayload::ModelCalled {
                conversation,
                turn_id,
                phase,
                request_digest,
                request,
                completion,
                reasoning,
                finish_reason,
                usage,
                duration_ms,
            } = event.payload
            {
                out.push(ModelCall {
                    seq,
                    recorded_at,
                    conversation,
                    turn_id,
                    phase,
                    request_digest,
                    request,
                    completion,
                    reasoning,
                    finish_reason,
                    usage,
                    duration_ms,
                });
            }
        }
        Ok(out)
    }

    /// The whole event log, oldest first — the raw record everything else is derived from (spec
    /// §Observability → the Events view). The console reconstructs its views from it.
    pub fn events(&self) -> Result<Vec<Event>, InstanceError> {
        self.events_from(Seq::ZERO)
    }

    /// The event log from `from` onward (every event with `seq >= from`), in order. The live
    /// console's catch-up and tail surface (spec §Observability → live phase): an initial
    /// `events_from(ZERO)` seeds the replica, then repeated `events_from(head)` polls the new tail.
    pub fn events_from(&self, from: Seq) -> Result<Vec<Event>, InstanceError> {
        Ok(self.server.engine.store.lock().read_from(from)?)
    }

    /// Inspect a memory's local content entries by name — their text, teller, and visibility — for
    /// auditing what was written and how it is gated (e.g. that a private aside was not stored
    /// `Public`). Empty if the memory is unknown.
    pub fn entries(&self, name: &str) -> Result<Vec<EntryView>, InstanceError> {
        let graph = self.server.engine.graph.lock();
        Ok(graph
            .memory_by_name(MemoryName::new(name))?
            .map(|m| graph.entries_local(m.id))
            .transpose()?
            .unwrap_or_default())
    }

    /// The agent's current behavioral settings: the latest `ConfigSet` snapshot.
    pub fn settings(&self) -> Result<Settings, InstanceError> {
        Ok(Settings::from_store(
            self.server.engine.store.lock().as_ref(),
        )?)
    }

    /// Refresh the derived gauges from instance state, so a `/control/metrics` scrape sees fresh
    /// agent-state values (spec §Observability → metrics). The process-level gauges (uptime, the
    /// event-log file size) are set by the serving host, which knows the boot time and the log path;
    /// everything else — the graph's size, the live session count, the worker lag, the MCP catalogue
    /// — is derived here from the instance.
    pub fn refresh_gauges(&self) -> Result<(), InstanceError> {
        let head = self.server.engine.store.lock().head()?;
        set_head_seq(head.0);
        set_sessions_active(self.server.sessions.active_count() as u64);
        let graph = self.server.engine.graph.lock();
        set_graph_counts(
            graph.memory_count()?,
            graph.entry_count()?,
            graph.link_count()?,
            graph.all_tags()?.len(),
            graph.all_relations()?.len(),
        );
        // Read through the graph guard already held above — the graph lock is not reentrant.
        let describer_backlog = graph.stale_memory_count()?;
        let adjudicator_lag = head
            .0
            .saturating_sub(self.server.adjudicator_cursor_value().0);
        let indexer_lag = self.server.engine.retrieval.as_ref().map(|retrieval| {
            retrieval
                .vectors
                .lock()
                .cursor()
                .map(|cursor| head.0.saturating_sub(cursor.0))
                .unwrap_or(head.0)
        });
        set_lag(indexer_lag, describer_backlog, adjudicator_lag);
        let (servers_up, tools_total) = self
            .server
            .mcp
            .as_ref()
            .map(|runtime| {
                (
                    runtime.catalogue.server_count(),
                    runtime.catalogue.tool_count(),
                )
            })
            .unwrap_or((0, 0));
        set_mcp(servers_up, tools_total);
        Ok(())
    }

    /// Replace the agent's behavioral settings, logged as an operator `ConfigSet` (source
    /// `Operator`) — the read-modify-write the configuration design calls for (spec §Initialization →
    /// configuration). The new snapshot is the latest and takes effect on the next read; settings are
    /// read from the log, so no projection is involved.
    pub fn set_settings(&self, settings: Settings) -> Result<(), InstanceError> {
        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            vec![EventPayload::config_set(settings, EventSource::Operator)],
        )?;
        Ok(())
    }

    /// The sessions of a conversation, addressed by its locator, oldest first — operator inspection
    /// of how the conversation segmented into sessions. Empty if the room has never been seen.
    pub fn sessions(
        &self,
        locator: &ConversationLocator,
    ) -> Result<Vec<SessionView>, InstanceError> {
        let graph = self.server.engine.graph.lock();
        match graph.conversation_for_locator(locator)? {
            Some(conversation) => Ok(graph.sessions_in(conversation)?),
            None => Ok(Vec::new()),
        }
    }

    /// Append raw events to the store and materialize the graph, for callers that set up
    /// deterministic state directly rather than driving the agent through a conversation. The events
    /// are appended as-is (the caller constructs them), so the caller controls exactly what state
    /// exists — no agent or Lua in the loop. The clock advances to the store head afterward, so a
    /// subsequent catch-up pass sees the seeded state.
    pub fn seed_events(&self, events: Vec<EventPayload>) -> Result<(), InstanceError> {
        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(now, events)?;
        // Graph (written) before store (read), per the lock-ordering rule.
        let mut graph = self.server.engine.graph.lock();
        graph.materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(())
    }
}
