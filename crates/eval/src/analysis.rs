//! Reading a run's event log for assessment — the deterministic side of an oracle (spec §Validation →
//! assessment is a pure function of the log). Everything here is a query over `&[Event]`, so it works
//! identically on a live run and on a stored package being re-assessed.

use std::collections::{BTreeMap, BTreeSet};

use zuihitsu::{
    BEFORE_AFTER_EPSILON_MILLIS, ConversationId, EntryId, Event, EventPayload, Initiation,
    LinkSource, MemoryId, MemoryName, MergeProposalSource, Teller, TemporalRef, Timestamp, TurnId,
    TurnRole, Visibility, Volatility,
};

/// One durable content entry, projected from a `MemoryContentAppended` for assessment: which memory it
/// landed on, its text, the visibility it was written with, who it is attributed to (so an oracle can
/// catch a relayed fact re-recorded under the wrong teller), and the entry id (so an oracle can ignore
/// entries the agent later superseded).
pub struct EntryFacts {
    pub memory: String,
    pub text: String,
    pub visibility: Visibility,
    pub told_by: Teller,
    pub entry_id: EntryId,
}

/// Every agent reply to a participant, in order. Only `Responding` turns count: an `Initiated` agent
/// turn is the agent acting unprompted — the pre-compaction flush, a wake-up — self-directed bookkeeping
/// addressed to no one, not a reply. Under a tight compaction budget every turn trails a flush, so
/// including those would let a flush summary stand in for the agent's actual answer to a probe.
pub fn agent_replies(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::Agent,
                initiation: Initiation::Responding,
                text,
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The agent's last reply, if it spoke.
pub fn last_agent_reply(events: &[Event]) -> Option<&str> {
    agent_replies(events).into_iter().last()
}

/// Every agent reply paired with the `turn_id` its `run_lua` blocks share — a block commits (and
/// records its `LuaExecuted`) before the agent's reply turn, both stamped with the same `turn_id`, so
/// this ties a reply's claim to whether that same turn actually committed a write (see
/// [`turn_committed_write`]). Only `Responding` turns count, exactly as [`agent_replies`].
pub fn agent_replies_with_turn(events: &[Event]) -> Vec<(TurnId, &str)> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                turn_id,
                role: TurnRole::Agent,
                initiation: Initiation::Responding,
                text,
                ..
            } => Some((*turn_id, text.as_str())),
            _ => None,
        })
        .collect()
}

/// Whether any block belonging to `turn_id` committed a durable write — a `LuaExecuted` for that turn
/// whose `result` carries the `Committed:` summary the runtime folds in only when the block's buffer
/// actually landed events. Two outcomes read as `false`, and both are the honest signal that the turn's
/// reply may not claim a write: a block that crashed or aborted (its `terminal_cause` set and `result`
/// `None`, its writes rolled back with it), and one that ran clean but wrote nothing (a `result`
/// present with no `Committed:` line — a revise loop that matched nothing, or a read-only block). A turn
/// that never reached a committing block at all (a max-steps death) has no such `LuaExecuted` either, so
/// it too reads as `false`.
pub fn turn_committed_write(events: &[Event], turn_id: TurnId) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LuaExecuted { turn_id: id, result: Some(result), .. }
            if *id == turn_id && result.contains("Committed:")
        )
    })
}

/// Every Lua block the agent executed, in order.
pub fn lua_scripts(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::LuaExecuted { script, .. } => Some(script.as_str()),
            _ => None,
        })
        .collect()
}

/// Whether any executed Lua block *calls* `path` as a function — the name immediately followed by `(`
/// (whitespace allowed). Stricter than a bare substring search: a mention of the name in a comment, a
/// string, or a longer identifier does not count, so the oracle asserts the agent reached for the call
/// rather than merely that the text appeared.
pub fn lua_called(events: &[Event], path: &str) -> bool {
    lua_scripts(events)
        .iter()
        .any(|script| script_calls(script, path))
}

/// Whether the tag named `tag` was applied to any memory.
pub fn tag_applied(events: &[Event], tag: &str) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::TagAppliedToMemory { tag: applied, .. } if applied.as_str() == tag)
    })
}

/// Whether a link of the relation named `relation` was created — the structural signal that the agent
/// recorded a typed edge, not just any link or prose.
pub fn link_created_with(events: &[Event], relation: &str) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::LinkCreated { relation: created, .. } if created.as_str() == relation)
    })
}

/// Whether an inferred link of relation `relation` was created directed from `from` to `to` — the
/// structural signal that the link-inference pass extracted *this specific* relationship from content
/// (source is `Inferred`), on the right endpoints and the right way round. A caller with two
/// direction-equivalent candidates (`a mentored_by b` versus `b mentor_of a`) checks each, so a
/// semantically-equivalent coinage passes while a wrong relation, a wrong pair, or a reversed edge does
/// not.
pub fn link_inferred_directed(
    events: &[Event],
    from: MemoryId,
    to: MemoryId,
    relation: &str,
) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LinkCreated {
                from: created_from,
                to: created_to,
                relation: created,
                source: LinkSource::Inferred,
                ..
            }
            if *created_from == from && *created_to == to && created.as_str() == relation
        )
    })
}

/// Whether a relation type was registered under `name`, as either its forward name or its inverse — the
/// structural signal that the link-inference pass introduced this typed edge into the registry.
/// Registering a relation defines both its name and its inverse in one event (`mentored_by` with inverse
/// `mentor_of`, or the reverse), so the same relation is reachable under either name; matching on both
/// lets a caller name one direction and still recognize the pair coined the other way round.
pub fn relation_registered(events: &[Event], name: &str) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LinkTypeRegistered { name: registered, inverse, .. }
            if registered.as_str() == name || inverse.as_str() == name
        )
    })
}

/// Whether any executed block reached for a link reader — `mem:outgoing`, `mem:incoming`, or
/// `mem:links` — the structural signal that the agent traversed the relationship graph to answer,
/// rather than reconstructing the connections from prose. (`links` here is the `:links()` reader; the
/// `links.*` registry calls are `links.list`/`get`/`register`, which `script_calls` does not match on
/// the bare `links(`.)
pub fn link_reader_called(events: &[Event]) -> bool {
    lua_called(events, "outgoing") || lua_called(events, "incoming") || lua_called(events, "links")
}

/// Whether the agent renamed a memory's handle (a `MemoryRenamed`) — the structural signal that it
/// followed a name change by renaming the existing memory rather than creating a second one for the
/// same person (spec §Identity → Renaming).
pub fn memory_renamed(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(&event.payload, EventPayload::MemoryRenamed { .. }))
}

/// Whether the agent proposed a cross-platform merge (a `MergeProposed`) — the agent's judgment that
/// two stubs may be one person, before any adjudication.
pub fn merge_proposed(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(&event.payload, EventPayload::MergeProposed { .. }))
}

/// Whether the identity-resolution orchestration proposed a merge (a `MergeProposed` sourced
/// `Orchestration`) — the signal that a platform arrival's handle matched an existing but
/// platform-unbound stub, raising a candidate reunion for the operator rather than asserting identity.
pub fn orchestration_merge_proposed(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::MergeProposed {
                source: MergeProposalSource::Orchestration,
                ..
            }
        )
    })
}

/// Whether the run actually merged two stubs: an adjudication accepted *and* authored the `same_as`
/// (`source = Adjudicated`). This is the surfacing-changing outcome — distinct from merely proposing,
/// which is inert.
pub fn merge_committed(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LinkCreated { relation, source, .. }
                if relation.as_str() == "same_as" && source.as_str() == "Adjudicated"
        )
    })
}

/// Whether the run marked any memory `High` volatility — the agent classified a fast-changing memory
/// so its facts decay and can read as stale (spec §Recency and volatility).
pub fn volatility_set_high(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::MemoryVolatilitySet {
                volatility: Volatility::High,
                ..
            }
        )
    })
}

/// The durable conversation opened for `platform`/`scope`, if the run reached that room — so a
/// multi-room scenario can scope a query to one room's events.
pub fn conversation_id(events: &[Event], platform: &str, scope: &str) -> Option<ConversationId> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::ConversationStarted { id, locator, .. }
            if locator.platform.as_str() == platform && locator.scope_path.as_str() == scope =>
        {
            Some(*id)
        }
        _ => None,
    })
}

/// Every agent reply to a participant within one room (located by `platform`/`scope`), in order —
/// the per-room slice of [`agent_replies`], so a two-room scenario can probe each room's exposed
/// surface separately.
pub fn agent_replies_in<'a>(events: &'a [Event], platform: &str, scope: &str) -> Vec<&'a str> {
    let Some(conversation) = conversation_id(events, platform, scope) else {
        return Vec::new();
    };
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::ConversationTurn {
                conversation: turn_conversation,
                role: TurnRole::Agent,
                initiation: Initiation::Responding,
                text,
                ..
            } if *turn_conversation == conversation => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// How many sessions opened in the run — one more than the number of cuts (compaction or idle
/// re-segmentation), so `session_count - 1` counts the seams the carryover crossed.
pub fn session_count(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| matches!(&event.payload, EventPayload::SessionStarted { .. }))
        .count()
}

/// Whether a fired wake-up was raised into a session — the recurrence actually surfaced, not merely
/// got recorded.
pub fn scheduled_item_surfaced(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(&event.payload, EventPayload::ScheduledItemSurfaced { .. }))
}

/// The id of the memory created under the exact handle `name`, if the run created one — so an oracle
/// that needs a specific memory's endpoints can resolve it from the log rather than threading run-time
/// ids into the assessment.
pub fn memory_id_named(events: &[Event], name: &str) -> Option<MemoryId> {
    events.iter().find_map(|event| match &event.payload {
        EventPayload::MemoryCreated { id, name: created } if created.as_str() == name => Some(*id),
        _ => None,
    })
}

/// The id → name map of every memory created in the run.
pub fn memory_names(events: &[Event]) -> BTreeMap<MemoryId, String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemoryCreated { id, name } => Some((*id, name.as_str().to_owned())),
            _ => None,
        })
        .collect()
}

/// The names of memories that received a recurring occurrence.
pub fn recurring_memory_names(events: &[Event]) -> Vec<String> {
    let names = memory_names(events);
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemoryContentAppended {
                id,
                occurred_at: Some(TemporalRef::Recurring(_)),
                ..
            } => names.get(id).cloned(),
            _ => None,
        })
        .collect()
}

/// Every durable content entry written in the run, with the memory it landed on and its visibility.
pub fn entries(events: &[Event]) -> Vec<EntryFacts> {
    let names = memory_names(events);
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemoryContentAppended {
                id,
                entry_id,
                text,
                visibility,
                told_by,
                ..
            } => Some(EntryFacts {
                memory: names.get(id).cloned().unwrap_or_default(),
                text: text.clone(),
                visibility: visibility.clone(),
                told_by: told_by.clone(),
                entry_id: *entry_id,
            }),
            _ => None,
        })
        .collect()
}

/// The set of entry ids that have been superseded, from `MemorySuperseded` events.
pub fn superseded_entry_ids(events: &[Event]) -> BTreeSet<EntryId> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::MemorySuperseded { entry, .. } => Some(*entry),
            _ => None,
        })
        .collect()
}

/// How many distinct conversation participants are credited as the teller of a non-Public entry on a
/// memory whose name contains `subject`. A confidence about `subject` should be held under exactly its
/// one original teller; a count above one means it was re-recorded under a second teller — typically
/// the current speaker when the agent redundantly re-wrote a fact it already held — which silently
/// re-keys whose private note it is and can later surface the confidence to the wrong person.
pub fn private_tellers_of(events: &[Event], subject: &str) -> usize {
    let subject = subject.to_lowercase();
    let mut tellers = BTreeSet::new();
    for entry in entries(events) {
        if entry.visibility != Visibility::Public
            && entry.memory.to_lowercase().contains(&subject)
            && let Teller::Participant(id) = entry.told_by
        {
            tellers.insert(id);
        }
    }
    tellers.len()
}

/// Each memory's latest synthesized description (the always-visible summary), as `(name, text)`. A
/// later regeneration supersedes an earlier one, so only the last per memory is kept.
pub fn descriptions(events: &[Event]) -> Vec<(String, String)> {
    let names = memory_names(events);
    let mut latest: BTreeMap<MemoryId, String> = BTreeMap::new();
    for event in events {
        if let EventPayload::MemoryDescriptionRegenerated { id, new_text, .. } = &event.payload {
            latest.insert(*id, new_text.clone());
        }
    }
    latest
        .into_iter()
        .map(|(id, text)| (names.get(&id).cloned().unwrap_or_default(), text))
        .collect()
}

/// The names of every memory minted in the run whose name begins with `prefix` (e.g. `event/`), for
/// asserting the agent reused an existing memory rather than creating a duplicate of the same entity.
pub fn memories_in_namespace(events: &[Event], prefix: &str) -> Vec<String> {
    memory_names(events)
        .into_values()
        .filter(|name| name.starts_with(prefix))
        .collect()
}

/// Whether the run superseded any entry — the structured "this supersedes that" move that records an
/// explicit correction or update in state, rather than leaving the stale value standing or only saying
/// so in a reply. The signal that a correction landed durably (distinct from a genuine contradiction,
/// where both accounts are kept and the synthesis arbitrates instead).
pub fn any_superseded(events: &[Event]) -> bool {
    events
        .iter()
        .any(|event| matches!(event.payload, EventPayload::MemorySuperseded { .. }))
}

/// The reconciling statements of every belief arbitration the run recorded.
pub fn arbitrations(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::BeliefArbitrated { resolution, .. } => Some(resolution.statement.clone()),
            _ => None,
        })
        .collect()
}

/// Whether the run recorded a *both-stand* arbitration: one that credits neither competing entry
/// (`credited` empty), keeping both accounts standing rather than resolving the contradiction to one
/// side. This is the faithful signal for a genuine unresolved conflict — distinct from an arbitration
/// that credits a side, which is supersession by another name (one account chosen over the other).
pub fn both_stand_arbitration(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::BeliefArbitrated { resolution, .. } if resolution.credited.is_empty()
        )
    })
}

/// Whether the run recorded any recurring occurrence — emitted inline on a content append or resolved
/// by the turn-end temporal extraction. Either path proves the model produced a `Recurring` reference
/// rather than flattening "every Tuesday" to a single day.
pub fn has_recurring_occurrence(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::MemoryContentAppended {
                occurred_at: Some(TemporalRef::Recurring(_)),
                ..
            } | EventPayload::EntryTemporalResolved {
                occurred_at: TemporalRef::Recurring(_),
                ..
            }
        )
    })
}

/// One content entry's full temporal picture: its `MemoryContentAppended` (the text, the memory it
/// landed on, and when it was asserted, plus any occurrence stamped *at append* — the authored slot)
/// joined with any later `EntryTemporalResolved` (the occurrence the turn-end extraction pass resolved
/// — the extracted slot). The two slots let an oracle tell an authored date from an extracted one on the
/// same entry: the distinction the search-hit and neighborhood-line projections lean on when they prefer
/// authored over extracted, and the one a temporal-honesty oracle checks for a fabricated resolution.
pub struct EntryOccurrence {
    pub memory: String,
    pub text: String,
    pub asserted_at: Timestamp,
    /// The occurrence the agent stamped at append time; `None` when it wrote the entry untimed.
    pub authored: Option<TemporalRef>,
    /// The occurrence the turn-end extraction pass resolved later; `None` when it left the entry
    /// unextracted (or could not parse the model's string).
    pub extracted: Option<TemporalRef>,
}

/// Every content entry's temporal picture, in append order — each `MemoryContentAppended` joined with
/// any later `EntryTemporalResolved` on the same entry (see [`EntryOccurrence`]).
pub fn entry_occurrences(events: &[Event]) -> Vec<EntryOccurrence> {
    let names = memory_names(events);
    let mut occurrences: Vec<EntryOccurrence> = Vec::new();
    let mut index: BTreeMap<EntryId, usize> = BTreeMap::new();
    for event in events {
        match &event.payload {
            EventPayload::MemoryContentAppended {
                id,
                entry_id,
                asserted_at,
                occurred_at,
                text,
                ..
            } => {
                index.insert(*entry_id, occurrences.len());
                occurrences.push(EntryOccurrence {
                    memory: names.get(id).cloned().unwrap_or_default(),
                    text: text.clone(),
                    asserted_at: *asserted_at,
                    authored: occurred_at.clone(),
                    extracted: None,
                });
            }
            EventPayload::EntryTemporalResolved {
                entry_id,
                occurred_at,
                ..
            } => {
                if let Some(&position) = index.get(entry_id) {
                    occurrences[position].extracted = Some(occurred_at.clone());
                }
            }
            _ => {}
        }
    }
    occurrences
}

/// Whether a temporal reference pins a fixed instant — an `Instant`, `Day`, `Range`, or `Approx`, all of
/// which denormalize to a representative sort instant. A `BeforeAfter` (relative to another memory) and a
/// `Recurring` rule have no fixed instant of their own, so they read as *not* concrete — which is the
/// honest-anchoring outcome for a phrase that names another event rather than the speaker's now. The
/// anchor is resolved with `None`, so a `BeforeAfter` stays instant-less here rather than borrowing one.
pub fn resolves_to_instant(occurred_at: &TemporalRef) -> bool {
    occurred_at
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort
        .is_some()
}

/// Whether a temporal reference resolves to a concrete instant within `window_ms` of `anchor_ms` — the
/// structural signal of a clock-anchored resolution when `anchor_ms` is the conversation's own now. A
/// `BeforeAfter` or `Recurring` reference has no fixed instant (see [`resolves_to_instant`]), so it is
/// never "near" anything: exactly the honest outcome an oracle wants to let pass.
pub fn resolves_near(occurred_at: &TemporalRef, anchor_ms: i64, window_ms: i64) -> bool {
    occurred_at
        .bounds(None, BEFORE_AFTER_EPSILON_MILLIS)
        .sort
        .is_some_and(|sort| (sort.as_millis() - anchor_ms).abs() <= window_ms)
}

/// The anchor memory a `BeforeAfter` reference names, if `occurred_at` is one — the honest resolution for
/// a phrase anchored to another event rather than to the speaker's now (spec §Time → the anchor rule).
pub fn before_after_anchor(occurred_at: &TemporalRef) -> Option<&MemoryName> {
    match occurred_at {
        TemporalRef::BeforeAfter { anchor, .. } => Some(anchor),
        _ => None,
    }
}

/// A crude lexical leak backstop: the subject term co-occurring with any of `terms` in the text. A dumb
/// floor under the judge — an obvious leak can't slip a judge hiccup — never the primary signal.
pub fn lexical_leak(text: &str, subject: &str, terms: &[&str]) -> bool {
    let lower = text.to_lowercase();
    lower.contains(&subject.to_lowercase()) && terms.iter().any(|term| lower.contains(term))
}

/// Whether `script` calls `path`: an occurrence of `path` immediately followed (whitespace aside) by an
/// opening parenthesis. Not a full parse — it does not exclude occurrences inside strings or comments —
/// but it distinguishes a call from an incidental mention, which is what the oracles need.
fn script_calls(script: &str, path: &str) -> bool {
    let mut from = 0;
    while let Some(found) = script[from..].find(path) {
        let after = from + found + path.len();
        if script[after..].trim_start().starts_with('(') {
            return true;
        }
        from = after;
    }
    false
}
