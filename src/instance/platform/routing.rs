//! Message routing and the compaction the turn loop triggers: delivering participant batches into a
//! session, and ending a session to compact when the buffer crosses the token budget.

use std::collections::HashMap;

use zuihitsu_platform_connector_types::PlatformResponse;

use crate::{
    agent::{InboundMessage, TurnError, TurnOutcome},
    event::{PromptTemplateName, SessionEndCause},
    ids::{ConversationId, ConversationLocator, MemoryId, PersonId, TurnId},
    instance::{
        InstanceError, RoutedTurn,
        platform::{MessageInput, Platform, estimate_tokens},
    },
    memory::memory_block::Authority,
    model::ModelClient,
    settings::Settings,
};

impl Platform<'_> {
    /// Deliver a single inbound message and run the agent's response cycle — a convenience for the
    /// common single-message case, equivalent to [`route_messages`](Self::route_messages) with a
    /// one-element batch.
    pub async fn route_message(
        &self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
        sender: &PersonId,
        text: &str,
        present: &[PersonId],
    ) -> Result<PlatformResponse, InstanceError> {
        self.route_messages(
            model,
            locator,
            &[MessageInput {
                sender: sender.clone(),
                text: text.to_owned(),
            }],
            present,
        )
        .await
    }

    /// Deliver a batch of inbound messages and run one agent response cycle. The client hands over
    /// the room it arrived in, the messages (each with its own sender), and who is currently present
    /// (as platform user ids); the server resolves them to stubs (minting on first contact), opens or
    /// continues a session — freezing a fresh brief at each open — appends each inbound turn, runs
    /// the loop once, and returns the outcome.
    pub async fn route_messages(
        &self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
        messages: &[MessageInput],
        present: &[PersonId],
    ) -> Result<PlatformResponse, InstanceError> {
        // Resolve the room (minting its context memory on first contact), then the participants. Each
        // resolution is atomic under the graph lock (resolve, mint, and materialize under one guard),
        // so a concurrent first contact for the same room or identity cannot double-mint.
        let conversation = self.ensure_conversation(locator)?;

        // The unique platform ids to resolve: everyone present, plus every sender. Deduplicating spares
        // the redundant resolve-and-materialize cycle for an id seen twice; each first-contact mint is
        // already made atomic by `resolve_or_mint_person`.
        let mut uids: Vec<&PersonId> = Vec::new();
        for person in present.iter().chain(messages.iter().map(|m| &m.sender)) {
            if !uids.contains(&person) {
                uids.push(person);
            }
        }
        let mut present_set = Vec::new();
        let mut participant_ids: HashMap<&PersonId, MemoryId> = HashMap::new();
        for person in &uids {
            let id = self.resolve_or_mint_person(person)?;
            participant_ids.insert(*person, id);
            present_set.push(id);
        }

        // Build the inbound batch and generate turn ids. The participant turns are recorded inside
        // `run_session_turn` (after `ensure_session` opens the session) so the session's `start_seq`
        // precedes them — the flush substance gate reads the buffer from `start_seq`, and must see the
        // turns to measure their delta.
        let mut inbound: Vec<InboundMessage> = Vec::with_capacity(messages.len());
        let mut participant_turn_ids: Vec<TurnId> = Vec::with_capacity(messages.len());
        for msg in messages {
            let participant = *participant_ids.get(&msg.sender).unwrap();
            let turn_id = TurnId::generate();
            inbound.push(InboundMessage {
                participant,
                text: msg.text.clone(),
            });
            participant_turn_ids.push(turn_id);
        }

        // Read the supersession window before registering the arrival. The window is store-backed and
        // read per batch, so it is runtime-tunable; a zero (or negative) window leaves the admitted
        // turn uncancellable while serialization stays on. Reading it first means a failed settings
        // read short-circuits before `arrive`, so the supersession signal never fires for a batch that
        // then evaporates — an arrival that bumped the watch but never admitted would spuriously
        // supersede an in-flight turn.
        let window_seconds = Settings::from_store(self.server.engine.store.lock().as_ref())?
            .turn
            .supersede_window_seconds;
        let window = std::time::Duration::from_secs(window_seconds.max(0) as u64);

        // Register the batch's arrival before queueing for anything, so a turn already generating for
        // this conversation is signalled the moment this batch lands, not after it has waited for a
        // slot or a permit (spec §Concurrency → per-conversation supersession). Resolution and minting
        // above ran under no permit — the permit caps concurrent model work, which none of that is.
        let ticket = self
            .server
            .turns
            .arrive(conversation, self.server.engine.clock.now());

        // Take the conversation's turn slot: this batch's turn waits here behind any earlier turn for
        // the same conversation, so a room's turns never overlap on the shared session VM.
        let mut admission = ticket.admit(window).await;

        // Hold a stream permit for the model work this batch drives — the turn and any compaction flush
        // it triggers — so no more than `max_concurrent_streams` turns crowd the shared model at once
        // (spec §Concurrency). Taken only after slot admission so waiting batches never camp on a
        // permit; released when this scope returns.
        let _stream = self
            .server
            .streams
            .acquire()
            .await
            .expect("the stream semaphore is never closed");

        // Open or continue the session and run the turn under ordinary platform authority, handing the
        // turn its supersession handle so a newer batch's arrival can cooperatively cancel it.
        let (report, buffer) = self
            .server
            .run_session_turn(
                model,
                &RoutedTurn {
                    conversation,
                    present_set: &present_set,
                    inbound: &inbound,
                    participant_turn_ids: &participant_turn_ids,
                    template: PromptTemplateName::Scaffold,
                    authority: Authority::Platform,
                },
                Some(admission.supersession()),
            )
            .await?;

        // A deferred or superseded turn skips the compaction check entirely. A deferred turn's model
        // proved unreachable, so the pre-compaction flush could not run anyway and the buffer gained
        // no agent turn. A superseded turn lost its slot to a newer batch whose turn now owns the
        // buffer: running compaction from this loser would race the winner, so the winner's own
        // handling performs the compaction check instead.
        if matches!(
            report.outcome,
            TurnOutcome::Deferred | TurnOutcome::Superseded
        ) {
            // A superseded turn lost its slot to a newer batch: leave its arrival anchoring the burst
            // (the successor measures its window from the original origin and clears the arrivals when
            // it completes) rather than pruning as a completed turn would.
            if matches!(report.outcome, TurnOutcome::Superseded) {
                admission.mark_superseded();
            }
            return Ok(PlatformResponse {
                outcome: report.outcome,
                participant_turn_ids: report
                    .participant_turn_ids
                    .iter()
                    .map(|id| id.0.to_string())
                    .collect(),
            });
        }

        // Token-triggered compaction: if the turn's peak prompt crossed the budget, end the session
        // now so the next message re-segments with a fresh brief and a carried tail (spec
        // §Compaction). The estimate fallback keeps the trigger meaningful when the backend reports
        // no usage (the in-memory and no-openai builds).
        let token_budget = Settings::from_store(self.server.engine.store.lock().as_ref())?
            .compaction
            .token_budget;
        let observed = report
            .prompt_tokens
            .map(i64::from)
            .unwrap_or_else(|| estimate_tokens(&buffer, messages));
        // `reported` distinguishes the authoritative real-usage path from the coarse estimate
        // fallback: if the backend never reports `prompt_tokens`, the trigger is running on the
        // (system-prefix-omitting) estimate, which is an operability signal worth seeing.
        tracing::debug!(
            observed,
            token_budget,
            reported = report.prompt_tokens.is_some(),
            "compaction trigger check",
        );
        if observed > token_budget
            && let Err(error) = self.end_session_for_compaction(conversation, model).await
        {
            // The turn's outcome is already in hand; if the model went down between the reply and
            // the compaction flush, deliver the reply rather than turning it into an error. The
            // flush failed before `SessionEnded`, so the session is still open in the log — the
            // next message's cold-start recovery resumes or closes it (the session was already
            // taken out of the live map).
            match &error {
                InstanceError::Turn {
                    error: TurnError::Model(model_error),
                    ..
                } if model_error.is_unavailable() => {
                    tracing::warn!(
                        %error,
                        "the model became unreachable during the compaction flush; delivering \
                         the reply and leaving the session for recovery"
                    );
                }
                _ => return Err(error),
            }
        }
        Ok(PlatformResponse {
            outcome: report.outcome,
            participant_turn_ids: report
                .participant_turn_ids
                .iter()
                .map(|id| id.0.to_string())
                .collect(),
        })
    }

    /// Force the live session in `locator`'s room to end and compact right now, through the exact path
    /// the token-budget trigger drives — the pre-compaction flush, the raw-transcript and working-set
    /// carryover staging, and a fresh session seeded from that carryover on the next message. This
    /// states the intent "cut here" directly, so a caller that wants a compaction seam at a chosen
    /// point does not have to size a token budget so the organic trigger *happens* to fire. Returns
    /// whether a live session was compacted — `false` if the room has never been seen or has no live
    /// session.
    pub async fn force_compaction(
        &self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
    ) -> Result<bool, InstanceError> {
        let Some(conversation) = self
            .server
            .engine
            .graph
            .lock()
            .conversation_for_locator(locator)?
        else {
            return Ok(false);
        };
        if self.server.sessions.get(conversation).is_none() {
            return Ok(false);
        }
        self.end_session_for_compaction(conversation, model).await?;
        Ok(true)
    }

    /// End the live session because the buffer crossed the token budget (spec §Compaction). Runs the
    /// budget-gated pre-compaction flush and records `SessionEnded` with a [`SessionEndCause::Compaction`]
    /// cause (inside [`Instance::flush_and_end`]). Nothing is staged for the reopen: the next message's
    /// `ensure_session` reconstructs the tail from the log, and because it reopens promptly (within the
    /// idle gap) it reads as a warm continuation and carries this session's touched set — no runtime hand-
    /// off needed (issue #86).
    async fn end_session_for_compaction(
        &self,
        conversation: ConversationId,
        model: &dyn ModelClient,
    ) -> Result<(), InstanceError> {
        // Take the session out under the map guard; the `Arc` then carries it across the flush and
        // `shutdown_mcp().await` inside `flush_and_end` without holding the guard.
        let Some(open) = self.server.sessions.remove(conversation) else {
            return Ok(());
        };
        // Flush the ending session's working state to memory and record `SessionEnded`; the buffer the
        // flush reads includes the turn that crossed the budget.
        let flushed = self
            .server
            .flush_and_end(
                conversation,
                open.as_ref(),
                model,
                SessionEndCause::Compaction,
            )
            .await?;
        tracing::info!(
            ?conversation,
            session = ?open.id,
            flushed,
            "token budget crossed; ended session for compaction",
        );
        crate::metrics::observe_compaction();
        Ok(())
    }
}
