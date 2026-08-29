# Events and roles

An Event is the stable identity of a happening. Its identity is minted when the happening is first represented and does not depend on its type, participants, occurrence time, extraction Occasion, or current role set. Those facts may be corrected without replacing the Event.

[Statements](statements.md) owns Proposition, Assertion, Attestation, Occasion, Activity, and Derivation identity and lifecycle. This chapter only specifies how those objects describe Events. Event occurrence time is owned by [time](time.md), and Event-to-Event predicates use the registry described in [relations](relations.md).

The event-and-role shape is supported by the ontology review and by a corpus case in which one happening was rotated onto several subjects ([research report](research/2026-07-24/report.md#32-events-and-roles-the-fix-for-one-event-many-copies), [modelling study](research/2026-08-03/modelling-study.md#the-multi-participant-event)). The corpus also showed that only two of four entries represented the same happening. Stable Event identity, reversible co-reference, and the projection rules below are design decisions to avoid turning that limited evidence into destructive deduplication.

## Roles and attributes

An Event has no mutable type or occurrence field. Registered Event type, participants, occurrence, location, outcome, manner, and other properties are attribute or role Propositions about the Event. Accepted role and attribute Assertions therefore retain independent validity, Attestations, source locators, transmission principles, and lifecycle transitions. An Event read projects type and occurrence only from the audience-safe folded Assertions available under the read environment.

```text
event/e1  type: event/create

(event/e1, role/agent, person/wren)
(event/e1, role/theme, person/quill)
(event/e1, role/source, topic/instance_architecture)
(event/e1, occurred_during, [2026-07-14, 2026-07-16))
```

The notation is a projected reading aid, not a serialisation. The `type:` line abbreviates an audience-safe Event-type Assertion under a registered relation; it is not an Event field. None of these edges constitutes the Event's identity.

Universal parent roles stay small: `agent`, `theme`, `instrument`, `source`, `recipient`, `time`, and `place`, plus only the few additions justified across Event types. A registered Event type may define typed subroles such as `buyer` and `seller`, or `approver` and `requester`. Every subrole declares its universal parent and filler constraints. Generic traversal uses the parent; precise queries may use the subrole.

This compromise preserves distinctions that a universal role set cannot answer while retaining a teachable fallback. Research supports a small universal inventory and warns that role tails are inconsistent even among expert annotators ([research report](research/2026-07-24/report.md#32-events-and-roles-the-fix-for-one-event-many-copies)). The typed-subrole scheme is a gated design synthesis. A subrole is activated only after fixtures establish teachability, stable filler constraints, and correct parent traversal. When no registered role fits, extraction leaves the content at the source locator or proposes a schema addition; it does not coin a hidden role or guess.

A role may have several fillers. Two people acting are two role Propositions, not one pair-valued edge. A count is appropriate only when participants were not individuated. The quantity rules belong to [statements](statements.md).

## Event-to-Event relations

Roles place entities within a happening. Ordinary registered relations connect happenings to each other:

```text
(event/e1, sparked, event/e2)
(event/e2, preceded, event/e3)
```

The modelling study found causation between happenings that roles alone could not express ([event relation finding](research/2026-08-03/modelling-study.md#events-have-no-relations-to-other-events)). Each relation instance is an Assertion, not an identity-bearing link baked into either Event.

## Co-reference is a reversible hypothesis

A new description does not resolve to an existing Event by structural equality. Repeated meetings can have the same type, participants, place, and overlapping approximate time. An ambiguous arrival mints a separate Event and may mint an Event-resolution hypothesis. `possibly_same_event` is the candidate projection of that object, not an ordinary relation and not the accepted composite.

The immutable proposal contains a minted hypothesis ID; an ordered, duplicate-free set of at least two member Event IDs; evidence locators; the proposing Activity or Occasion; ontology and matching-policy versions; and the identity-resolution environment used to propose it. Binary proposals are the initial policy. The n-ary representation is permanent so later evidence can resolve a set without chaining pairwise equivalence.

The append-only transition union is:

| Transition | Required fields | Fold result |
|---|---|---|
| `resolution_accepted` | hypothesis ID, separately minted composite-resolution ID, authority, evidence, policy version, and source head | `accepted`; the named composite becomes readable under this environment |
| `resolution_rejected` | hypothesis ID, authority, reason, evidence, and source head | `rejected`; members remain separate |
| `resolution_withdrawn` | hypothesis ID, authority, severance evidence, and source head | `withdrawn`; any accepted composite is no longer live |
| `resolution_superseded` | hypothesis ID, replacement hypothesis ID, authority, and reason | `superseded`; the replacement folds independently |

With no applicable transition, the fold is `candidate`. A transition cannot mutate members or reuse a composite ID. Competing candidate hypotheses may overlap. Accepted hypotheses must form disjoint member sets within one resolution environment; an acceptance that overlaps another live accepted set is rejected unless the same atomic batch withdraws or supersedes the conflict. Acceptance is idempotent only for the same hypothesis, composite ID, and source head. A retry with different evidence is a new Activity or transition, not a rewritten proposal.

Affirmative co-reference requires evidence beyond a similar role set. Useful evidence includes an explicit re-mention, a shared source occurrence, a sufficiently precise time and location, a series-instance identifier, or a matching causal neighbourhood. Acceptance exposes a composite view with its own stable resolution identity and leaves every source Event ID and role or attribute Assertion intact. It does not move edges, rewrite source records, or consume an Event. Derivations that use the composite are stamped with the accepted resolution environment.

Withdrawal or supersession restores the source views. Derivations stamped with the no-longer-live environment become invalid or pending recomputation under the [Derivation lifecycle](statements.md). Severance does not infer which source Event should own an Assertion that a later derivation attached only to the composite. That result remains linked to the composite environment and cannot silently migrate. Replay orders transitions by log sequence, validates conflict rules at each step, and deterministically reproduces the hypothesis state and live composite set.

### Co-reference fixtures

| Arrival | Required result |
|---|---|
| “We met on Tuesday” followed by “that Tuesday meeting” with matching source occurrence | The second Occasion may Attest Assertions about the existing Event, or support an accepted composite resolution if two IDs already exist. Both Occasions remain. |
| Two weekly meetings with the same people and an overlapping coarse date | Two Events linked at most by `possibly_same_event`; participant overlap is not enough to merge. |
| Two Events are accepted as one, then a precise location proves they were separate | Append severance; restore both stable Event views; invalidate composite-dependent Derivations; preserve all source Assertions and Attestations. |

These fixtures are required for the Stage 5 gate. The local four-entry case demonstrates duplication risk, but it does not validate an autonomous matching threshold ([modelling study](research/2026-08-03/modelling-study.md#the-multi-participant-event)).

## Disclosure-safe projection

Audience resolution happens before Event rendering. It selects visible Attestations and folded Assertions under [privacy and provenance](privacy-and-provenance.md); this chapter does not define a second visibility test.

Each Event type registers a projection policy for partial visibility:

- **Independently omissible roles** may be absent without changing the meaning of the visible Assertions. A public meeting may omit one confidential attendee and render as explicitly incomplete.
- **Jointly meaningful roles** must render with an incomplete shell that does not imply a hidden filler. A transfer with a visible item but hidden parties may be shown only as “a restricted transfer occurred”, if that shell is itself licensed.
- **Meaning-changing omissions** suppress the whole Event. Showing a visible `buyer` while hiding the only `seller`, for example, may manufacture a stronger or false account of the transaction.

The shell is a registered projection of visible Assertions, not a new Attestation and not evidence that an unspecified participant exists. It carries an explicit incompleteness marker and the Event type's projection-policy version. If no safe shell is registered, suppression is the default.

### Partial-disclosure fixture

An Occasion yields public time and place Assertions, an attributed participant Assertion, and a confidential participant Assertion for one Event. A public query may render only the time and place if the Event type declares them independently meaningful; an attributed query may add the first participant; an authorised query may show both. No view may expose hidden-role cardinality, substitute “someone” where that implies a known filler, or turn the visible participant into the sole actor.
