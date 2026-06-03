//! The agent server: the single writer that owns the event log, the materialized graph, and the
//! clock, and exposes its API split by client authority (spec §Clients and the server boundary).
//!
//! Authority is a property of the client's role, enforced here — never of where the client runs.
//! The operator-authority surface is [`Control`] (agent creation and read-only inspection; its
//! writes are authored as source `Debugger`). The platform-authority surface — delivering
//! participant turns via `route_message` — arrives with the agent loop in Stage 4 as a sibling
//! facet that structurally lacks Control's creation and inspection methods, which is what makes
//! "the operator has no platform identity" enforceable.

#[cfg(feature = "lua")]
use crate::{
    agent::{
        Flush, Turn, TurnError, TurnOutcome, TurnView, buffer_turns, run_flush, run_turn,
        session_touched,
    },
    brief::{self, BriefError},
    event::{Initiation, TurnRole},
    identity::{IdentityError, resolve_or_mint_conversation, resolve_or_mint_participant},
    ids::{ConversationId, MemoryId, Seq, SessionId, TurnId},
    lua::Session,
    model::ModelClient,
};
use crate::{
    clock::Clock,
    event::{EventPayload, EventSource},
    genesis::{self, GenesisStatus, Rollout, SeedSelf},
    graph::{EntryView, Graph, GraphError, MemoryView, SessionView},
    ids::ConversationLocator,
    settings::Settings,
    store::{MemoryStore, Store, StoreError},
};
#[cfg(feature = "lua")]
use std::collections::{BTreeSet, HashMap};

pub struct Server {
    store: Box<dyn Store>,
    graph: Graph,
    clock: Box<dyn Clock>,
    /// The live session per conversation: its id, the VM whose globals persist across the session's
    /// turns, the frozen brief, and the last-activity time the idle-gap is measured from. Pure
    /// runtime state — never logged (the `SessionStarted` / `SessionEnded` events are); an agent
    /// restart drops it and the next message opens a fresh session.
    #[cfg(feature = "lua")]
    sessions: HashMap<ConversationId, OpenSession>,
    /// Carryover staged by a token-triggered compaction, consumed by the next `ensure_session` to
    /// seed the re-segmented session (spec §Compaction). Keyed by conversation; an entry lives only
    /// between the compacting turn and the next message in that room.
    #[cfg(feature = "lua")]
    pending_carryover: HashMap<ConversationId, Carryover>,
}

impl Server {
    pub fn new(store: Box<dyn Store>, graph: Graph, clock: Box<dyn Clock>) -> Server {
        Server {
            store,
            graph,
            clock,
            #[cfg(feature = "lua")]
            sessions: HashMap::new(),
            #[cfg(feature = "lua")]
            pending_carryover: HashMap::new(),
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

    /// The platform-authority API facet — delivering participant turns. It structurally lacks
    /// Control's creation and inspection methods, which is what makes "the operator has no platform
    /// identity" enforceable.
    #[cfg(feature = "lua")]
    pub fn platform(&mut self) -> Platform<'_> {
        Platform { server: self }
    }
}

/// The raw-transcript carryover a compaction stages for the next session (spec §Compaction →
/// raw-transcript carryover). The oldest carried turn is both the `seeded_from_turn` boundary
/// recorded on the new `SessionStarted` and the `from_seq` the new session's buffer is read from, so
/// the carried tail plus the new turns reconstruct the post-cut buffer.
#[cfg(feature = "lua")]
struct Carryover {
    seeded_from_turn: TurnId,
    from_seq: Seq,
    /// The memories the ending session touched (read or wrote), re-surfaced in the new session's
    /// brief as active threads — the touch-derived working set (spec §Compaction → working-set
    /// carryover).
    working_set: Vec<MemoryId>,
}

/// The live session backing a conversation (runtime state, see [`Server::sessions`]).
#[cfg(feature = "lua")]
struct OpenSession {
    id: SessionId,
    vm: Session,
    brief: String,
    last_activity: crate::ids::Timestamp,
    /// The log seq the live buffer is read from: the `SessionStarted` seq for a fresh or idle-opened
    /// session, or a carried tail's seq across a compaction seam (so the carryover plus this
    /// session's turns reconstruct the buffer — see [`buffer_turns`]).
    start_seq: Seq,
}

/// Platform-authority operations: a client delivering participant turns. It can act only as the
/// participants it represents, and cannot reach Control's operator surface.
#[cfg(feature = "lua")]
pub struct Platform<'a> {
    server: &'a mut Server,
}

#[cfg(feature = "lua")]
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
        let conversation = resolve_or_mint_conversation(
            self.server.store.as_mut(),
            self.server.clock.as_ref(),
            &self.server.graph,
            locator,
        )?;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;

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
            let id = resolve_or_mint_participant(
                self.server.store.as_mut(),
                self.server.clock.as_ref(),
                &self.server.graph,
                platform,
                uid,
            )?;
            if *uid == sender {
                sender_id = Some(id);
            }
            present_set.push(id);
        }
        let sender_id = sender_id.expect("the sender is among the resolved ids");
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;

        // Open or continue the session, freezing a brief at each open.
        self.ensure_session(conversation, &present_set)?;

        let max_steps = Settings::from_store(self.server.store.as_ref())?
            .turn
            .max_steps as usize;
        let open = self
            .server
            .sessions
            .get(&conversation)
            .expect("ensure_session left an open session");
        // The live buffer the model sees as the prompt suffix: the session's prior turns (or, across
        // a compaction seam, the carried tail plus this session's turns), read from `start_seq`.
        let buffer = buffer_turns(self.server.store.as_ref(), conversation, open.start_seq)?;
        let report = run_turn(Turn {
            session: &open.vm,
            model,
            store: self.server.store.as_mut(),
            graph: &mut self.server.graph,
            clock: self.server.clock.as_ref(),
            inbound: text,
            inbound_participant: sender_id,
            brief: &open.brief,
            buffer: &buffer,
            max_steps,
        })
        .await?;

        // Token-triggered compaction: if the turn's peak prompt crossed the budget, end the session
        // now so the next message re-segments with a fresh brief and a carried tail (spec
        // §Compaction). The estimate fallback keeps the trigger meaningful when the backend reports
        // no usage (the in-memory and no-openai builds).
        let token_budget = Settings::from_store(self.server.store.as_ref())?
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

    /// Ensure a live session for `conversation`: reuse the open one if activity is within the
    /// idle-gap, otherwise end it (if any) and open a new one — composing and freezing its brief and
    /// minting a fresh VM. The session boundary is recorded (`SessionStarted` / `SessionEnded`) and
    /// not recomputed at replay.
    fn ensure_session(
        &mut self,
        conversation: ConversationId,
        present_set: &[MemoryId],
    ) -> Result<(), ServerError> {
        let now = self.server.clock.now();
        let settings = Settings::from_store(self.server.store.as_ref())?;
        let idle_gap_ms = settings.compaction.idle_gap_seconds.saturating_mul(1_000);

        let reuse =
            self.server.sessions.get(&conversation).is_some_and(|open| {
                now.as_millis() - open.last_activity.as_millis() <= idle_gap_ms
            });
        if reuse {
            if let Some(open) = self.server.sessions.get_mut(&conversation) {
                open.last_activity = now;
            }
            return Ok(());
        }

        // A lapsed session ends before the new one opens.
        if let Some(old) = self.server.sessions.remove(&conversation) {
            self.server.store.append(
                now,
                vec![EventPayload::SessionEnded {
                    conversation,
                    id: old.id,
                }],
            )?;
        }

        // A pending carryover from a just-compacted session seeds the new one: the next buffer read
        // starts at the carried tail (not this `SessionStarted`), the boundary is recorded as
        // `seeded_from_turn` for faithful replay, and the touch-derived working set augments the new
        // brief as active threads (spec §Compaction → carryover).
        let carryover = self.server.pending_carryover.remove(&conversation);
        let seeded_from_turn = carryover.as_ref().map(|carry| carry.seeded_from_turn);
        let working_set: &[MemoryId] = carryover
            .as_ref()
            .map_or(&[], |carry| carry.working_set.as_slice());

        let context = self.server.graph.context_for_conversation(conversation)?;
        let brief = brief::compose(
            &self.server.graph,
            present_set,
            context,
            &settings.brief,
            working_set,
        )?;
        let id = SessionId::generate();
        let committed = self.server.store.append(
            now,
            vec![EventPayload::SessionStarted {
                conversation,
                id,
                participants: present_set.to_vec(),
                started_at: now,
                seeded_from_turn,
                brief: brief.clone(),
            }],
        )?;
        let session_start_seq = committed[0].seq;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;
        self.server.sessions.insert(
            conversation,
            OpenSession {
                id,
                vm: Session::new(conversation),
                brief,
                last_activity: now,
                start_seq: carryover
                    .map(|carry| carry.from_seq)
                    .unwrap_or(session_start_seq),
            },
        );
        Ok(())
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
        let Some(conversation) = self.server.graph.conversation_for_locator(locator)? else {
            return Ok(());
        };
        let Some(session) = self.server.sessions.get(&conversation).map(|open| open.id) else {
            return Ok(());
        };

        let joiner = resolve_or_mint_participant(
            self.server.store.as_mut(),
            self.server.clock.as_ref(),
            &self.server.graph,
            locator.platform.as_str(),
            participant,
        )?;
        self.server
            .graph
            .materialize_from(self.server.store.as_ref())?;

        // The brief is filtered against the present set including the joiner, so the subject-guard
        // fires for asides about them.
        let mut present_set = self.server.graph.session_participants(session)?;
        if !present_set.contains(&joiner) {
            present_set.push(joiner);
        }
        let join_brief = brief::compose_participant(
            &self.server.graph,
            joiner,
            &present_set,
            &Settings::from_store(self.server.store.as_ref())?.brief,
        )?;

        let now = self.server.clock.now();
        let turn_id = TurnId::generate();
        self.server.store.append(
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
            .graph
            .materialize_from(self.server.store.as_ref())?;
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
        let settings = Settings::from_store(self.server.store.as_ref())?;
        // The buffer includes the turn that just crossed the budget; it is both the flush's context
        // and the source of the carried tail.
        let buffer = buffer_turns(self.server.store.as_ref(), conversation, open.start_seq)?;

        // Budget-gated pre-compaction flush: a substantive session gets one turn to write durable
        // working state to memory before the cut; a low-activity one (below the turn threshold) is
        // skipped, so the hot-path model call is paid only when there is something to flush.
        let flushed = buffer.len() as i64 >= settings.compaction.flush_min_turns;
        if flushed {
            run_flush(Flush {
                session: &open.vm,
                model,
                store: self.server.store.as_mut(),
                graph: &mut self.server.graph,
                clock: self.server.clock.as_ref(),
                brief: &open.brief,
                buffer: &buffer,
                max_steps: settings.turn.max_steps as usize,
            })
            .await?;
            self.server
                .graph
                .materialize_from(self.server.store.as_ref())?;
        }

        let now = self.server.clock.now();
        self.server.store.append(
            now,
            vec![EventPayload::SessionEnded {
                conversation,
                id: open.id,
            }],
        )?;

        // Re-read the buffer (now including any flush turn) for the carried tail, and assemble the
        // working set (likewise after the flush, so its writes and active_in flags are included).
        let buffer = buffer_turns(self.server.store.as_ref(), conversation, open.start_seq)?;
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
        if let Some(context) = self.server.graph.context_for_conversation(conversation)? {
            for memory in self.server.graph.outgoing(context, "has_active")? {
                if seen.insert(memory.id) {
                    working_set.push(memory.id);
                }
            }
        }
        for id in session_touched(self.server.store.as_ref(), conversation, from_seq)? {
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
#[cfg(feature = "lua")]
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
#[cfg(feature = "lua")]
fn estimate_tokens(buffer: &[TurnView], inbound: &str) -> i64 {
    let chars: usize = buffer
        .iter()
        .map(|turn| turn.text.chars().count())
        .sum::<usize>()
        + inbound.chars().count();
    (chars / 4) as i64
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

    /// Inspect the live memories in a namespace (e.g. `"person/"`), ordered by name.
    pub fn memories(&self, prefix: &str) -> Result<Vec<MemoryView>, ServerError> {
        Ok(self.server.graph.memories_in_namespace(prefix)?)
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

/// A server-side failure, delegating its message to the underlying error.
#[derive(Debug)]
pub enum ServerError {
    Store(StoreError),
    Graph(GraphError),
    /// A turn (the agent loop) failed while routing a message.
    #[cfg(feature = "lua")]
    Turn(TurnError),
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Store(error) => write!(f, "server (store): {error}"),
            ServerError::Graph(error) => write!(f, "server (graph): {error}"),
            #[cfg(feature = "lua")]
            ServerError::Turn(error) => write!(f, "server (turn): {error}"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServerError::Store(error) => Some(error),
            ServerError::Graph(error) => Some(error),
            #[cfg(feature = "lua")]
            ServerError::Turn(error) => Some(error),
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

// Identity and brief resolution fail only into store/graph errors, so they map onto the existing
// variants rather than widening the enum; the agent loop's richer `TurnError` keeps its own.
#[cfg(feature = "lua")]
impl From<IdentityError> for ServerError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::Store(error) => ServerError::Store(error),
            IdentityError::Graph(error) => ServerError::Graph(error),
        }
    }
}

#[cfg(feature = "lua")]
impl From<BriefError> for ServerError {
    fn from(error: BriefError) -> Self {
        match error {
            BriefError::Graph(error) => ServerError::Graph(error),
        }
    }
}

#[cfg(feature = "lua")]
impl From<TurnError> for ServerError {
    fn from(error: TurnError) -> Self {
        ServerError::Turn(error)
    }
}

#[cfg(all(test, feature = "lua"))]
mod tests {
    use super::*;
    use crate::ids::TurnId;

    fn turn(seq: u64, text: &str) -> TurnView {
        TurnView {
            seq: Seq(seq),
            turn_id: TurnId::generate(),
            role: TurnRole::Participant,
            text: text.to_owned(),
            participant: None,
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
