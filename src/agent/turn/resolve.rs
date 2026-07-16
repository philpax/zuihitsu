//! The `convo.turn` transcript resolver: resolve a turn id to that moment plus a window of
//! surrounding turns, under the audience rule (spec §Transcripts).

use crate::agent::turn::*;

/// One conversation turn resolved for the `convo.turn` transcript link resolver (spec §Transcripts):
/// its stable id, who spoke, its role, its text, when it was recorded, and a ready-made canonical
/// reference to cite it by. `speaker` is the participant's conversational display name for a
/// participant turn, `self` for the agent's own turn, and `system` for an injected system turn.
/// `reference` is the `[turn:<ulid>]` token ([`turn_ref::construct`]) — the agent copies it to cite
/// the moment, so citation syntax lives in one place, never hand-assembled from the id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTurn {
    pub turn_id: TurnId,
    pub role: TurnRole,
    pub speaker: String,
    pub text: String,
    pub recorded_at: Timestamp,
    pub reference: String,
}

/// A resolved turn together with a small window of the turns immediately around it within its own
/// session, in chronological order. `focus` indexes the requested turn within `turns`. The window is
/// clamped to the focal turn's session and each neighbor is filtered by the same audience rule as the
/// focal turn, so a mid-session join never widens the window past what the current present set shared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnWindow {
    pub turns: Vec<ResolvedTurn>,
    pub focus: usize,
}

/// The outcome of resolving a turn reference, with its two refusals deliberately distinct (spec
/// §Transcripts → the two refusal tiers).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnResolution {
    /// The reference names a turn the current present set was all party to: the moment and its window.
    Resolved(TurnWindow),
    /// The reference names a real turn, but the audience rule fails — someone present here was not in
    /// that moment's audience. Refused without the content or the source room, so the agent recalls
    /// through the visibility-filtered memory channel instead of replaying a transcript to a new
    /// audience.
    AudienceMismatch,
    /// The reference names no turn anywhere in the log — an unknown or (having been parsed) a
    /// well-formed but never-recorded id.
    NotFound,
}

/// Resolve a turn id to that moment plus a window of `before`/`after` surrounding turns, under one
/// audience rule for every conversation: the turn resolves iff **every member of `present_set` was in
/// that turn's audience** (spec §Transcripts → the audience rule). A turn's audience is derived from
/// the log — its session's `SessionStarted.participants` plus every `ParticipantJoined` for that
/// session up to the turn's seq — so the check is "was everyone here also there then."
///
/// This is a deliberate loosening from the v1 same-room scope: a solo DM (present is just the
/// requester) resolves any turn the requester attended in any room, and a two-person DM resolves turns
/// whose audience included both. The distinction between [`TurnResolution::AudienceMismatch`] and
/// [`TurnResolution::NotFound`] confirms, on a cross-room id, that the id maps to *something* — which
/// is safe because ULIDs are unguessable: holding one means you were there, or someone who was there
/// gave you the link.
///
/// The whole log is read — turns are event-sourced, not materialized in the graph, so a store scan is
/// the read shape available. No content visibility filtering is applied within the window: it is a
/// transcript replay of a moment the whole current present set was party to (that is exactly what the
/// audience rule enforces), so it opens no new visibility surface (spec §Visibility). System turns
/// (join briefs, drained wake-ups) resolve too — they were injected into that session and read there.
pub fn resolve_turn(
    engine: &Engine,
    present_set: &[MemoryId],
    turn_id: TurnId,
    before: usize,
    after: usize,
) -> Result<TurnResolution, StoreError> {
    // Read every turn's audience off the store lock, then resolve speakers off the graph lock — the
    // two locks are taken in sequence, never held together, so this read observes the graph-before-
    // store ordering without violating it.
    let turns = {
        let store = engine.store.lock();
        audience_turns(store.as_ref())?
    };
    let Some(focus) = turns.iter().find(|turn| turn.view.turn_id == turn_id) else {
        return Ok(TurnResolution::NotFound);
    };
    // Resolve every id the audience rule will weigh — the present set and every turn's audience — to
    // its `same_as` class, so the rule compares identities rather than raw stubs. An operator-asserted
    // cross-platform merge (`person/maya@direct` same_as `person/maya@discord`) then reads as one
    // person: the merged identity resolves a turn recorded under either stub, consistent with how
    // class-wide reads (`class_entries`) already treat a merge (spec §Visibility). Built once off a
    // single graph lock, taken after the store lock above is released (graph-before-store ordering is
    // for holding both at once, not for taking them in sequence).
    let class_of = class_map(engine, present_set, &turns);
    if !audience_admits(present_set, &focus.audience, &class_of) {
        // The id maps to a real turn, but not one everyone present shared — a distinct, teachable
        // refusal that (safely, ULIDs being unguessable) confirms existence.
        return Ok(TurnResolution::AudienceMismatch);
    }
    // Clamp the window to the focal turn's own session (a session belongs to one conversation, so its
    // turns are the transcript neighbors), then keep only neighbors the present set also shared — a
    // mid-session join changes the audience within a session, so an earlier neighbor may fail the rule
    // the focal turn passed.
    let session_turns: Vec<&AudienceTurn> = turns
        .iter()
        .filter(|turn| turn.conversation == focus.conversation && turn.session == focus.session)
        .collect();
    let focus_idx = session_turns
        .iter()
        .position(|turn| turn.view.turn_id == turn_id)
        .expect("the focal turn is in its own session");
    let start = focus_idx.saturating_sub(before);
    let end = focus_idx
        .saturating_add(after)
        .saturating_add(1)
        .min(session_turns.len());
    let admitted: Vec<&AudienceTurn> = session_turns[start..end]
        .iter()
        .copied()
        .filter(|turn| audience_admits(present_set, &turn.audience, &class_of))
        .collect();
    let focus_position = admitted
        .iter()
        .position(|turn| turn.view.turn_id == turn_id)
        .expect("the focal turn passed the audience rule, so it survives the filter");
    let views: Vec<TurnView> = admitted.iter().map(|turn| turn.view.clone()).collect();
    let names = participant_names(engine, &views, &[]);
    let resolved = views
        .iter()
        .map(|turn| ResolvedTurn {
            turn_id: turn.turn_id,
            role: turn.role,
            speaker: turn_speaker(turn, &names),
            text: turn.text.clone(),
            recorded_at: turn.recorded_at,
            reference: turn_ref::construct(turn.turn_id),
        })
        .collect();
    Ok(TurnResolution::Resolved(TurnWindow {
        turns: resolved,
        focus: focus_position,
    }))
}

/// One conversation turn read for the resolver, carrying enough to apply the audience rule: the turn's
/// [`TurnView`] fields, which conversation and session it belongs to, and the audience that had
/// accumulated by its seq (its session's opening participants plus every mid-session joiner up to it).
struct AudienceTurn {
    view: TurnView,
    conversation: ConversationId,
    /// The session the turn was recorded in, or `None` for a turn outside any open session (which
    /// should not arise in normal operation, but is handled so a stray turn resolves only to itself).
    session: Option<SessionId>,
    audience: Vec<MemoryId>,
}

/// Whether every member of `present_set` is in `audience` — the audience rule, compared by `same_as`
/// class rather than by raw id. Each side is mapped through `class_of` (a member with no class in the
/// graph stands for itself), so a present member is admitted when any of its class siblings was in the
/// audience. This makes an operator-merged cross-platform identity one person for transcript
/// resolution, matching how class-wide reads treat a merge. An empty present set is vacuously admitted
/// (there is no one to have been excluded).
fn audience_admits(
    present_set: &[MemoryId],
    audience: &[MemoryId],
    class_of: &HashMap<MemoryId, MemoryId>,
) -> bool {
    let class = |id: &MemoryId| class_of.get(id).copied().unwrap_or(*id);
    let audience_classes: BTreeSet<MemoryId> = audience.iter().map(class).collect();
    present_set
        .iter()
        .all(|member| audience_classes.contains(&class(member)))
}

/// Map every id the audience rule weighs — the present set and every turn's audience — to its
/// `same_as`-class id, off a single graph lock. A member whose class cannot be read (unknown, soft-
/// deleted, or a graph error) is simply absent, so [`audience_admits`] falls back to its raw id — the
/// strict pre-merge behavior, never a looser one. The lock is taken after the resolver's store read is
/// released, so it never holds the store and graph guards together.
fn class_map(
    engine: &Engine,
    present_set: &[MemoryId],
    turns: &[AudienceTurn],
) -> HashMap<MemoryId, MemoryId> {
    let graph = engine.graph.lock();
    let mut class_of = HashMap::new();
    for id in present_set
        .iter()
        .copied()
        .chain(turns.iter().flat_map(|turn| turn.audience.iter().copied()))
    {
        if let std::collections::hash_map::Entry::Vacant(entry) = class_of.entry(id)
            && let Ok(Some(class)) = graph.class_id(id)
        {
            entry.insert(class);
        }
    }
    class_of
}

/// Every `ConversationTurn` in the log, each tagged with the audience in effect at its seq. One
/// forward pass tracks the open session per conversation and the audience accumulating within it
/// (`SessionStarted.participants`, then each `ParticipantJoined`), so a turn's audience is the set as
/// of the moment it was recorded — which is what lets the window filter mid-session joins correctly.
fn audience_turns(store: &dyn Store) -> Result<Vec<AudienceTurn>, StoreError> {
    let mut open: HashMap<ConversationId, (SessionId, Vec<MemoryId>)> = HashMap::new();
    let mut turns = Vec::new();
    for event in store.read_from(Seq::ZERO)? {
        match event.payload {
            EventPayload::SessionStarted {
                conversation,
                id,
                participants,
                ..
            } => {
                open.insert(conversation, (id, participants));
            }
            EventPayload::ParticipantJoined {
                conversation,
                session,
                participant,
                ..
            } => {
                if let Some((open_id, audience)) = open.get_mut(&conversation)
                    && *open_id == session
                    && !audience.contains(&participant)
                {
                    audience.push(participant);
                }
            }
            EventPayload::SessionEnded { conversation, id }
                if open
                    .get(&conversation)
                    .is_some_and(|(open_id, _)| *open_id == id) =>
            {
                open.remove(&conversation);
            }
            EventPayload::ConversationTurn {
                conversation,
                turn_id,
                role,
                text,
                participant,
                produced_by,
                ..
            } => {
                let (session, audience) = match open.get(&conversation) {
                    Some((id, audience)) => (Some(*id), audience.clone()),
                    None => (None, Vec::new()),
                };
                turns.push(AudienceTurn {
                    view: TurnView {
                        seq: event.seq,
                        turn_id,
                        role,
                        text,
                        participant,
                        recorded_at: event.recorded_at,
                        steps: Vec::new(),
                        produced_by,
                    },
                    conversation,
                    session,
                    audience,
                });
            }
            _ => {}
        }
    }
    Ok(turns)
}

/// The conversational display name for a resolved turn: the participant's handle for a participant
/// turn (falling back to `someone` when it is not in the graph, matching [`participant_names`]),
/// `self` for the agent's own turn, and `system` for an injected system turn.
fn turn_speaker(turn: &TurnView, names: &BTreeMap<MemoryId, String>) -> String {
    match turn.role {
        TurnRole::Participant => turn
            .participant
            .and_then(|id| names.get(&id))
            .cloned()
            .unwrap_or_else(|| "someone".to_owned()),
        TurnRole::Agent => MemoryName::SELF.to_owned(),
        TurnRole::System => "system".to_owned(),
    }
}
