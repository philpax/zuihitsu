//! The platform-authority facet: a client delivering participant turns, and the compaction the turn
//! loop triggers. It can act only as the participants it represents and cannot reach Control's
//! operator surface — the structural absence of those methods is what makes "the operator has no
//! platform identity" enforceable (spec §Clients and the server boundary).

use std::collections::BTreeSet;

use super::{Carryover, RoutedTurn, Server, ServerError};
use crate::{
    agent::{Flush, TurnOutcome, TurnView, buffer_turns, run_flush, session_touched},
    event::{EventPayload, Initiation, PromptTemplateName, TurnRole},
    ids::{ConversationId, ConversationLocator, MemoryId, Seq, TurnId},
    memory::{
        brief,
        identity::{resolve_or_mint_conversation, resolve_or_mint_participant},
        memory_block::Authority,
    },
    model::ModelClient,
    settings::Settings,
};

/// Platform-authority operations: a client delivering participant turns. It can act only as the
/// participants it represents, and cannot reach Control's operator surface.
pub struct Platform<'a> {
    pub(super) server: &'a mut Server,
}

impl Platform<'_> {
    /// Deliver an inbound message and run the agent's response cycle. The client hands over the room
    /// it arrived in, who sent it, and who is currently present (as platform user ids); the server
    /// resolves them to stubs (minting on first contact), opens or continues a session — freezing a
    /// fresh brief at each open — appends the inbound turn, runs the loop, and returns the outcome.
    pub async fn route_message(
        &mut self,
        model: &dyn ModelClient,
        locator: &ConversationLocator,
        sender: &str,
        text: &str,
        present: &[&str],
    ) -> Result<TurnOutcome, ServerError> {
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

    /// Note a participant arriving mid-session. If the room has a live session, this records a
    /// `ParticipantJoined` and injects the joiner's brief — built against the now-present set, so the
    /// subject-guard suppresses asides about them — as a `system` turn at the join point, rather than
    /// rebuilding the frozen prompt (spec §Mid-conversation joins). A no-op if the room has never been
    /// seen or has no live session; the next message then opens a session with the joiner present.
    pub fn note_join(
        &mut self,
        locator: &ConversationLocator,
        participant: &str,
    ) -> Result<(), ServerError> {
        let Some(conversation) = self
            .server
            .engine
            .graph
            .lock()
            .conversation_for_locator(locator)?
        else {
            return Ok(());
        };
        let Some(session) = self.server.sessions.get(&conversation).map(|open| open.id) else {
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

        // The brief is filtered against the present set including the joiner, so the subject-guard
        // fires for asides about them.
        let mut present_set = self
            .server
            .engine
            .graph
            .lock()
            .session_participants(session)?;
        if !present_set.contains(&joiner) {
            present_set.push(joiner);
        }
        let join_brief = brief::compose_participant(
            &self.server.engine.graph.lock(),
            joiner,
            &present_set,
            &Settings::from_store(self.server.engine.store.lock().as_ref())?.brief,
        )?;

        let now = self.server.engine.clock.now();
        let turn_id = TurnId::generate();
        self.server.engine.store.lock().append(
            now,
            vec![
                EventPayload::ParticipantJoined {
                    conversation,
                    session,
                    participant: joiner,
                    at_turn: turn_id,
                },
                EventPayload::ConversationTurn {
                    conversation,
                    turn_id,
                    role: TurnRole::System,
                    text: join_brief,
                    participant: Some(joiner),
                    initiation: Initiation::Responding,
                    produced_by: None,
                },
            ],
        )?;
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;
        Ok(())
    }

    /// End the live session because the buffer crossed the token budget, running the budget-gated
    /// pre-compaction flush and staging a raw-transcript carryover for the next message to re-segment
    /// from (spec §Compaction). The working-set carryover lands in a later stage.
    async fn end_session_for_compaction(
        &mut self,
        conversation: ConversationId,
        model: &dyn ModelClient,
    ) -> Result<(), ServerError> {
        let Some(open) = self.server.sessions.remove(&conversation) else {
            return Ok(());
        };
        let settings = Settings::from_store(self.server.engine.store.lock().as_ref())?;
        // The buffer includes the turn that just crossed the budget; it is both the flush's context
        // and the source of the carried tail.
        let buffer = buffer_turns(
            self.server.engine.store.lock().as_ref(),
            conversation,
            open.start_seq,
        )?;

        // Budget-gated pre-compaction flush: a substantive session gets one turn to write durable
        // working state to memory before the cut; a low-activity one (below the turn threshold) is
        // skipped, so the hot-path model call is paid only when there is something to flush.
        let flushed = buffer.len() as i64 >= settings.compaction.flush_min_turns;
        if flushed {
            run_flush(Flush {
                session: &open.vm,
                model,
                engine: self.server.engine.clone(),
                brief: &open.brief,
                buffer: &buffer,
                max_steps: settings.turn.max_steps as usize,
            })
            .await?;
            self.server
                .engine
                .graph
                .lock()
                .materialize_from(self.server.engine.store.lock().as_ref())?;
        }

        let now = self.server.engine.clock.now();
        self.server.engine.store.lock().append(
            now,
            vec![EventPayload::SessionEnded {
                conversation,
                id: open.id,
            }],
        )?;

        // Re-read the buffer (now including any flush turn) for the carried tail, and assemble the
        // working set (likewise after the flush, so its writes and active_in flags are included).
        let buffer = buffer_turns(
            self.server.engine.store.lock().as_ref(),
            conversation,
            open.start_seq,
        )?;
        let working_set = self.compaction_working_set(conversation, open.start_seq)?;
        if let Some(mut carry) = carryover_tail(&buffer, settings.compaction.carryover_char_budget)
        {
            carry.working_set = working_set;
            self.server.pending_carryover.insert(conversation, carry);
        }
        tracing::info!(
            ?conversation,
            session = ?open.id,
            flushed,
            "token budget crossed; ended session for compaction",
        );
        Ok(())
    }

    /// The working set carried across a compaction seam (spec §Compaction → working-set carryover):
    /// the context's `active_in`-flagged threads first — deliberate "keep this live" signals,
    /// promoted to first-class survivors — then the touch-derived set, deduped. (The third source,
    /// the brief's recent facts, the brief adds itself.) Read after the flush, which is what sets the
    /// `active_in` flags and records the touches.
    fn compaction_working_set(
        &self,
        conversation: ConversationId,
        from_seq: Seq,
    ) -> Result<Vec<MemoryId>, ServerError> {
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
                .outgoing(context, "has_active")?;
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
    let char_budget = char_budget.max(0) as usize;
    let mut total = 0usize;
    let mut oldest: Option<&TurnView> = None;
    for turn in buffer.iter().rev() {
        let next = total.saturating_add(turn.text.chars().count());
        if oldest.is_some() && next > char_budget {
            break;
        }
        total = next;
        oldest = Some(turn);
    }
    oldest.map(|turn| Carryover {
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
    use crate::{ids::TurnId, time::Timestamp};

    fn turn(seq: u64, text: &str) -> TurnView {
        TurnView {
            seq: Seq(seq),
            turn_id: TurnId::generate(),
            role: TurnRole::Participant,
            text: text.to_owned(),
            participant: None,
            recorded_at: Timestamp::from_millis(0),
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
    fn estimate_tokens_counts_buffer_and_inbound() {
        let buffer = vec![turn(1, "12345678")]; // 8 chars
        // (8 + 4) / 4 = 3.
        assert_eq!(estimate_tokens(&buffer, "1234"), 3);
    }
}
