//! Reading a run's event log for assessment — the deterministic side of an oracle (spec §Validation →
//! assessment is a pure function of the log). Everything here is a query over `&[Event]`, so it works
//! identically on a live run and on a stored package being re-assessed.

mod events;
mod state;

use std::collections::BTreeSet;

use zuihitsu::{
    Event, EventPayload, Initiation, LinkSource, MemoryId, MergeProposalSource, Teller, TurnId,
    TurnRole, Visibility, Volatility,
};

pub use events::*;
use state::script_calls;
pub use state::{
    EntryOccurrence, before_after_anchor, entry_occurrences, lexical_leak, resolves_near,
    resolves_to_instant,
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
    pub entry_id: zuihitsu::EntryId,
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

/// The last agent reply to a participant, or `None` if the agent never replied.
pub fn last_agent_reply(events: &[Event]) -> Option<&str> {
    agent_replies(events).last().copied()
}

/// Every agent reply paired with the inbound message that prompted it — the `(turn_id, inbound, reply)` triples an
/// oracle needs when it assesses whether the agent's answer addresses the specific question asked. The
/// inbound message is the most recent participant turn before the agent's reply; if there is none
/// (the agent spoke first), the inbound is an empty string.
pub fn agent_replies_with_inbound(events: &[Event]) -> Vec<(TurnId, &str, &str)> {
    let mut inbound = "";
    let mut replies = Vec::new();
    for event in events {
        match &event.payload {
            EventPayload::ConversationTurn {
                role: TurnRole::Participant,
                text,
                ..
            } => inbound = text.as_str(),
            EventPayload::ConversationTurn {
                turn_id,
                role: TurnRole::Agent,
                initiation: Initiation::Responding,
                text,
                ..
            } => replies.push((*turn_id, inbound, text.as_str())),
            _ => {}
        }
    }
    replies
}

/// Whether the turn with `turn_id` actually landed events. Two outcomes read as `false`, and both are the honest signal that the turn's
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

/// Every Lua block the agent executed within the turn with `turn_id`, in order — the scope for an
/// oracle that assesses one exchange's deliberation (how hard the agent looked for one answer) without
/// charging it for the legitimate work of unrelated turns.
pub fn lua_scripts_for_turn(events: &[Event], turn_id: TurnId) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::LuaExecuted {
                turn_id: id,
                script,
                ..
            } if *id == turn_id => Some(script.as_str()),
            _ => None,
        })
        .collect()
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

/// Whether the tag named `tag` was removed from any memory — the structural signal that a retroactive
/// untag landed. For `confidential` this is the mutation the untag gate forbids: it can only be cleared
/// from the console, never by a turn.
pub fn tag_removed(events: &[Event], tag: &str) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::TagRemovedFromMemory { tag: removed, .. } if removed.as_str() == tag)
    })
}

/// The set of memories a run's links connect to `seeds`, transitively and in either direction — the
/// closure the link readers can traverse from those memories. Computed over the run's own
/// `LinkCreated` events (a handful), so a simple fixpoint suffices. The seeds themselves are
/// included; a caller distinguishing "reached" from "is a seed" filters them out.
pub fn link_closure(events: &[Event], seeds: &BTreeSet<MemoryId>) -> BTreeSet<MemoryId> {
    let links: Vec<(MemoryId, MemoryId)> = events
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::LinkCreated { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .collect();
    let mut reachable = seeds.clone();
    loop {
        let mut grew = false;
        for (from, to) in &links {
            if reachable.contains(from) && reachable.insert(*to) {
                grew = true;
            }
            if reachable.contains(to) && reachable.insert(*from) {
                grew = true;
            }
        }
        if !grew {
            return reachable;
        }
    }
}

/// Whether a link of the relation named `relation` was created — the structural signal that the agent
/// recorded a typed edge, not just any link or prose.
pub fn link_created_with(events: &[Event], relation: &str) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::LinkCreated { relation: created, .. } if created.as_str() == relation)
    })
}

/// Whether a link of relation `relation` was created directed from `from` to `to`, regardless of its
/// source — the structural signal that the agent (or a pass) recorded *this specific* typed edge on
/// the right endpoints and the right way round. Unlike [`link_inferred_directed`], it accepts any
/// [`LinkSource`], so it recognizes an agent-authored `mem:link` as well as an inferred edge. A caller
/// with a coined relation whose direction is label-dependent (the agent may register `mentors` and
/// link `dave → erin`, or register `mentored_by` and link `erin → dave`) checks each direction-and-label
/// candidate, so a reversed edge or a wrong pair still fails.
pub fn link_directed(events: &[Event], from: MemoryId, to: MemoryId, relation: &str) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::LinkCreated { from: created_from, to: created_to, relation: created, .. }
            if *created_from == from && *created_to == to && created.as_str() == relation
        )
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

/// Whether any executed block read recorded links back — a link reader (`mem:outgoing`,
/// `mem:incoming`, or `mem:links`) or the whole-record `mem:details`, whose render includes the
/// links line with directions — the structural signal that the agent consulted the relationship
/// graph to answer, rather than reconstructing the connections from prose. (`links` here is the
/// `:links()` reader; the `links.*` registry calls are `links.list`/`get`/`register`, which
/// `script_calls` does not match on the bare `links(`.)
pub fn link_reader_called(events: &[Event]) -> bool {
    lua_called(events, "outgoing")
        || lua_called(events, "incoming")
        || lua_called(events, "links")
        || lua_called(events, "details")
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

/// Whether the agent proposed a cross-platform merge *and stated its grounds* — a `MergeProposed`
/// carrying a non-empty rationale, the coincidence the agent reasoned from, which the adjudicator reads
/// as the proposer's claim and weighs against the recorded facts. Stricter than [`merge_proposed`],
/// which counts a bare proposal too: this asserts the agent said *why*.
pub fn merge_proposed_with_rationale(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::MergeProposed { rationale: Some(rationale), .. } if !rationale.trim().is_empty()
        )
    })
}

/// The agent's responding replies recorded after the first cross-platform merge was proposed and before
/// the first adjudication settled it — the window in which the two stubs are proposed-but-not-yet-merged.
/// A reply here that already treats the two as one person is premature merged awareness: the gate exists
/// precisely so nothing crosses the would-be merge until an adjudication (or the operator) accepts it. An
/// initiated turn (a flush, a wake-up) is not a reply, so only `Responding` turns count. Empty when no
/// merge was proposed (nothing to check), and open-ended when a proposal was never adjudicated.
pub fn replies_between_proposal_and_adjudication(events: &[Event]) -> Vec<&str> {
    let Some(proposed_at) = events.iter().find_map(|event| {
        matches!(&event.payload, EventPayload::MergeProposed { .. }).then_some(event.seq)
    }) else {
        return Vec::new();
    };
    let adjudicated_at = events.iter().find_map(|event| {
        matches!(&event.payload, EventPayload::MergeAdjudicated { .. }).then_some(event.seq)
    });
    events
        .iter()
        .filter(|event| match adjudicated_at {
            Some(settled) => event.seq > proposed_at && event.seq < settled,
            None => event.seq > proposed_at,
        })
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
