# Events and roles

An Event is the stable identity of a happening. Its ULID is minted when the happening is first represented. It does not depend on the Event's type, participants, occurrence time, extracting Occasion, or current role set, so any of those can be corrected without replacing the Event.

The [object model](statements.md) owns Proposition, Assertion, Attestation, Occasion, and Activity identity and lifecycle. This chapter specifies how those objects describe Events. [Time](time.md) owns occurrence values, and [relations](relations.md) owns the definitions that Event-to-Event predicates use.

The ontology review and a corpus case in which one happening was rotated onto several subjects support the event-and-role shape ([research report](research/2026-07-24/report.md#32-events-and-roles-the-fix-for-one-event-many-copies); [modelling study](research/2026-08-03/modelling-study.md#the-multi-participant-event)). The same corpus showed that only two of four entries represented the same happening. Stable Event identity without automatic deduplication, and the projection rules below, are design decisions that keep that limited evidence from turning into destructive merging.

## Roles and attributes

An Event has no mutable type or occurrence field. Its registered type, participants, occurrence, location, outcome, manner, and other properties are role or attribute Propositions about the Event. Each is an ordinary [Assertion](statements.md#assertion). An Event read projects type and occurrence only from the audience-safe folded Assertions.

```text
event/e1  type: event/create

(event/e1, role/agent, person/wren)
(event/e1, role/theme, person/quill)
(event/e1, role/source, topic/instance_architecture)
(event/e1, occurred_during, [2026-07-14, 2026-07-16))
```

The notation is a projected reading aid, not a serialisation. Handles stand for [Entity](statements.md#entity) ULIDs. The `type:` line abbreviates an audience-safe Event-type Assertion under a registered relation. None of these edges constitutes the Event's identity.

Universal parent roles stay small: `agent`, `theme`, `instrument`, `source`, `recipient`, `time`, and `place`, plus only the few additions justified across Event types. A registered Event type may define typed subroles such as `buyer` and `seller`, or `approver` and `requester`. Every subrole declares its universal parent and its filler constraints. Generic traversal uses the parent. Precise queries may use the subrole.

This compromise keeps distinctions that a universal role set cannot express and retains a teachable fallback. Research supports a small universal inventory and shows that role tails beyond the first two positions are inconsistent even among expert annotators ([research report](research/2026-07-24/report.md#32-events-and-roles-the-fix-for-one-event-many-copies)). Roles, subroles, and Event types are operator-governed definitions under [relations](relations.md#governance). A broader role vocabulary is deferred until scenarios establish teachability, stable filler constraints, and correct parent traversal. When no registered role fits, extraction leaves the content at the source locator or proposes a definition to the operator. It never coins a hidden role or guesses.

A role may have several fillers. Two people acting are two role Propositions, not one pair-valued edge. A count is appropriate only when participants were not individuated. The corpus study tested the role inventory, not filler multiplicity, so the multiple-filler rule is design synthesis.

## Event-to-Event relations

Roles place entities within a happening. Ordinary registered relations connect happenings to each other:

```text
(event/e1, sparked, event/e2)
(event/e2, preceded, event/e3)
```

The modelling study found causation between happenings that roles alone could not express ([event relation finding](research/2026-08-03/modelling-study.md#events-have-no-relations-to-other-events)). Each relation instance is an Assertion, not a link built into either Event.

## Co-reference

A new description never resolves to an existing Event by structural equality. Repeated meetings can share a type, participants, place, and an overlapping approximate time. When the writer recognises an explicit re-mention of a known Event, such as "that Tuesday meeting", the new Occasion attests Assertions about the existing Event directly. When the arrival is ambiguous, the writer mints a separate Event.

No Event merge exists at genesis. Two Events that denote one happening stay two Events, and duplicates are acceptable at this scale. Resolution hypotheses are identity-only ([identity](identity.md#resolution-hypotheses)). Event co-reference is deferred as `event-coreference` in [evolution](evolution.md#deferred-capabilities), and it reopens when measured duplicate-Event cost justifies it. The local four-entry case shows the duplication risk but is not evidence that merging is safe ([modelling study](research/2026-08-03/modelling-study.md#the-multi-participant-event)).

## Disclosure-safe projection

Audience resolution happens before Event rendering. It selects visible Attestations and folded Assertions under [privacy and provenance](privacy-and-provenance.md). This chapter defines no second visibility test. The [subject guard](privacy-and-provenance.md#subject-guard) protects every person-valued role position, including any role or subrole whose filler range admits a person, whatever the role is named.

Each Event type registers a projection policy for partial visibility:

- An independently omissible role may be absent without changing the meaning of the visible Assertions. A public meeting may omit one confidential attendee and render as explicitly incomplete.
- A jointly meaningful role set renders as an incomplete shell that does not imply a hidden filler. A transfer with a visible item and hidden parties may render only as "a restricted transfer occurred", and only if that shell is itself licensed.
- A meaning-changing omission suppresses the whole Event. Showing a visible `buyer` while hiding the only `seller` can manufacture a stronger or false account of the transaction.

The shell is a registered projection of visible Assertions. It is not an Attestation and not evidence that an unspecified participant exists. It carries an explicit incompleteness marker and the projection-policy version. When no safe shell is registered, suppression is the default. No view exposes hidden-role cardinality, substitutes "someone" where that implies a known filler, or turns a visible participant into the sole actor.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `event-one-happening-many-subjects` | One utterance describes a meeting with three participants. | One Event with three role Assertions, not three per-subject copies. |
| `event-multiple-fillers` | Two people jointly approve a request. | Two `agent` (or `approver`) role Propositions on one Event; no pair-valued edge. |
| `event-role-fallback` | A happening has a participant whose role fits no registered role or subrole. | No role Assertion is written for that participant. The participant stays in the cited span, and any new role reaches the operator as a definition proposal. |
| `event-subrole-parent-traversal` | A query asks for all agents of Events that have a `buyer`. | The `buyer` filler is returned through its declared parent `agent`. |
| `event-to-event-relation` | "The outage sparked the migration." | A `sparked` Assertion between two Events; neither Event record changes. |
| `event-remention` | "We met on Tuesday", later "that Tuesday meeting" with a matching source occurrence. | The second Occasion attests Assertions about the existing Event. Both Occasions remain. |
| `event-weekly-meetings-not-merged` | Two weekly meetings with the same people and an overlapping coarse date. | Two Events. No hypothesis, composite, or link between them is created, because no Event merge exists at genesis. |
| `event-partial-disclosure` | An Occasion yields public time and place, an attributed participant, and a confidential participant for one Event, and the type declares time and place independently meaningful. | A public read returns time and place with an incompleteness marker. A read cleared for the attributed participant adds that participant with its teller. A read cleared for both adds both. No read reveals the hidden-role count. |
| `event-meaning-changing-omission` | A transfer whose only `seller` is confidential is read by an audience that can see the `buyer`. | If the transfer type registers a licensed shell, the read returns only that shell with its incompleteness marker. Otherwise the Event is absent from the result. The buyer is never returned as the sole party. |
