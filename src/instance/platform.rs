//! The platform-authority facet: a client delivering participant turns, and the compaction the turn
//! loop triggers. It can act only as the participants it represents and cannot reach Control's
//! operator surface — the structural absence of those methods is what makes "the operator has no
//! platform identity" enforceable (spec §Clients and the server boundary).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{Carryover, Instance, InstanceError, RoutedTurn};
use crate::{
    agent::{
        TurnError, TurnOutcome, TurnView, bounded_buffer_turns, carryover_start, session_touched,
    },
    event::PromptTemplateName,
    ids::{ConversationId, ConversationLocator, MemoryId, Seq},
    memory::{
        identity::{resolve_or_mint_conversation, resolve_or_mint_participant},
        memory_block::Authority,
    },
    model::ModelClient,
    settings::Settings,
};

/// Platform-authority operations: a client delivering participant turns. It can act only as the
/// participants it represents, and cannot reach Control's operator surface.
pub struct Platform<'a> {
    pub(super) server: &'a Instance,
}

/// The outcome of a roster resync ([`Platform::note_presence`]): the arrivals it briefed into the
/// live session, and how many prior members the new roster no longer lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterResync {
    /// The platform user ids that were newly present. Each received a `ParticipantJoined` and an
    /// injected join-brief, exactly as an explicit [`Platform::note_join`] would.
    pub joined: Vec<String>,
    /// The count of the session's prior members absent from the new roster. Departures are
    /// deliberately eventless (spec §Conversations and briefs → n is per-session): the per-message
    /// present set drives visibility, so a departed participant stops affecting retrieval on the
    /// next message with no event of its own, and membership drift is carried by each session's own
    /// present set. The count is reported for the connector's confirmation and for observability,
    /// never recorded.
    pub departed: usize,
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

        // A deferred turn skips the compaction check entirely: the model just proved unreachable,
        // so the pre-compaction flush could not run anyway, and the buffer gained no agent turn.
        if report.outcome == TurnOutcome::Deferred {
            return Ok(report.outcome);
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

    /// Resync a room's full roster — the counterpart to `note_join` for a connector that observes
    /// presence directly (a voice channel's member list, a presence event) rather than only through
    /// messages. Given every platform id currently present, this diffs against the live session's
    /// members: each arrival routes through the same join machinery as `note_join` — a
    /// `ParticipantJoined` and an injected join-brief built against the now-present set — while a
    /// departure is acknowledged but records no event, because the message-carried present set is
    /// what drives per-turn visibility and membership drift is carried by each session's own present
    /// set (spec §Conversations and briefs → n is per-session). A no-op returning an empty resync if
    /// the room has never been seen or has no live session; the next message then opens a session
    /// with the current roster present. `model`, when configured, feeds each arrival's describe
    /// catch-up before its brief composes, as `note_join` does.
    pub async fn note_presence(
        &self,
        model: Option<&dyn ModelClient>,
        locator: &ConversationLocator,
        roster: &[&str],
    ) -> Result<RosterResync, InstanceError> {
        let Some(conversation) = self
            .server
            .engine
            .graph
            .lock()
            .conversation_for_locator(locator)?
        else {
            return Ok(RosterResync::default());
        };
        let Some(session) = self.server.sessions.get(conversation).map(|open| open.id) else {
            return Ok(RosterResync::default());
        };

        // Resolve the roster to memory ids, deduplicating first: resolution reads the graph, which is
        // not re-materialized between mints within this pass, so the same id seen twice would
        // otherwise be minted twice.
        let platform = locator.platform.as_str();
        let mut uids: Vec<&str> = Vec::new();
        for uid in roster {
            if !uids.contains(uid) {
                uids.push(uid);
            }
        }
        let mut present_ids = Vec::with_capacity(uids.len());
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
            present_ids.push(id);
        }
        self.server
            .engine
            .graph
            .lock()
            .materialize_from(self.server.engine.store.lock().as_ref())?;

        // Diff against the session's members, read once. An id present but not a member is an arrival
        // to brief in; a member absent from the roster is a departure to acknowledge. Two distinct
        // platform ids can resolve to one memory (a merged cross-platform identity), so a joined-id
        // guard keeps a single arrival from being briefed twice within the pass.
        let members = self
            .server
            .engine
            .graph
            .lock()
            .session_participants(session)?;
        let mut joined = Vec::new();
        let mut joined_ids: Vec<MemoryId> = Vec::new();
        for (uid, &id) in uids.iter().zip(present_ids.iter()) {
            if !members.contains(&id) && !joined_ids.contains(&id) {
                self.server
                    .join_participant(model, conversation, session, id)
                    .await?;
                joined.push((*uid).to_owned());
                joined_ids.push(id);
            }
        }
        let departed = members
            .iter()
            .filter(|member| !present_ids.contains(member))
            .count();

        Ok(RosterResync { joined, departed })
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
    /// the session's touch-derived set — every memory ID it read or wrote, taken from the per-block
    /// `touched` sets on its `LuaExecuted` events. (The other source, the brief's recent facts, the
    /// brief adds itself.) Read after the flush, so its own reads and writes count too. Purely
    /// platform-derived: no agent-managed link flags on the semantic graph, which would strand stale
    /// "keep live" markers when a thread closes without an explicit clear.
    fn compaction_working_set(
        &self,
        conversation: ConversationId,
        from_seq: Seq,
    ) -> Result<Vec<MemoryId>, InstanceError> {
        let mut working_set = Vec::new();
        let mut seen = BTreeSet::new();
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
            produced_by: None,
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
