# Verified writes

A verified write is a durable proposal. A model or deterministic caller proposes typed records, and hard critics decide whether they are structurally admissible. The critics do not establish truth.

[The object model](statements.md) owns every object identity and lifecycle and [the predecessor rule](statements.md#lifecycle-mechanics). A proposal refers to those objects and never mints substitutes. [The write surface](write-surface.md) describes the two candidate surfaces that produce proposals.

Nondeterministic work is recorded before its result is used, and replay consumes the record without calling a model. This follows the current record-at-call-time contract ([`docs/events-and-storage.md`](../docs/events-and-storage.md#event-sourcing)). The proposal lifecycle and atomic publication are design synthesis driven by permanence, not an established result.

## Proposal states

A proposal has a stable ID and a source: one or more Occasions or Activities from one block. It has four states.

| State | Meaning | Successors |
|---|---|---|
| `proposed` | Work is in progress. The sources are durable and readable on the source lane. No proposed structure is visible to conversational reads. | `published`, `source_only`, `abandoned` |
| `published` | One atomic publication record names every accepted object and transition. The accepted Assertions become available through audience-resolved reads. | terminal |
| `source_only` | Structured acceptance did not complete. The record states why: extraction failed, every item was dropped, the retry bound was exhausted, or review could not complete. The sources stay durable and searchable. | terminal |
| `abandoned` | An actor ended the proposal, or a source was invalidated or erased. The record names the actor, the reason, and any replacement proposal. | terminal |

Work inside `proposed` is recorded as attempts, not states. An attempt names its kind (extraction, critic run, review, publication), its number, and its outcome or failure class. Item dispositions (`proposed`, `amended`, `dropped`, `accepted`) are appended records too. A dropped item cannot later be accepted. An amendment creates a new item version with lineage, and the critics run on it again.

A correction after publication is an ordinary Assertion or Attestation transition and never reopens the proposal.

### Publication and the predecessor check

Publication appends every accepted record and the publication marker in one atomic write under the single writer. A crash before it exposes nothing; a crash after it exposes everything. Items carry proposal-local IDs until the publication write mints their ULIDs, so a failed write mints nothing and a retry cannot duplicate an identity.

Every transition in the publish set names the head it follows, and the publication record names the proposal's latest record. At commit the writer checks each against the current head. A mismatch rejects the publication and is recorded as a failed attempt with the class `stale_predecessor`. This prevents a lost update between a turn and a background job that read the same head, and it prevents an abandoned proposal from publishing. Unrelated appends and the proposal's own attempt records do not stale it.

Two further inputs are checked at commit. Every source must still be live, so an invalidated or erased source publishes nothing. A derivation citing a scoped negative query or an aggregate records the query and frontier, and publication re-evaluates it and rejects a changed result. When irrelevance cannot be established, publication fails closed.

After a `stale_predecessor` rejection the proposal stays `proposed`. The reviewer reruns the affected critics against the new head, reusing retained extraction output, and accepts a revised set or abandons. A transient storage failure retries the same set within the bounded retry policy, and exhaustion appends `source_only`.

### Crash recovery

A proposal still `proposed` after a crash reruns extraction as a new attempt. Under [candidate B](write-surface.md#candidate-b-typed-claims) the proposal is the block's own claims, so a crash before block commit leaves no proposal to recover. The earlier attempt and its model call stay in the log, and replay consumes both without calling a model. A critic infrastructure failure reruns only the critics. A deterministic critic rejection returns diagnostics for amendment or dropping and never reruns an unchanged extraction.

## Typed proposals

An extraction Activity or a direct write can propose:

- a Proposition and an Assertion over it;
- an Attestation of any source type, grounded according to that type;
- an Event and independently addressable role or attribute Assertions;
- a Perception grounded in an ArtefactReference and a selector;
- a Task with its trigger conditions;
- an explicit `nothing_to_record` result.

A direct write uses an Activity as its source ([`claim`](write-surface.md#claim)). Conversational structuring creates a `testimony` Attestation only when a cited span passes the testimony-grounding critic below. A `structuring` job opens the same kind of proposal off-turn ([off-turn work](off-turn.md#authority-classes)).

## Hard critics

Hard critics are deterministic, versioned, and gating. Each failure is machine-readable with a correction hint. The critics check separate concerns so that one valid property cannot conceal an invalid one.

- The proposition critic checks registered subjects and relations, typed object variants, frames, polarity, modality, domain and range, mutually exclusive kinds, typed quantities, and the prohibition on duplicating frame, modality, or lexical negation inside the relation.
- The assertion critic checks validity shape, temporal precision, the immutable assertion mode, lifecycle-transition authority, and that no proposal carries settlement or support, which [belief](belief.md#settlement-and-withdrawal) projects at read time.
- The attestation critic checks source authority: for `testimony`, the teller, expression strength, transmission principle, scoped witness evidence, and locator. The teller must be the participant who produced the cited span on that Occasion, and reported speech must be `quoted` with `relayed_from` lineage ([teller binding](write-surface.md#teller-binding)). The principle must be no wider than the floor ([transmission principles](privacy-and-provenance.md#transmission-principles)) unless the item cites a teller grant, checked below.
- The testimony-grounding critic checks that every typed value and entity reference in a `testimony` Attestation's Proposition appears in the cited span or resolves from it. Typed values are dates, quantities, and named handles. A value resolves from the span only through a recorded resolution that lists its inputs: a relative date resolves against the Occasion's observed time, and "my sister" resolves to Quinn through a kinship Assertion. Every listed input is an input of the Attestation, so the audience critic conjoins its restriction.
- The grounding critic checks spans, selectors bound to the cited ArtefactReference, and observation, operator, and derivation sources according to type.
- The Event critic checks registered roles and the Event type's projection rules.
- The Task critic checks that only an agent-authored Task carries trigger conditions. A descriptive Event occurrence cannot arm scheduling.
- The derivation critic checks typed inputs, exactly one output, the direct-observation exception, and a single recorded frontier.
- The audience critic computes each output's restriction under the rules in [privacy and provenance](privacy-and-provenance.md#influence-from-context-manifests), over every context manifest in the proposal's Activity ancestry, including notes, retries, and intermediate calls. It rejects an output whose declared principle is wider than that result.
- The episodic wall critic applies [the episodic wall](two-traces.md#the-episodic-wall) to the same manifests.

The teller-grant check runs like grounding. A grant is a span that expresses permission to share, such as "feel free to tell people". The hard check requires the grant span to be cited, to lie within a text part, and to be produced by the Attestation's teller. A soft critic judges whether the span expresses permission and how far. The widening fails closed: it takes effect only when the soft critic confirms the grant. An unconfirmed grant does not reject the item. The item publishes at its floor, and the claimed widening is recorded for audit, where the operator may apply it. An operator widening is an operator transition, not a proposal item. [`principle-assignment`](evolution.md) sets the soft critic's thresholds.

Testimony grounding closes a laundering path. Without it, a model that read Quinn's confidence could file the same fact as testimony by citing any span of Rowan's message, taking Rowan's floor instead of the confidence. A claim that fails the check cannot be testimony. It may be proposed again as a `derivation` Attestation, which inherits the context's ancestry restriction. The hard check covers values and references, not the relation. A soft critic judges whether the span expresses the relation chosen, and the testimony classification fails closed on it: an item the soft critic does not confirm publishes only as a `derivation`, under the context's ancestry restriction. Without this, a model that read Quinn's confidence "Rowan is pregnant" could file it as Rowan's testimony by citing Rowan's "I'm tired today", because the entity resolves from "I" and only the relation is wrong. [`write-surface-comparison`](evolution.md) measures the critic's rejection rate and its catch rate on seeded laundering cases.

Grounding verifies that a span lies within the cited text part, or that a selector targets the Artefact behind the cited ArtefactReference and the Activity's input edges name the selector, decoder, and reference. A `text_span` over a derived text Artefact must also be contained in one span that a context in the write's Activity ancestry rendered. Overlap is not enough, because the uncovered part was never read ([reading a document](artefacts-and-perceptions.md#reading-a-document)). This establishes source attachment, not correct interpretation or truth. The current span check reduced unsupported dated occurrences by roughly 40% on one field in one local measurement; the generalisation to every extracted value is untested ([`research/2026-08-06/current-system-fixes.md`](research/2026-08-06/current-system-fixes.md)).

## Soft critics

Soft critics judge linguistic quality and plausibility. They can rank, request review, or attach diagnostics. They cannot publish, reject a valid proposal, settle, widen an audience, or override a hard critic. Two gates consult them and fail closed on a missing confirmation: the testimony classification and a teller grant. Neither gate rejects the item; an unconfirmed item publishes under the narrower restriction. Their calls are recorded Activities.

## Derivation inputs

A `derivation` Attestation records its ordered typed inputs (Assertions, scoped negative results, aggregates, tool observations, Perceptions), criterion and implementation versions, ontology and policy versions, assumptions, and the frontier, from which the ResolutionEnvironment is recomputed. A negative result names its query, audience, frontier, and closed-world scope. An absence outside that scope is not a premise.

A lineage response is complete only when the typed inputs and the producing Activity's context manifests account for every influence. Otherwise the system returns an audit trace and labels the unrecorded boundary. It does not call any explanation a proof.

## Admissibility is not settlement

Passing the critics establishes admissibility, not truth. Publication appends no settlement record: whether a published Assertion is settled for an audience is the read-time projection in [belief](belief.md#settlement-and-withdrawal). Publication runs mechanical contradiction detection over the publish set and appends a `contradiction_detected` mark for each match ([contest](belief.md#contest-and-contradiction)).

## Drift controls

Critic versions prevent known malformed writes. Canaries, replay audits, rate checks, and re-derivation detect regressions but establish no correctness. Similarity thresholds are tied to their embedding version. Persistent rejection, an identity-boundary disclosure, an unclear erasure authority, and a drift alarm enter the operator exception queue, which is an explicit operational cost: a review cost constant per stored fact does not scale.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `publication-stale-predecessor` | A multi-item proposal includes a transition from head H, and a turn transitions the target first. | The whole publish set is rejected as `stale_predecessor`, recorded as a failed attempt, and its critics rerun against the new head. Unlike `lost-update-rejected`, the unit is an atomic publish set and the proposal stays `proposed`. |
| `abandon-publish-race` | A proposal is abandoned while its publication is pending. | Publication names a stale proposal head and is rejected as `stale_predecessor`. Nothing publishes, and the proposal stays `abandoned`. |
| `unrelated-append-no-stale` | Unrelated records and the proposal's own attempts are appended before publication. | Publication succeeds. |
| `atomic-publication-crash` | A crash occurs before, and separately after, the publication write. | Before: nothing visible, no ULID minted. After: everything visible once. |
| `crash-reruns-extraction` | A crash occurs after the extraction call of a `proposed` proposal. | Extraction reruns as a new attempt. Both calls are logged; replay calls no model. |
| `critic-retry-no-reextraction` | A critic run fails on infrastructure; separately, a critic rejects an item. | Only the critics rerun, or diagnostics return. Extraction never reruns. |
| `source-only-fallback` | Extraction fails until its retry bound is exhausted. | The proposal is `source_only`; the Occasion stays searchable; no partial structure. |
| `dropped-item-stays-dropped` | A reviewer drops an item, then tries to accept it. | Refused. An amended version is a new item that faces the critics again. |
| `erased-source-cannot-publish` | The source is erased while its proposal is `proposed`. | The proposal is abandoned with reason `source_erased`; nothing publishes. |
| `negative-query-changed` | A derivation cites an absence at frontier F, and a matching Assertion publishes first. | Publication re-evaluates the query, rejects the derivation, and records a failed attempt. |
| `span-outside-part` | An Attestation cites a span past the end of its text part. | The grounding critic rejects it with a teachable error. |
| `hidden-input-wider-output` | A call's manifest holds an `in_confidence` Attestation, and it proposes a `public` derivation from it. | The audience critic rejects the derivation with a teachable error naming the manifest entry. |
| `laundered-testimony-rejected` | Quinn's confidence is in context, and the model files Quinn's fact as `public` testimony citing a span of Rowan's message. | The testimony-grounding critic rejects it, because the values do not appear in the span. Proposed again as a derivation, it inherits the confidence's restriction. |
| `testimony-value-resolved` | Rowan says "the launch is tomorrow", and the claim carries the resolved date. | The resolution record lists the Occasion's observed time as its input, and the testimony passes. |
| `resolution-input-restricts` | Rowan says "my sister is moving" in channel C, and "my sister" resolves to Quinn through a kinship Assertion held `in_confidence`. | The resolution record lists the kinship Assertion. The Attestation's restriction is `channel(C)` conjoined with that Assertion's restriction. |
| `teller-grant-widens` | Rowan says "feel free to tell people" in a DM, and the item cites that span with a `public` principle. | The hard check passes, the soft critic confirms the grant, and the item publishes as `public`. |
| `unconfirmed-grant-publishes-at-floor` | The item cites Rowan's span "I'm tired today" as a grant with a `public` principle. | The hard check passes, the soft critic does not confirm the grant, and the item publishes at the DM floor with the claimed widening recorded for operator audit. |
| `grant-by-other-teller-rejected` | The cited grant span was produced by Quinn, not the Attestation's teller Rowan. | The attestation critic rejects the widening with a teachable error. |
| `publication-appends-no-settlement` | A single teller's grounded testimony publishes. | The publish set holds no settlement record. A next-day read by an audience that sees the Attestation renders it settled. |
| `unconfirmed-relation-becomes-derivation` | After a context rendered Quinn's confidence that Rowan is pregnant, an item files (Rowan, pregnant) as Rowan's testimony citing "I'm tired today". | The hard check passes, the soft critic does not confirm the relation, and the item publishes only as a `derivation` restricted to the confidence's audience. |
