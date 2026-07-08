//! Recording a participant arriving mid-session.

use crate::{
    event::{EventPayload, Initiation, TurnRole},
    ids::{ConversationId, MemoryId, SessionId, TurnId},
    memory::brief,
    model::ModelClient,
    settings::Settings,
};

use super::super::{Instance, InstanceError};

impl Instance {
    /// Record a participant arriving mid-session: a `ParticipantJoined` plus the joiner's brief,
    /// injected as a `system` turn at the join point rather than by rebuilding the frozen prompt
    /// (spec §Mid-conversation joins). The brief is filtered against the present set including the
    /// joiner, so the subject-guard suppresses asides about them. When a model is available, the
    /// joiner's description is caught up first, so the brief never reads stale prose for a memory a
    /// prior turn just wrote (spec §Starvation bound → composing a brief forces the catch-up); with
    /// none (the modelless `/platform/join` path) the brief composes off the current prose — a
    /// slightly stale join-brief beats refusing the join. The joiner must already be resolved to a
    /// memory id; the caller owns locating the conversation and the live session. Shared by the
    /// per-message presence sync above and `Platform::note_join`.
    pub(crate) async fn join_participant(
        &self,
        model: Option<&dyn ModelClient>,
        conversation: ConversationId,
        session: SessionId,
        joiner: MemoryId,
    ) -> Result<(), InstanceError> {
        if let Some(model) = model {
            self.describe_catch_up_for(model, &[joiner]).await?;
        }
        let mut present_set = self.engine.graph.lock().session_participants(session)?;
        if !present_set.contains(&joiner) {
            present_set.push(joiner);
        }
        let now = self.engine.clock.now();
        // Compose the join-brief as structured data: the `system` turn carries the rendered markup as
        // its `text` (the prompt-build reads turn text verbatim, so the agent path is unchanged) and
        // the struct alongside, so a structured consumer renders a proper entrance rather than the raw
        // markup (spec §Mid-conversation joins).
        let join_brief = brief::compose_participant_brief(
            &self.engine.graph.lock(),
            joiner,
            &present_set,
            &Settings::from_store(self.engine.store.lock().as_ref())?.brief,
            now,
        )?;
        let text = join_brief
            .as_ref()
            .map(brief::Brief::render)
            .unwrap_or_default();

        let turn_id = TurnId::generate();
        self.engine.store.lock().append(
            now,
            vec![
                EventPayload::participant_joined(conversation, session, joiner, turn_id),
                EventPayload::ConversationTurn {
                    conversation,
                    turn_id,
                    role: TurnRole::System,
                    text,
                    participant: Some(joiner),
                    initiation: Initiation::Responding,
                    produced_by: None,
                    brief: join_brief,
                },
            ],
        )?;
        self.engine
            .graph
            .lock()
            .materialize_from(self.engine.store.lock().as_ref())?;
        Ok(())
    }
}
