//! The platform-authority facet: a client delivering participant turns, and the compaction the turn
//! loop triggers. It can act only as the participants it represents and cannot reach Control's
//! operator surface — the structural absence of those methods is what makes "the operator has no
//! platform identity" enforceable (spec §Clients and the server boundary).

use std::collections::BTreeSet;

use super::{Carryover, Instance, InstanceError, RoutedTurn};
use crate::{
    agent::{TurnOutcome, TurnView, bounded_buffer_turns, carryover_start, session_touched},
    event::PromptTemplateName,
    ids::{ConversationId, ConversationLocator, MemoryId, Seq},
    memory::{
        identity::{resolve_or_mint_conversation, resolve_or_mint_participant},
        memory_block::Authority,
    },
    model::ModelClient,
    settings::Settings,
    vocabulary::RelationName,
};

/// Platform-authority operations: a client delivering participant turns. It can act only as the
/// participants it represents, and cannot reach Control's operator surface.
pub struct Platform<'a> {
    pub(super) server: &'a Instance,
}

impl Platform<'_> {
    /// Deliver an inbound message and run the agent's response cycle. The client hands over the room
    /// it arrived in, who sent it, and who is currently present (as platform user ids); the server
    /// resolves them to stubs (minting on first contact), opens or continues a session — freezing a
    /// fresh brief at each open — appends the inbound turn, runs the loop, and returns the outcome.
    pub async fn route_message(
        &self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
        sender: &str,
        text: &str,
        present: &[&str],
    ) -> Result<TurnOutcome, InstanceError> {
        // Hold a stream permit for this message's whole handling — the turn and any compaction flush
        // it triggers — so no more than `max_concurrent_streams` messages crowd the shared model at
        // once (spec §Concurrency). Released when this scope returns.
        let _stream = self
            .server
            .streams
            .acquire()
            .await
            .expect("the stream semaphore is never closed");

        // Resolve the room (minting its context memory on first contact) and the participants. Each
        // call borrows the store, clock, and graph fields disjointly and releases before the next,
        // so the interleaved `materialize_from` calls are free to take the graph mutably.
        let conversation = {
            // Graph before store, per the lock-ordering rule (this resolve holds both at once).
            let graph = self.server.engine.graph.lock();
            resolve_or_mint_conversation(
                self.server.engine.store.lock().as_mut(),
                self.server.engine.clock.as_ref(),
                &graph,
                locator,
            )?
        };
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

        // The unique platform ids to resolve: everyone present, plus the sender. Deduplicating
        // matters because resolution reads the graph, which is not re-materialized between mints
        // within this call — the same id seen twice would otherwise be minted twice.
        let platform = locator.platform.as_str();
        let mut uids: Vec<&str> = Vec::new();
        for uid in present.iter().chain(std::iter::once(&sender)) {
            if !uids.contains(uid) {
                uids.push(uid);
            }
        }
        let mut present_set = Vec::new();
        let mut sender_id = None;
        for uid in &uids {
            let id = {
                // Graph before store, per the lock-ordering rule.
                let graph = self.server.engine.graph.lock();
                resolve_or_mint_participant(
                    self.server.engine.store.lock().as_mut(),
                    self.server.engine.clock.as_ref(),
                    &graph,
                    platform,
                    uid,
                )?
            };
            if *uid == sender {
                sender_id = Some(id);
            }
            present_set.push(id);
        }
        let sender_id = sender_id.expect("the sender is among the resolved ids");
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

        // Open or continue the session and run the turn under ordinary platform authority.
        let (report, buffer) = self
            .server
            .run_session_turn(
                model,
                &RoutedTurn {
                    conversation,
                    present_set: &present_set,
                    participant: sender_id,
                    inbound: text,
                    template: PromptTemplateName::Scaffold,
                    authority: Authority::Platform,
                },
            )
            .await?;

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
            .unwrap_or_else(|| estimate_tokens(&buffer, text));
        // `reported` distinguishes the authoritative real-usage path from the coarse estimate
        // fallback: if the backend never reports `prompt_tokens`, the trigger is running on the
        // (system-prefix-omitting) estimate, which is an operability signal worth seeing.
        tracing::debug!(
            observed,
            token_budget,
            reported = report.prompt_tokens.is_some(),
            "compaction trigger check",
        );
        if observed > token_budget {
            self.end_session_for_compaction(conversation, model).await?;
        }
        Ok(report.outcome)
    }

    /// Note a participant arriving mid-session — the explicit join path, for clients that deliver
    /// presence changes as their own signal (the per-message presence sync in the turn path covers
    /// those that only deliver messages). If the room has a live session, this records a
    /// `ParticipantJoined` and injects the joiner's brief — built against the now-present set, so the
    /// subject-guard suppresses asides about them — as a `system` turn at the join point, rather than
    /// rebuilding the frozen prompt (spec §Mid-conversation joins). A no-op if the room has never been
    /// seen or has no live session; the next message then opens a session with the joiner present.
    /// `model` feeds the joiner's describe catch-up before the brief composes; with none configured
    /// the brief composes off the current prose — a slightly stale join-brief beats refusing the join.
    pub async fn note_join(
        &self,
        model: Option<&dyn ModelClient>,
        locator: &ConversationLocator,
        participant: &str,
    ) -> Result<(), InstanceError> {
        let Some(conversation) = self
            .server
            .engine
            .graph
            .lock()
            .conversation_for_locator(locator)?
        else {
            return Ok(());
        };
        let Some(session) = self.server.sessions.get(conversation).map(|open| open.id) else {
            return Ok(());
        };

        let joiner = {
            // Graph before store, per the lock-ordering rule.
            let graph = self.server.engine.graph.lock();
            resolve_or_mint_participant(
                self.server.engine.store.lock().as_mut(),
                self.server.engine.clock.as_ref(),
                &graph,
                locator.platform.as_str(),
                participant,
            )?
        };
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

        self.server
            .join_participant(model, conversation, session, joiner)
            .await
    }

    /// End the live session because the buffer crossed the token budget, running the budget-gated
    /// pre-compaction flush and staging a raw-transcript carryover for the next message to re-segment
    /// from (spec §Compaction). The working-set carryover lands in a later stage.
    async fn end_session_for_compaction(
        &self,
        conversation: ConversationId,
        model: &dyn ModelClient,
    ) -> Result<(), InstanceError> {
        // Take the session out under the map guard; the `Arc` then carries it across the flush and
        // `shutdown_mcp().await` below without holding the guard.
        let Some(open) = self.server.sessions.remove(conversation) else {
            return Ok(());
        };
        let settings = Settings::from_store(self.server.engine.store.lock().as_ref())?;
        // Flush the ending session's working state to memory and record `SessionEnded` (shared with the
        // idle and recovery closes); the buffer the flush reads includes the turn that crossed the
        // budget. The carried tail and working set are staged below, after the flush turn lands.
        let flushed = self
            .server
            .flush_and_end(conversation, open.as_ref(), model)
            .await?;

        // Stage the next carryover from this session's *own* turns (those at or after its
        // `SessionStarted`), not the whole buffer — so `from_seq` advances into the current session
        // with each seam rather than sticking at the original carryover point (the buffer would
        // otherwise re-span every session since it, unbounded, when the turns are small relative to the
        // char budget). The prior carried tail has already served its continuity; the token-budget
        // compaction bounds this session's own turns, and `carryover_tail` trims them to the char
        // budget. The read starts at this session's own start (an empty carried tail).
        let own = bounded_buffer_turns(
            self.server.engine.store.lock().as_ref(),
            conversation,
            open.session_start_seq,
            open.session_start_seq,
            settings.compaction.carryover_char_budget,
        )?;
        let working_set = self.compaction_working_set(conversation, open.start_seq)?;
        if let Some(mut carry) = carryover_tail(&own, settings.compaction.carryover_char_budget) {
            carry.working_set = working_set;
            self.server.sessions.insert_carryover(conversation, carry);
        }
        tracing::info!(
            ?conversation,
            session = ?open.id,
            flushed,
            "token budget crossed; ended session for compaction",
        );
        crate::metrics::observe_compaction();
        Ok(())
    }

    /// The working set carried across a compaction seam (spec §Compaction → working-set carryover):
    /// the context's `_session_carryover`-flagged threads first — deliberate "keep this live" signals,
    /// promoted to first-class survivors — then the touch-derived set, deduped. (The third source,
    /// the brief's recent facts, the brief adds itself.) Read after the flush, which is what sets the
    /// `_session_carryover` flags and records the touches.
    fn compaction_working_set(
        &self,
        conversation: ConversationId,
        from_seq: Seq,
    ) -> Result<Vec<MemoryId>, InstanceError> {
        let mut working_set = Vec::new();
        let mut seen = BTreeSet::new();
        // Resolve the context and its active threads up front, each releasing the graph guard before
        // the next read, so the lock (not reentrant) is never held while re-acquired.
        let context = self
            .server
            .engine
            .graph
            .lock()
            .context_for_conversation(conversation)?;
        if let Some(context) = context {
            let actives = self
                .server
                .engine
                .graph
                .lock()
                .outgoing(context, RelationName::SessionCarries.as_str())?;
            for memory in actives {
                if seen.insert(memory.id) {
                    working_set.push(memory.id);
                }
            }
        }
        let touched = session_touched(
            self.server.engine.store.lock().as_ref(),
            conversation,
            from_seq,
        )?;
        for id in touched {
            if seen.insert(id) {
                working_set.push(id);
            }
        }
        Ok(working_set)
    }
}

/// The raw-transcript carryover tail: the most recent turns that fit `char_budget`, filled backward
/// from the cut (spec §Compaction → raw-transcript carryover). The newest turn is always carried so
/// the immediate conversational thread survives the seam, then older turns are added while they fit.
/// Returns the oldest carried turn as the carryover extent, or `None` for an empty buffer.
fn carryover_tail(buffer: &[TurnView], char_budget: i64) -> Option<Carryover> {
    let start = carryover_start(buffer, char_budget);
    buffer.get(start).map(|turn| Carryover {
        seeded_from_turn: turn.turn_id,
        from_seq: turn.seq,
        // Filled in by the caller, which has the session's touched set.
        working_set: Vec::new(),
    })
}

/// A deterministic `chars / 4` estimate of the prompt's token count over the buffer plus the inbound
/// message — the compaction-trigger fallback when the backend reports no usage. Coarse and an
/// under-count (it omits the frozen prefix); only the real client's `prompt_tokens` is authoritative.
fn estimate_tokens(buffer: &[TurnView], inbound: &str) -> i64 {
    let chars: usize = buffer
        .iter()
        .map(|turn| turn.text.chars().count())
        .sum::<usize>()
        + inbound.chars().count();
    (chars / 4) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{event::TurnRole, ids::TurnId, time::Timestamp};

    fn turn(seq: u64, text: &str) -> TurnView {
        TurnView {
            seq: Seq(seq),
            turn_id: TurnId::generate(),
            role: TurnRole::Participant,
            text: text.to_owned(),
            participant: None,
            recorded_at: Timestamp::from_millis(0),
            steps: Vec::new(),
        }
    }

    #[test]
    fn carryover_tail_admits_the_newest_turns_that_fit_the_budget() {
        // Texts of 4, 4, and 2 chars, newest last.
        let buffer = vec![turn(1, "aaaa"), turn(2, "bbbb"), turn(3, "cc")];
        // Budget 6 admits "cc" (2) + "bbbb" (4) = 6, but not the next "aaaa" — extent is seq 2.
        let carry = carryover_tail(&buffer, 6).expect("a non-empty buffer carries a tail");
        assert_eq!(carry.from_seq, Seq(2));
        assert_eq!(carry.seeded_from_turn, buffer[1].turn_id);
    }

    #[test]
    fn carryover_tail_always_keeps_the_newest_turn_even_over_budget() {
        let buffer = vec![
            turn(1, "short"),
            turn(2, "a long final turn that alone exceeds the budget"),
        ];
        // The immediate thread survives the seam: the newest turn is carried regardless.
        let carry = carryover_tail(&buffer, 1).expect("the newest turn is always carried");
        assert_eq!(carry.from_seq, Seq(2));
        assert_eq!(carry.seeded_from_turn, buffer[1].turn_id);
    }

    #[test]
    fn carryover_tail_of_an_empty_buffer_is_none() {
        assert!(carryover_tail(&[], 100).is_none());
    }

    #[test]
    fn carryover_start_indexes_the_oldest_turn_that_fits() {
        let buffer = vec![turn(1, "aaaa"), turn(2, "bbbb"), turn(3, "cc")];
        // Budget 6 admits "cc" (2) + "bbbb" (4) = 6, not "aaaa" — the kept tail starts at index 1.
        assert_eq!(carryover_start(&buffer, 6), 1);
        // A budget below the newest turn still keeps it (index 2), never an empty tail.
        assert_eq!(carryover_start(&buffer, 0), 2);
        // A budget the whole buffer fits keeps everything (index 0).
        assert_eq!(carryover_start(&buffer, 1_000), 0);
        // An empty slice keeps nothing — the past-the-end index.
        assert_eq!(carryover_start(&[], 100), 0);
    }

    #[test]
    fn estimate_tokens_counts_buffer_and_inbound() {
        let buffer = vec![turn(1, "12345678")]; // 8 chars
        // (8 + 4) / 4 = 3.
        assert_eq!(estimate_tokens(&buffer, "1234"), 3);
    }
}
