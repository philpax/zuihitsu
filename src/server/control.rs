//! The operator-authority facet: agent creation and read-only inspection. A platform client can
//! never obtain one of these, which is what keeps the operator surface off the platform boundary
//! (spec §Clients and the server boundary).

#[cfg(feature = "lua")]
use super::RoutedTurn;
use super::{Server, ServerError};
#[cfg(feature = "lua")]
use crate::{
    agent::TurnOutcome,
    event::PromptTemplateName,
    memory::{identity::resolve_or_mint_conversation, memory_block::Authority},
    model::ModelClient,
};
use crate::{
    agent::genesis::{self, GenesisStatus, Rollout, SeedSelf},
    event::{EventPayload, EventSource},
    graph::{EntryView, MemoryView, SessionView},
    ids::ConversationLocator,
    settings::Settings,
};

/// Operator-authority operations: agent creation and read-only inspection. A platform client can
/// never obtain one of these.
pub struct Control<'a> {
    pub(super) server: &'a mut Server,
}

impl Control<'_> {
    /// Run one operator message of the imprint interview: the control-panel conversation where the
    /// operator introduces themselves and the agent learns who they are and what it is for (spec
    /// §Imprint interview). It runs under operator authority, so the agent may write `self` — the
    /// only path that may — and authors its links as `Debugger`. The operator is a stable
    /// `person/operator` stub (minted on first contact, no platform binding); the agent learns their
    /// real name, creates `person/<name>`, and merges the two with `same_as`. Multi-turn and
    /// re-runnable: each call delivers one operator message and runs the agent's response. No
    /// compaction — the interview is short, and its flush would run barred from `self`.
    #[cfg(feature = "lua")]
    pub async fn imprint(
        &mut self,
        model: &dyn ModelClient,
        text: &str,
    ) -> Result<TurnOutcome, ServerError> {
        let operator = self.server.resolve_or_mint_operator()?;
        let conversation = resolve_or_mint_conversation(
            self.server.store.as_mut(),
            self.server.clock.as_ref(),
            &self.server.graph,
            &ConversationLocator::new("operator", "imprint"),
        )?;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;
        let (report, _buffer) = self
            .server
            .run_session_turn(
                model,
                &RoutedTurn {
                    conversation,
                    present_set: &[operator],
                    participant: operator,
                    inbound: text,
                    template: PromptTemplateName::Imprint,
                    authority: Authority::Operator,
                },
            )
            .await?;
        Ok(report.outcome)
    }

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

    /// Inspect the live memories in a namespace (e.g. `"person/"`), ordered by name.
    pub fn memories(&self, prefix: &str) -> Result<Vec<MemoryView>, ServerError> {
        Ok(self.server.graph.memories_in_namespace(prefix)?)
    }

    /// Inspect the live memories carrying a `Recurring` occurrence — the operator's view of the
    /// agent's recurring calendar, the inspection parallel to the agent-facing `calendar.recurring()`.
    pub fn recurring(&self) -> Result<Vec<MemoryView>, ServerError> {
        Ok(self.server.graph.recurring_memories()?)
    }

    /// Inspect a memory's local content entries by name — their text, teller, and visibility — for
    /// auditing what was written and how it is gated (e.g. that a private aside was not stored
    /// `Public`). Empty if the memory is unknown.
    pub fn entries(&self, name: &str) -> Result<Vec<EntryView>, ServerError> {
        Ok(self
            .server
            .graph
            .memory_by_name(name)?
            .map(|m| self.server.graph.entries_local(m.id))
            .transpose()?
            .unwrap_or_default())
    }

    /// The agent's current behavioral settings: the latest `ConfigSet` snapshot.
    pub fn settings(&self) -> Result<Settings, ServerError> {
        Ok(Settings::from_store(self.server.store.as_ref())?)
    }

    /// Replace the agent's behavioral settings, logged as an operator `ConfigSet` (source
    /// `Debugger`) — the read-modify-write the configuration design calls for (spec §Initialization →
    /// configuration). The new snapshot is the latest and takes effect on the next read; settings are
    /// read from the log, so no projection is involved.
    pub fn set_settings(&mut self, settings: Settings) -> Result<(), ServerError> {
        let now = self.server.clock.now();
        self.server.store.append(
            now,
            vec![EventPayload::ConfigSet {
                settings,
                source: EventSource::Debugger,
            }],
        )?;
        Ok(())
    }

    /// The sessions of a conversation, addressed by its locator, oldest first — operator inspection
    /// of how the conversation segmented into sessions. Empty if the room has never been seen.
    pub fn sessions(&self, locator: &ConversationLocator) -> Result<Vec<SessionView>, ServerError> {
        match self.server.graph.conversation_for_locator(locator)? {
            Some(conversation) => Ok(self.server.graph.sessions_in(conversation)?),
            None => Ok(Vec::new()),
        }
    }
}
