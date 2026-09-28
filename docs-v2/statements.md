# Object model

This chapter, the object model, defines every identity-bearing object in the successor, its legal transitions, and the lifecycle mechanics the objects share. Where another chapter owns an object's rules, this chapter states the object's identity, fields, and lifecycle, and links to the owner.

The assertion layer separates semantic content, situated claims, and support into Proposition, Assertion, and Attestation. The dated snapshots under [`research/`](research/) use `Statement` for combinations of these objects. The exact split is a permanence-driven design decision. Research supports addressable contextual assertions and recorded lineage, but it does not establish this exact object model ([report](research/2026-07-24/report.md), [fact-shape lane](research/2026-07-24/lanes/fact-shape.md), [confidence register](program/confidence.md#evidence-map)).

Every minted identity is a ULID. No stable identity is a content hash, a human-readable handle, or a local log sequence number. The log position a read or write observed is recorded as an opaque frontier ([overview](overview.md#distributed-operation)).

```text
Occasion --contains--> ArtefactReference --refers-to--> Artefact
 (inbound or outbound)         |
    |                          +--consumed-by--> Activity --produces--> Perception, derived Artefact
    |                                               |
    +--grounds--> Attestation (testimony)           +--records--> context manifest (model calls)
                     |                              |
                     v                              +--grounds--> Attestation (observation, derivation)
Proposition <--keys-- Assertion <--supports-- Attestation

Entity, Event <--subject or object of-- Assertions
```

## Occasion

An Occasion is one message or delivery that crosses the agent's boundary: an inbound message, connector delivery, or operator input, or an outbound agent utterance. It has a minted ID and owns:

- a direction, `inbound` or `outbound`;
- one ordered sequence of content parts, in which text parts and ArtefactReference parts interleave, and either kind may be absent;
- a `caption_of` marker on a text part that describes a reference part on the same Occasion;
- participants, each with witness evidence ([privacy and provenance](privacy-and-provenance.md#witness-evidence));
- a restriction;
- an optional `in_reply_to` link to another Occasion, supplied by the connector;
- observed time and recorded time.

Text parts keep their order and jointly form the utterance. A caption is ordinary participant-authored text that can ground testimony. Accompanying the bytes never makes it mechanically true. A compound utterance produces one Occasion and any number of Assertions and Attestations, with no manufactured source phrases.

An inbound Occasion's restriction is its availability audience at receipt: a direct message's parties, or `channel(C)` for a channel message, whose membership follows the connector roster under the history rule the connector declares. An outbound Occasion's author is the agent, its recipients carry witness evidence of at least `delivered` assurance, and its restriction is its delivered audience. [Privacy and provenance](privacy-and-provenance.md#occasion-restriction) owns how restrictions are evaluated and propagate, the [testimony principle floor](privacy-and-provenance.md#testimony-principle-floor), and the [pre-delivery check](privacy-and-provenance.md#pre-delivery-check). An agent utterance is never testimony by the agent about what it relays ([belief](belief.md#dependence)).

`in_reply_to` records a platform reply from the platform's own reply metadata, and nothing splices a reference token into the text. It names the target's ID, so it survives the target's erasure as a link to a tombstone. This answers issue #109. Direction, restriction at receipt, and `in_reply_to` are raw input that later structure cannot recover, so all three are in the genesis design.

The lifecycle is `live`, `invalidated` for a malformed or unauthorised source, and `erased`. A correction arrives as a new Occasion that links to the old one.

## Activity

An Activity is any agent, operator, tool, or model action that the log already records: a model call, Lua block, tool call, or operator action. It is not a separate object kind with a lifecycle. It records its actor and source kind, implementation version, ordered input edges, output edges and named output fields, observed and recorded time, and outcome. A retry is a new Activity. A direct observation or Perception cites an Activity and needs no synthetic utterance or teller. Erasure removes an Activity's governed payload and leaves its ID and outcome class.

## Context manifest

Every model-call Activity records a context manifest: the ordered stable IDs of every object whose content was rendered into that call's context, with a source locator where only part was rendered. Content without an object ID, such as a prompt template or the API reference, is identified by template name and version. Every renderer goes through one choke point that appends the entry, and no other path places content into a model context. Rejected proposals, critic diagnostics, and retry contexts pass through it too.

Influence, restriction intersection, erasure dependants ([privacy and provenance](privacy-and-provenance.md#influence-from-context-manifests)), non-evidentiary taint ([the two traces](two-traces.md#the-episodic-wall)), and access accounting ([query surface](query-surface.md#access-accounting)) are projections over manifests plus Activity edges. None is stored on the objects it describes.

Manifest completeness is the load-bearing invariant of this design. A missing entry under-taints silently: the projections compute a narrower influence, a wider audience, or an incomplete erasure closure, and nothing detects the gap at read time. The single choke point makes completeness a property of one component. The manifest holds IDs only, so it survives erasure of the payloads it names.

## Entity

An Entity is a minted identity for a person, organisation, place, topic, or other thing, with a registered entity kind fixed at mint. Entity kinds are operator-governed ([relations](relations.md#governance)). Proposition keys name Entities by ULID.

A handle, such as `person/rowan`, is a mutable label that resolves to at most one live Entity at a frontier. Renaming appends `entity_handle_changed` and changes no key, Assertion, or Attestation. The earlier handle then returns a teachable error naming the current one. A handle minted in a restricted context carries that restriction until a clearing Occasion uses it or the operator publishes it ([Occasion restriction](privacy-and-provenance.md#occasion-restriction)).

The agent mints non-account Entities: people without an account, organisations, places, and topics. A connector mints a stub, a person Entity with connector scope keyed to the platform's most stable identifier ([identity](identity.md#permanent-stubs)). An agent-minted person and a stub never convert into each other. They join only through an accepted [resolution hypothesis](#resolution-hypothesis).

The lifecycle is `live`, `superseded`, and `erased`. `entity_superseded` names a replacement, and the handle moves to the replacement in the same transition. `entity_erased` removes the payload, including every handle, and leaves the ULID. An entity kind never changes. A mis-kinded Entity is corrected by minting a new Entity, superseding the old one onto it, and superseding each live Assertion onto the new ULID with Attestations that carry over the tellers and locators. `same_as` across kinds is refused, so a kind correction is never a merge.

## Proposition

A Proposition is canonical semantic content. It is a computed key over six coordinates, not a stored record:

```text
(subject, relation, object, frame, polarity, modality)
```

Equality includes all six coordinates and excludes teller, provenance, audience, Occasion, validity, mode, and lifecycle. Relation, frame, modality, and value encodings are registered definition IDs, never versions: the [Assertion](#assertion) records the versions, so a new definition version does not split one content into two keys. [Relations](relations.md) owns schema governance. Indexes keyed by Proposition are built from live payloads, so an erased Assertion leaves no key behind.

The subject is an Entity or Event ULID, never a handle. The object can be:

- an Entity or Event ULID;
- a typed date, duration, quantity, bounded count, or recurrence value;
- an opaque literal for formal content that the ontology does not model;
- a Proposition reference, reserved for attitudes about mental state such as "believes" or "wants", and never used for reported speech;
- an Occasion, content-part span, ArtefactReference, Perception, or Activity reference for metalinguistic and provenance claims.

A count records amount, kind or unit, exactness, and optional bounds. Named participants remain individual Event role Assertions, and an enumeration of named entities is many Assertions. Collective versus distributive readings can remain in the utterance.

### Frame

The frame identifies the referential layer. The genesis values are `system`, `persona`, and `source`, disjoint from modality values. A write may name the `principal` redirect, which resolves through a registered `presents` relation to the subject an account or persona presents and records that resolution as an input. `principal` is not a stored frame. The writer declares the redirect and it is never inferred, because a misfire files a claim about a bot onto a human.

### Polarity

Polarity is `positive` or `negative` from genesis. Lexical negation canonicalises into polarity over the same positive relation and object, so "works at X" and "does not work at X" are mechanically comparable. A lexically negative relation name is permitted only when its semantics are not the complement of another relation. A negative quantity stays a signed value unless the utterance negates the whole proposition.

### Modality

Modality is a registered, extensible axis. Its genesis definition contains `actual`, `planned`, `hypothetical`, `habitual`, `deontic`, and `cancelled`. "Not planned" is negative polarity over a `planned` Proposition, distinct from negative polarity over an `actual` one. Third-party deontic content is an Assertion, and only an agent-authored Task causes action. Initial inference can treat non-actual modalities opaquely, but it cannot omit the coordinate, because modality cannot be reconstructed from old structure. [Time](time.md#modality) owns the temporal reading of each value, including cancellation.

## Assertion

An Assertion situates a Proposition. It has a minted ID and an immutable record containing the Proposition coordinates, the definition versions it was accepted under, typed validity ([time](time.md#validity-observation-and-recording)), and a mode, `asserted` or `quoted`. One Assertion can have many Attestations. Disjoint validity periods are separate Assertions over one Proposition.

The creation record folds to `live`. `assertion_validity_closed` closes the open bound with a cause (world change, expiry, correction, retirement, or `unknown`) and leaves the state `live`, so a corrected claim and a claim that stopped holding stay distinguishable. `assertion_superseded` names a replacement, used for every correction or refinement. `assertion_retracted` keeps the payload. `assertion_invalidated` records that an assumption, authorisation, or resolution input no longer holds. `assertion_erased` leaves a tombstone.

### Per-audience state

Validity and lifecycle fold per audience. Every Assertion transition other than erasure names its evidence source: an Attestation, an Occasion, or an Activity. An audience sees only the transitions whose source it can see, so an Assertion is `live` for any audience that sees none of its supersession, retraction, or invalidation. The validity an audience sees is folded from the validity asserted by its visible Attestations and its visible closures, and it is `unknown` when none asserts one. The record's own validity is the reuse key and an operator audit field, never rendered to a conversational audience. A public read therefore never renders a date or a closure that came from a confidence.

Settlement is not an Assertion state and has no transition. An Assertion is settled for an audience when eligible support is visible to that audience, and default conversational reads return Assertions settled for the reader ([belief](belief.md#settlement-and-withdrawal)).

### Mode and reported speech

No transition changes mode. Reported speech, a relayed utterance or a claim an Artefact makes, is always `quoted`. A quoted Assertion renders only as reported speech with its teller and its `relayed_from` or Artefact source ("Rowan says Quinn said X"), never as a flat claim. A later flat assertion of the same content mints a separate `asserted` Assertion linked by corroboration lineage ([belief](belief.md#dependence)).

### Assertion reuse

A re-mention attaches a new Attestation to an existing live Assertion when the Proposition key and mode are equal and either the validity is identical, or the new validity is unknown or open and a live Assertion with that key and mode has open validity or covers the Occasion's observed time. Otherwise a new Assertion is minted. When several qualify, the most recently created one receives the Attestation, and the publishing Activity records the ambiguity. The assertion critic applies the rule deterministically over the writer's full view ([verified writes](verified-write.md#hard-critics)); the per-audience fold keeps the result safe. A re-mention under a newer definition version attaches like any other.

## Attestation

An Attestation records one source act's support for one Assertion. It has a minted ID, the Assertion ID, the validity its source asserted, a transmission principle, observed and recorded time, and a typed source, specified as a tagged union and implemented as a Rust enum.

| Variant | Contents |
|---|---|
| `testimony` | The teller, the Occasion, source locators into it, expression strength (`hedged`, `plain`, or `emphatic`), any grant span, resolution inputs, and witness and dependence lineage, including `relayed_from`. |
| `observation` | The agent, operator, or tool Activity and its observation edges. |
| `derivation` | The producing Activity, ordered typed inputs, criterion, implementation, ontology, and policy versions, assumptions, and the frontier it read. |

Only `testimony` contributes teller support or corroboration. An observation or derivation never fabricates a teller or an utterance.

A testimony teller is the participant who produced the cited span, recorded as the Entity the Occasion names, even when the write went through a composite handle. Relay is not a teller exception: Rowan's "Quinn told me that X" is Rowan's testimony for a `quoted` Assertion with `relayed_from` naming Quinn ([write surface](write-surface.md#teller-binding)). Stored records that grounding used to resolve the span, such as the kinship Assertion that resolves "my sister" to Quinn, are listed as resolution inputs. [Privacy and provenance](privacy-and-provenance.md#testimony-principle-floor) owns the principle floor, grants, and the restriction resolution inputs add. [Verified writes](verified-write.md#hard-critics) owns testimony grounding; a claim that fails it may be proposed as a `derivation`.

An `observation` records an uncomputed direct observation or operator assertion. Any transformation, aggregation, inference, extraction, or criterion-dependent result is a `derivation`. Its inputs can be Assertions, Perceptions, tool outputs, aggregates, definitions, policies, audience-safe negative query results, and source edges. A negative result records its exact query, audience, frontier, and projection version; absence outside that context is not evidence. A source edge names an Occasion with the exact locators consumed, or an Activity with the exact output fields consumed, and does not assert the source content. A derivation records its frontier, and the ResolutionEnvironment it used is recomputed from it ([resolution hypothesis](#resolution-hypothesis)).

Teller, Occasion, and Assertion together are not a uniqueness key. A hedge followed by a flat assertion from one teller is two Attestations linked as continuation, and expression strength changes neither Proposition identity nor corroboration ([belief](belief.md#expression-is-not-corroboration)).

The initial state is `live`. `attestation_superseded` withdraws this act's interpretation in favour of a named successor, `attestation_retracted` withdraws the support, and `attestation_invalidated` records that an authority, assumption, or input no longer holds. All three retain the payload, and none can be undone. `attestation_erased` leaves a tombstone. Losing the last live support changes the settlement projection, never the Assertion's identity or lifecycle. A withdrawn assumption invalidates dependent derivation Attestations in a rebuilt projection without editing them.

"How do you know?" returns the complete recorded lineage where the operation captured it, and otherwise an audit trace that states its limits. The system does not call every trace a proof.

## Perception

A Perception is the fallible output of a model or tool Activity over an ArtefactReference and a selector, such as OCR text or a generated caption. It records its producing Activity and typed inputs directly. It is never testimony and is never attributed to the person who supplied the bytes. An image-derived Assertion cites its Perception and the consumed reference through a `derivation` Attestation.

The lifecycle is `current`, `superseded`, `retracted`, `invalidated`, and `erased`. A correction mints a new Perception and supersedes the old one. A Perception is unusable while its consumed reference is not `authorised`. That is a read-time projection, not a transition, so restoring the reference restores its use. `perception_invalidated` records a real invalidation cause, such as a withdrawn assumption. [Artefacts and perceptions](artefacts-and-perceptions.md#perceptions) owns the recorded fields, reinspection, and initial image policy.

## Artefact and ArtefactReference

An Artefact is a minted ULID for one immutable byte sequence. It is not a content hash, because an erased file's tombstone would then confirm a guessed file. Its digest and byte metadata live in its erasable payload. Availability folds to `available`, `unavailable`, or `erased`. Retention is the set of non-erased references, including `withdrawn` and `retracted` ones, because both are reversible. `artefact_erased` is appended in the same batch as the erasure of the last non-erased reference, and it is legal only then. A derived Artefact, such as a page extract, text extraction, or generated episode, records its producing Activity, typed inputs, and an immutable output classification. It has no references of its own and is erased through [dependant closure](privacy-and-provenance.md#dependant-closure). `episodic_reconstruction` is the genesis non-evidentiary classification, and nothing removes it.

An ArtefactReference is one sharing act on one Occasion, recording the Artefact, supplier, filename, media type, part position, and transmission principle. Its lifecycle is `authorised`, then `withdrawn` or `retracted`, which `reference_restored` reverses, and `erased`, which is terminal. A transition on one reference never changes another.

A selector addresses all or part of one Artefact. It is a content-keyed value with no minted ID, compared by byte equality of a canonical encoding. The genesis variants are `whole_artefact`, `page_range`, and `text_span` over a derived text Artefact with a page map back to the original. A derived text Artefact is reusable across references to the same Artefact, and its readability always follows the consuming reference. [Artefacts and perceptions](artefacts-and-perceptions.md) owns both objects, [selectors](artefacts-and-perceptions.md#selectors), and the deferred selector variants.

## Event

An Event is a minted identity for a happening. Its type, participants, occurrence, and other properties are role and attribute Assertions about it, each with its own validity, Attestations, and lifecycle. Event identity does not depend on type, participants, occurrence, or the Occasion that first described it. The lifecycle is `live`, `invalidated` for an unsupported identity, and `erased`.

Events have stable identity and no co-reference mechanism: two Events that describe one happening stay two, which is acceptable at this scale. Event co-reference is deferred ([evolution](program/evolution.md#deferred-capabilities)). [Events and roles](events-and-roles.md) owns the universal parent roles with typed subroles, Event-to-Event relations, and the disclosure-safe projection rules (omissible role, incomplete shell, and suppression).

## Resolution hypothesis

A resolution hypothesis is one reversible identity `same_as` proposal. It has a minted ID and an immutable record containing an ordered, duplicate-free member set of at least two Entities or prior composites of one entity kind, the evidence, and the proposing authority. A hypothesis across kinds is refused, and Events are never members.

The initial state is `candidate`. `accepted` mints a separate composite ID and names a clearance level, `recall` or `disclosure`, and a primary member. `clearance_changed` and `primary_changed` adjust an accepted composite without changing its ID, and earlier writes stay where they landed. `rejected`, `withdrawn`, and `superseded` (naming a replacement) end it. Accepted member sets are disjoint at any frontier: an overlapping acceptance is rejected unless the same atomic write withdraws or supersedes the conflict. Hypotheses are never transitively closed; `{a,b}` and `{b,c}` do not imply `{a,b,c}`. Acceptance never consumes, moves, or rewrites a member. [Identity](identity.md#resolution-hypotheses) owns the transition fields, [clearance](identity.md#recall-and-disclosure-clearance), and [writes through a composite](identity.md#writes-through-a-composite).

A ResolutionEnvironment is a derived read concept, not a stored stamp: the accepted hypotheses, composites, and policy versions the log holds at a recorded frontier, recomputed on demand. A resolving read or derivation records its frontier. Withdrawal of a hypothesis invalidates whatever was derived at a frontier whose environment contained it ([identity](identity.md#resolution-environments)).

## Task

A Task is an agent-authored action intent with a minted ID and an immutable record of its action, actor, arguments, authority, audience, source, and one or more trigger conditions. Each firing is recorded against its condition.

The lifecycle is `proposed`, `active`, `completed`, `cancelled`, `superseded`, and `erased`. A trigger condition fires only while its Task is `active`. A descriptive Event occurrence never fires, whatever its modality or recurrence. A renewed intent mints a new Task. [Time](time.md#occurrence-and-task) owns condition kinds, due times, recurrence, and the genesis action vocabulary, and [privacy and provenance](privacy-and-provenance.md#pre-delivery-check) owns the audience checks at creation and firing.

## Lifecycle mechanics

Identity-bearing records are immutable. Every change is an appended transition, and a versioned fold derives each object's state from its creation record and transitions.

Every transition names its target and the one transition, or the creation record, that it follows. A write whose named predecessor is not the target's current head is rejected. This prevents lost updates between a turn and a background job that read the same state: the second writer fails, re-reads, and decides again. A transition also records its actor or authority, reason, evidence source, policy version, and frontier.

Two accepted transitions that name the same predecessor are a fork, which only an import or a future multi-writer merge can produce. The projection marks the target `conflicted`, and a later transition resolves it.

Erasure is terminal. No transition is accepted after it. Replay never reconstructs erased payload.

### Legal transitions

The fold accepts a transition only from the listed states. Any other transition is rejected, and the reference model tests every row. Every object except an Artefact also accepts its erasure transition from any non-erased state, which folds it to `erased`; those rows are omitted below.

| Object | Transition | From | To |
|---|---|---|---|
| Occasion | `occasion_invalidated` | `live` | `invalidated` |
| Activity | none; the outcome is recorded once, and erasure removes payload only | | |
| Entity | `entity_handle_changed` | `live` | `live` |
| Entity | `entity_superseded` | `live` | `superseded` |
| Proposition | none; it is a computed key | | |
| Assertion | `assertion_validity_closed` | `live`, with open validity | `live` |
| Assertion | `assertion_superseded`, `assertion_retracted`, `assertion_invalidated` | `live` | `superseded`, `retracted`, `invalidated` |
| Attestation | `attestation_superseded`, `attestation_retracted`, `attestation_invalidated` | `live` | `superseded`, `retracted`, `invalidated` |
| Perception | `perception_superseded`, `perception_retracted`, `perception_invalidated` | `current` | `superseded`, `retracted`, `invalidated` |
| Artefact | `artefact_stored` | `unavailable` | `available` |
| Artefact | `artefact_erased` | `available` or `unavailable`, in the batch that erases its last non-erased reference | `erased` |
| ArtefactReference | `reference_withdrawn`, `reference_retracted` | `authorised` | `withdrawn`, `retracted` |
| ArtefactReference | `reference_restored` | `withdrawn` or `retracted` | `authorised` |
| Event | `event_invalidated` | `live` | `invalidated` |
| Resolution hypothesis | `accepted`, `rejected` | `candidate` | `accepted`, `rejected` |
| Resolution hypothesis | `clearance_changed`, `primary_changed`, `withdrawn` | `accepted` | `accepted`, `accepted`, `withdrawn` |
| Resolution hypothesis | `superseded` | `candidate` or `accepted` | `superseded` |
| Task | `task_activated` | `proposed` | `active` |
| Task | `task_completed` | `active` | `completed` |
| Task | `task_cancelled`, `task_superseded` | `proposed` or `active` | `cancelled`, `superseded` |

A resolution hypothesis has no erasure state; when its evidence is erased it keeps a tombstone of its member set and transitions. Superseded, retracted, and invalidated records stay addressable for authorised audit, and no record reuses an erased ID. [Privacy and provenance](privacy-and-provenance.md#retraction-and-erasure) owns erasure authority, the envelope and payload split, and dependant closure.

## Mechanical contradiction

Contradiction detection covers a small registered subset over the polarity-free core `(subject, relation, object, frame, modality)`.

Two live Assertions with overlapping validity are mechanically contradictory when:

- they share a core and have opposite polarity;
- a registered functional or exclusive relation has incompatible objects, and the operator has confirmed that cardinality ([relations](relations.md#coining-under-critics));
- registered mutually exclusive kinds apply to the same subject;
- exact quantities differ, or an exact quantity falls outside another bounded quantity.

A changed functional value over disjoint validity is a temporal update. Linguistic negation, conditionals, context-dependent opposition, and general inconsistency are not mechanical. [Belief](belief.md#contest-and-contradiction) owns when detection runs, the queue mark it appends, and the `contested` ordinal. The exact polarity axis and rule subset are design synthesis that the reference model and the initial support policy must test ([identity and belief lane](research/2026-07-24/lanes/identity-belief.md), [current arbitration failure](../docs/ontology-failures/2026-07-23.md)).

## Source locators

A source locator points at the exact source material an Attestation or derivation input relies on. It is a typed union:

| Kind | Target |
|---|---|
| `text_part_span` | An Occasion text part and a half-open Unicode scalar span within it. |
| `artefact_selector` | An ArtefactReference and a selector on its original or derived Artefact. |
| `activity_output` | An Activity and one named output field, such as a tool result. |
| `activity` | A direct-observation or operator Activity as a whole. |
| `derivation_input` | A derivation Attestation and one of its typed input edges. |

Initial image policy can use only whole-artefact selectors. Page and span policies add behaviour without changing old records.

## Representational limits

Figurative language, analogy, and unmodelled formal content can remain in an utterance or an Artefact. An artefact-only Occasion is complete input, and source-only is a valid terminal state of the write transaction. Directives and the agent charter are configuration, not Assertions; [memory typology](memory-typology.md) owns their versioned containers. These limits prevent an extractor from filling required fields with unsupported structure.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `compound-utterance` | One Occasion's text "Rowan arrived. Quinn left." yields two claims. | Two Assertions, each with one Attestation citing its own `text_part_span`; the Occasion is unchanged. |
| `direct-tool-observation` | A tool returns "ok", and the agent asserts the service is healthy. | The Attestation is a `derivation` over the tool's `activity_output`; no `testimony` Attestation exists. |
| `quotation-then-assertion` | Rowan says "Quinn said Rowan approved X", then "I approved X". | A `quoted` Assertion with `relayed_from` Quinn and a separate `asserted` one, linked by corroboration lineage; reads render the first only as "Rowan says Quinn said …". |
| `reuse-requires-same-mode` | A live `quoted` Assertion of X exists, and Sam asserts X flatly. | A new `asserted` Assertion is minted; nothing attaches to the quoted one. |
| `attitude-not-reported-speech` | Rowan says "Quinn believes the office is closed" and "Quinn said the office is closed". | The first has a Proposition-reference object under a `believes` relation; the second is a `quoted` Assertion with `relayed_from` Quinn. |
| `per-audience-validity` | Quinn's confidence dates an Assertion to 2019, and Rowan's public Attestation attaches with no stated time. | A public read renders `unknown` validity; an audience that clears Quinn's Attestation sees 2019. |
| `closure-evidence-per-audience` | `assertion_validity_closed` cites Quinn's confidence, and `assertion_retracted` cites an operator-only Activity. | A public read renders the Assertion `live` with open validity; the operator sees it `retracted` and closed with its cause. |
| `lexical-negation` | "works at X" and "does not work at X" are written. | Both key onto `works_at` with opposite polarity; a `does_not_work_at` relation is rejected by the proposition critic. |
| `negative-quantity-and-modality` | A quantity of -5 and a "not planned" claim are written. | The first stores -5 with `positive` polarity; the second is `negative` over `planned`. |
| `opposite-polarity` | Positive and negative Assertions share one core over overlapping validity. | Both stay `live`, and the pair matches the opposite-polarity rule. |
| `quantity-contradiction-subset` | Exact count 5, lower bound 6, and an unbounded "about six" are written over one core. | Only the exact count and the lower bound match a rule; "about six" matches none. |
| `principal-redirect` | A write names `principal` for an account that `presents` a person. | The stored subject is the person's ULID under an ordinary frame, and the `presents` Assertion is listed as an input. |
| `entity-kind-correction` | `organisation/juniper` turns out to be a person. | A person Entity replaces it via `entity_superseded`, the handle resolves to the new ULID, live Assertions are superseded onto it, and `same_as` between the two is refused. |
| `handle-rename` | `person/rowan` is renamed to `person/rowan-hale`. | Keys, Assertions, and Attestations are unchanged; the old handle returns a teachable error naming the new one. |
| `definition-version-outside-key` | A relation gains version 2, and a re-mention is written under it. | The key is unchanged, the Attestation attaches, and the Assertion's recorded version stays 1. |
| `assertion-reuse-open-validity` | Rowan says "Quinn works at Northwind"; a week later Sam says it with no stated time. | Sam's Attestation attaches; the key still has one Assertion. |
| `assertion-reuse-distinct-validity` | A live Assertion dates Quinn's job to 2019, and Rowan says Quinn works there now. | A second Assertion over the same Proposition is minted. |
| `assertion-reuse-ambiguous` | Two live Assertions with one key and mode both have open validity, and a re-mention arrives. | It attaches to the newer one, and the publishing Activity records both IDs as ambiguous. |
| `reply-link` | A connector delivers Rowan's message as a platform reply. | `in_reply_to` names the target Occasion ID, the text has no spliced token, and after the target's erasure the link names its tombstone. |
| `lost-update-rejected` | Outside any proposal, the operator retracts an Assertion while a job closes its validity, both naming head H. | The first commits; the predecessor check rejects the second. This covers direct appends; proposals and job results have their own scenarios. |
| `illegal-transition-rejected` | Validity closure on a `superseded` Assertion, `reference_restored` on an `authorised` reference, and any transition on an erased reference. | All three are rejected; replay reproduces the same states and no erased payload. |
| `artefact-erasure-needs-last-reference` | `artefact_erased` is requested while one `withdrawn` reference remains; later that reference is erased. | The request is rejected; the reference's erasure appends `artefact_erased` in the same batch. |
| `perception-follows-reference` | A reference with a Perception is withdrawn, then restored. | While withdrawn, reads exclude the Perception and no `perception_invalidated` is appended; after restoration it is usable. |
| `manifest-records-rendered-content` | A model call renders a brief, two memory reads, an Occasion, and the API reference. | The manifest lists the object IDs in render order and the reference by template name and version. |

Owner chapters hold the scenarios for their mechanisms, such as settlement and `hedge-then-flat` in [belief](belief.md#scenarios) and Occasion restriction and laundering in [privacy and provenance](privacy-and-provenance.md#scenarios).
