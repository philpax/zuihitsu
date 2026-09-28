# Privacy and provenance

Privacy constrains information flow. It is not a tag on semantic content. [Occasions](statements.md#occasion) carry a restriction and witness evidence, [Attestations](statements.md#attestation) and [ArtefactReferences](artefacts-and-perceptions.md) carry transmission principles, and [Assertions](statements.md#assertion) are shown to an audience only through the Attestations and transitions that clear it. Everything derived inherits its inputs' restrictions through the [context manifest](statements.md#context-manifest). This chapter owns these rules, and other chapters point here.

Evidence is recorded broadly and immutably, then interpreted under narrow, versioned policies. Contextual integrity supports transmission principles as conditions over an information-flow trace. The vocabulary and compilation rules below are design decisions ([research lane](research/2026-07-24/lanes/provenance-privacy.md#4-contextual-integrity-transmission-principles-as-first-class-data)).

## Transmission principles

Each restriction is one registered, versioned principle or a conjunction of them. The genesis evaluator accepts only semantics it can decide cheaply and fail closed.

| Principle | Condition |
|---|---|
| `public` | Any authenticated audience, subject to the subject guard, erasure, and every other check. |
| `attributed` | As `public`, with a rendering obligation on the context: the content is rendered with its teller's handle. Only `testimony` can carry it. |
| `in_confidence` | Teller-only, widened only for audience members with disclosure-qualifying [witness evidence](#witness-evidence). |
| `channel(C)` | Every audience member is a member of channel C under the connector roster ([Occasion restriction](#occasion-restriction)). |
| `include(S)` | Every audience member resolves to a member of `S`. |
| `exclude(S)` | No audience member resolves to a member of `S`. |

Principles compose by conjunction. Evaluation is universally quantified over the audience, so a group is eligible only when every member is. An unknown identity or an unresolvable condition denies. Partial rendering is permitted only where the object type defines a disclosure-safe projection, such as the [Event projection](events-and-roles.md#disclosure-safe-projection). Otherwise the whole object is suppressed.

`attributed` binds the context assembler, not the reply, which can still drop the attribution. Reply compliance is audited and measured, not enforced.

Each record kind gets its restriction from one rule. An Occasion's is defined under [Occasion restriction](#occasion-restriction). A `testimony` Attestation's follows the [testimony principle floor](#testimony-principle-floor), and an ArtefactReference's follows the same floor over its Occasion. Every other output, including `observation` and `derivation` Attestations, working notes, Perceptions, and derived Artefacts, has the conjunction of the principle its Activity recorded, such as `include([operator])` for an operator note, and its [ancestry](#influence-from-context-manifests) restriction.

## Witness evidence

An Occasion records witness evidence rather than an unqualified participant set. This fixes #123, where the present set conflates channel audience with participation ([coverage](coverage.md#issues)). The connector distinguishes availability from witnessed presence and preserves `availability ⊇ presence`.

Each witness item names a person or platform stub, an assurance kind, its source, recorded time, and a mandatory scope: an utterance span, a content-part range, an ArtefactReference, a delivery or acknowledgement target, or a whole Occasion. Whole-Occasion scope needs an assurance kind registered for it and is never inferred from participation.

| Assurance kind | Meaning | May widen disclosure? | May suppress independence? |
|---|---|---:|---:|
| `teller` | The person supplied the Attestation. | yes | yes |
| `active_participant` | The connector demonstrates contribution to the relevant span. | yes | yes |
| `explicit_acknowledgement` | The person acknowledged the content or relevant span. | yes | yes |
| `delivered` | The connector reports delivery. | no | yes |
| `channel_member` | The person was a member of the channel. | no | yes |
| `unknown` | Presence cannot be established. | no | no |

A versioned disclosure policy starts with the teller and widens only for `active_participant` or `explicit_acknowledgement`. Widening is scoped: it applies only when the witness scope covers every source locator supporting the Attestation. Evidence for one part of a compound Occasion does not license another, and partial coverage falls back to teller-only. A connector-specific policy may admit another assurance kind only after its semantics are demonstrated.

A versioned exposure policy derives a conservative upper bound from all five positive kinds. Only [dependence analysis](belief.md#dependence) reads exposure, and it can only suppress apparent independence. Channel membership never licenses disclosure of an `in_confidence` Attestation. Evidence that only suppresses may be conservative, while evidence that licenses a flow must be demonstrated. Dynamic presence remains an admitted evidence gap, so teller-only is the fallback ([research lane](research/2026-07-24/lanes/provenance-privacy.md#implications-for-zuihitsu)). Milestone 1's `witness-assurance-audit` covers every current consumer of the present set ([evolution](evolution.md#experiments)).

## Occasion restriction

Every Occasion has a restriction: the audience that could see it.

- An inbound direct message's restriction is `include` of its parties.
- An inbound channel message's restriction is `channel(C)`. The connector declares whether the platform shows history to members who join later. If it does, membership is evaluated at read time. If it does not, the roster at receipt applies.
- An outbound Occasion's restriction is the audience it was delivered to, which passed the [pre-delivery check](#pre-delivery-check).

An Occasion rendered into a context contributes its restriction. A delivered outbound Occasion rendered as conversation history contributes only its delivered-audience restriction, and the restriction projection stops there without following its ancestry. Without this stop, one confidence read early in a conversation would restrict every later write there and the store would drift towards teller-only. Milestone 1's `taint-breadth` experiment measures whether the stop suffices ([evolution](evolution.md#experiments)).

Source-lane search results are Occasion text parts and render under their Occasion's restriction ([query surface](query-surface.md#search-lanes)).

An entity handle minted, or a relation name coined, in a context carries that context's restriction. It renders only to audiences that clear that restriction, until the name appears in an Occasion whose audience clears it or the operator publishes it. A published or cleared definition adds no restriction when rendered ([definition audience](relations.md#definition-audience)).

## Testimony principle floor

A `testimony` Attestation's principle has a floor: its source Occasion's restriction. The teller's own act sets it, not the restriction of the context that wrote the Attestation. The model may narrow it freely, for example to `in_confidence` or `exclude(S)`. Widening it needs a teller grant or an operator action.

A teller grant is a span by the same teller, cited in the proposal, that expresses permission to share, such as "feel free to tell people". The citation is checked like grounding: the span must exist and belong to that teller. A soft critic and audit sampling judge whether it expresses permission. The caller never sets a principle freely. It may only narrow the floor or cite a grant ([required caller decisions](write-surface.md#required-caller-decisions)).

The cost is direct: a fact told in a direct message stays in that direct message's audience unless widened. Per-person or per-conversation widening defaults, set by a teller or the operator, are deferred as option b ([deferred](#deferred)).

The teller is the participant who produced the cited span ([teller binding](write-surface.md#teller-binding)), so a relay's `relayed_from` source person is never its teller. Unchecked, the floor would let a model launder a confidence by filing it as testimony citing another participant's span. The [testimony-grounding hard critic](verified-write.md#hard-critics) closes that path. A claim that fails it may be proposed only as a `derivation` Attestation, which inherits its ancestry restriction.

Grounding may resolve a span through stored records, as when "my sister" resolves to Quinn through a kinship Assertion. "Resolves from the span" means exactly such a recorded resolution with its inputs listed. Those inputs are inputs: their restrictions are conjoined with the testimony principle.

## Pre-delivery check

Before an outbound Occasion is delivered, the restriction projection of the draft's manifest must admit every member of the delivery audience. The delivery audience is the conversation's current availability audience, not only its witnessed presence.

On failure, the draft is regenerated once with the offending manifest entries excluded, and both attempts are recorded. If the second draft also fails, the turn ends with a non-disclosing deferral whose text does not depend on either draft.

A [`wake_turn` Task](time.md#occurrence-and-task) is checked twice. Its note and arguments must clear the target conversation's audience when the Task is created. At firing, only content that clears the audience at that moment is rendered. A note that no longer clears is replaced by a non-disclosing marker, and the failure is recorded for the operator.

## Subject guard

The subject guard is an additional negative recipient predicate evaluated per Attestation. It never widens an audience. Under every candidate policy, a person is not shown what others told the agent about them unless scoped witness evidence shows that the person took part in or acknowledged the telling. A group audience that contains the person loses those facts too, because evaluation is universally quantified over the audience.

Three candidate policies share the algorithm below. Milestone 1's `guard-denial-rate` experiment decides between them ([evolution](evolution.md#experiments)).

| Policy | Difference from the algorithm |
|---|---|
| `subject-guard/v1` | None. Operator-told facts and agent observations are guarded like any other. |
| `subject-guard/v1-relaxed` | Exempts `public` testimony and operator-told `public` observations. |
| `subject-guard/v1-own` | As v1, and P may also see an `observation` or `derivation` about P when every input is P's own testimony, or when it was produced in a context whose audience was exactly P. The operator's own `include([operator])` notes about the operator fall under the second clause. |

The guard is the pure function `source_guard(query_scope, audience_member, attestation_id, frontier)`, returning `allow` or `deny` with an operator-only trace. The policies and the ResolutionEnvironment are those the log holds at `frontier`, so replay reproduces the decision. A missing, erased, invalidated, or unresolved Attestation denies.

`query_scope` has two forms. A search or structured Assertion read guards each candidate Assertion independently, so one protected candidate never denies an unrelated result. An Event read uses the Event as a candidate-set scope, because the roles of one Event must be evaluated together. Rendered values, support, rank, and visibility never narrow either scope, which breaks the circularity between discovering subjects and rendering an audience-safe Event.

1. Form the candidate set: the Assertion itself, or every non-erased role and attribute Assertion on the Event. Records in every lifecycle state count. A tombstone recording an erased person position contributes an opaque deny marker.
2. Extract protected positions: every person-valued position, subject or object, including every Event role whose filler range admits a person. Protection does not depend on role names. An unknown kind, a missing definition version, or erased coordinates contribute an opaque deny marker. Places, organisations, quantities, and literals are not protected.
3. Expand each handle through the identity composites at the frontier. An accepted disclosure-cleared composite contributes all its members. Every candidate, overlapping, or conflicting hypothesis contributes all possible members to the deny set and never to an allow. Unknown membership contributes an opaque marker. Expansion only adds.
4. Apply exceptions. A `testimony` teller is removed only for delivery of that teller's own Attestation, and only when its principle permits. Another person is removed only when `active_participant` or `explicit_acknowledgement` evidence identifies that resolved person and covers every source locator. An `observation` or `derivation` Attestation removes nobody without an Occasion witness edge, because its producer is provenance, not witness evidence; `v1-own` is the one extension. A relay's `relayed_from` source person gets no exception. Channel membership, delivery alone, and possible identity never qualify.
5. Deny when the audience member resolves to, is ambiguous with, or may be covered by an opaque marker for a remaining protected person. Otherwise return the transmission-principle result.

One teller's clearance cannot expose another teller's restricted support for the same Assertion. Support aggregation and the disclosure-safe Event projection run only after every Attestation has been guarded.

## Audience-safe state and zero residue

An uncleared input leaves no observable residue. Text, dates, counts, rankings, ordinals, omissions, queue activity, tool calls, and initiated actions are all observations. In the current system a visible link row carried a date from a withheld entry, because redaction is decided separately at each read path ([current-system evidence](research/2026-08-06/current-system-fixes.md#redaction-decided-per-read-path)). The audience is resolved centrally, before ranking ([query surface](query-surface.md)).

An Assertion's validity and lifecycle state also fold per audience. Each Attestation records the validity its source asserted. Each Assertion transition, meaning validity closure, supersession, retraction, and invalidation, names its evidence source: an Attestation, an Occasion, or an Activity. An audience sees the validity and state folded from the Attestations and transitions whose sources it can see. A public read therefore never renders a date or a closure that came from a confidence. Settlement is the same kind of per-audience projection ([belief](belief.md#settlement-and-withdrawal)).

A global support projection may exist for operator audit and may schedule restricted internal review, but it never feeds a conversational decision. Any ordinal, ordering, derived result, or action depends only on audience-safe inputs, or inherits the intersection of every influencing restriction and stays hidden unless that clears.

Zero residue applies non-interference. The research supports the information-flow framing. The central resolver and the compilation above are safety synthesis that scenario testing must establish.

## Influence from context manifests

Influence is broader than accepted Assertions: a rejected proposal changes a retry, and a working note carries a confidence into a later turn. It is not stored on objects. [The object model](statements.md#context-manifest) defines the manifest and its rendering choke point, and tool and job Activities record typed input and output edges.

An object's Activity ancestry is its producing Activity plus the ancestry of every object in that Activity's manifests and typed inputs. Three projections are computed over it.

- Restriction intersection: the conjunction of the restriction of every rendered record, meaning each Occasion's restriction, each Attestation's and ArtefactReference's principle, and each derived object's computed restriction. A delivered Occasion contributes only its delivered-audience restriction ([Occasion restriction](#occasion-restriction)). A rendered record whose restriction cannot be determined denies every audience. A `testimony` Attestation takes its [floor](#testimony-principle-floor) instead of this projection.
- Non-evidentiary marks: an object carries a mark when any manifest in its ancestry contains an object of the marked classification. The genesis mark is `episodic_reconstruction`, carried by generated episode Artefacts. Because ancestry only grows, nothing removes a mark, and original evidence in the same context does not cancel it. Marks, unlike restrictions, propagate through delivered Occasions. [The episodic wall](two-traces.md#the-episodic-wall) rejects a semantic write whose ancestry carries one, which would block semantic writes for the rest of any conversation in which an episode was read. That is an open question for the [generated-episodes reopen condition](two-traces.md#evidence-and-deferral).
- Access accounting: the set of manifest entries, counted per audience ([query surface](query-surface.md#access-accounting)).

These cover every Activity output, including rejected proposals: rejection does not erase influence, because the retry's manifest names what the rejection rendered.

Manifest completeness is the load-bearing invariant. A renderer that bypasses the choke point under-taints silently, and no later check can see the gap. The choke point is the only path from stored content to a model context, and the scenarios cover every renderer kind.

## Provenance and derivation

A computed conclusion is supported by a `derivation` Attestation ([object model](statements.md#attestation)). Perceptions and derived Artefacts record their producing Activity and inputs directly. A retry is a new Activity.

This extends PROV-shaped lineage with defeasible dependencies. PROV supports recorded lineage. Recomputed resolution environments and manifest-complete influence are this design's synthesis ([research lane](research/2026-07-24/lanes/provenance-privacy.md#the-derivationprovenance-record-failure-class-11-94-100)). "How do you know?" returns complete recorded lineage where it exists and an audit trace otherwise, never described as a proof.

A derivation records the frontier it read, and its ResolutionEnvironment is recomputed from the log at that frontier. Withdrawing an accepted hypothesis in that environment invalidates outputs produced under it ([identity](identity.md)). New evidence can mark an output as owing recomputation, and re-derivation is a later recorded Activity.

## Retraction and erasure

Retraction and supersession change folded state and keep content for audit. Erasure deletes governed payloads and is terminal: the envelope and erasure record remain, replay is deterministic over what survives, and replay never reconstructs an erased payload. Forgettable payloads beside an append-only envelope are an established event-sourcing pattern. Their combination with dependant invalidation is synthesis ([research lane](research/2026-07-24/lanes/provenance-privacy.md#5-forgetting-vs-append-only-reconciling-erasure-with-deterministic-replay)).

The model is sized for one operator, one agent, a few participants, and a server that is not publicly exposed. It guarantees that an erased payload never resurfaces, replay stays deterministic, shared bytes survive for a sharer who did not erase, and dependants are re-evaluated. It omits proof of destruction, safety while an uncontrolled copy exists, and multi-party authorisation. The envelope and payload split is the seam per-payload keys would need, so adding them is additive.

### Authority

| Requester | Authentication | Permitted scope |
|---|---|---|
| teller | Connector or operator binding to the Attestation's teller | Retract their own Attestations. Erase payload they supplied: their text parts, ArtefactReferences, and Attestations. Not another teller's records. |
| operator | Authenticated local operator authority | Retract or erase any governed record. Cannot represent an irreversible external effect as undone. |

A wider request is recorded as an ordinary Occasion for the operator to resolve, with no destructive action. An allowed erasure records requester, resolved scope, and frontier. A relay's `relayed_from` source person has no teller authority over the relay.

The erasure scope includes the governed text of the requesting Occasion by default, because a request such as "forget that I'm …" repeats the content it asks to remove. That text is the requester's own text part, so it lies within teller authority.

### Envelope and payload

Each event envelope stores type, version, stable IDs, times, routing metadata, and a salted commitment hash over its payload. The payload and its salt are stored separately, keyed by the envelope. Erasure deletes both and appends a tombstone. The hash chain over envelopes stays intact, so tamper evidence survives. Because the salt is gone, an erased low-entropy payload such as a date cannot be confirmed by hashing guesses against the commitment.

Artefact bytes are stored once per Artefact and retained while any reference is not erased ([object model](statements.md#artefact-and-artefactreference)). The content digest lives in the erasable payload, so an erased Artefact's tombstone keeps only its minted ID. Neither ID nor digest grants authorisation: every byte read checks the audience against an ArtefactReference.

### Dependant closure

Closure starts from the scoped records and invalidates dependants reached through typed dependencies: Attestation sources, derivation inputs, cited locators, Perception inputs, Task sources, and grounding resolution inputs. Milestone 1 measures closure size ([evolution](evolution.md#experiments)).

A dependant linked to an erased record only by being rendered in the same model call goes to operator review and is not invalidated. A delivered reply whose context rendered an erased record is one, since it may not contain the content. Closure does not recurse through later renders of that reply unless the operator erases it, which starts its own closure.

| Surface | Treatment |
|---|---|
| Text parts, ArtefactReferences, and Artefact bytes | Delete governed text, reference metadata, and bytes with no remaining reference. Invalidate locators and Attestations over the erased part. |
| Model calls whose manifest names an erased record | Delete rendered prompt, images, reasoning, reply, proposal content, and diagnostics, which contain the content. Manifest IDs, versions, timing, and outcome class survive. |
| Co-rendered dependants, including delivered replies | List for operator review. |
| Typed dependants | Fold to erased or invalidated, or re-record over surviving independent inputs as a new `derivation` Attestation. |
| Embedding vectors, indexes, projections, snapshots, and caches | Erase vectors with their payload, delete affected entries, and rebuild from survivors before serving. |
| Tasks | Cancel any Task sourced only from erased input. Record completed external effects without claiming reversal. |

A payload hidden from a caller is treated for that caller exactly as an erased one.

### Execution

Erasure, from a teller or the operator, runs inside the server as an exclusive operation without a process stop. It holds the single writer, and turns and workers are quiesced at their next boundary, so one pass reaches a fixed point. A worker result that read an erased record is recorded as stale ([off-turn work](off-turn.md#workers)).

1. Quiesce turns and workers at their boundaries, and resolve the scope and closure at the current frontier.
2. Append the erasure to the [ledger](#erasure-ledger-and-boot-reconciliation) and its second copy.
3. In one database transaction, append tombstones, delete payloads and salts, and cancel scheduled work.
4. Delete unretained Artefact bytes. Tombstones deny reads meanwhile.
5. Rebuild derived state from survivors, list co-rendered dependants for review, and resume turns and workers.

A crash before step 2 changes nothing and leaves the request pending. A crash after it is completed at the next boot, which treats a ledger entry without tombstones as it treats a restored backup. Deletion is reported complete only when every managed deletion succeeds, and a persistent failure is reported as blocked. Backup media may hold historical bytes until their retention expires. The ledger keeps them off every serving surface, and the report says so.

### Erasure ledger and boot reconciliation

The erasure ledger is a small append-only file, kept apart from the event log, that records each erasure as stable IDs and ledger positions. It never holds content or unkeyed content digests. Each entry carries the hash of the previous one, and every erasure copies the file to a second location.

Every boot reconciles the store against the ledger before serving. An erased ID without its tombstone has its closure re-applied at the current frontier. A tombstoned payload or unretained Artefact still present is deleted. A snapshot or cache older than the ledger head is scrubbed or rejected, and projections rebuild. Restoring an old backup therefore needs no special procedure: the restored log boots and reconciliation re-applies every later erasure.

When one ledger copy is a verified prefix of the other, the longer wins. When the ledger is missing, both copies fail verification, or the copies fork, boot is refused. The operator can override only with an explicit flag acknowledging that erased content may resurface. The override is recorded as an operator Activity and starts a new ledger from the log's tombstones.

The ledger reconciles one serving store. Concurrently serving copies are outside the design; erasure is one operation a future sync layer would coordinate ([overview](overview.md)).

### External copies

Eval packages, exports, console downloads, and debug captures built from a real log leave the store's control. Creating one records a copy record naming its IDs, recipient or location, and time, and is refused when an input does not clear the copy's audience. An erasure intersecting a copy reports `external_copy_unresolved` for it, and the operator removes it by hand.

## Deferred

- Consent, purpose-limitation, and reciprocity principles, as additive definitions. Consent needs a scoped, expiring, and revocable consent record. Purpose limitation needs an execution-purpose model, not a caller-supplied string. Reciprocity needs a stable definition of comparable disclosure.
- Option b widening defaults, per person or per conversation. Reopens when `principle-assignment` measures the option-a recall cost as too high.
- Connector-specific assurance kinds that widen disclosure.
- Inter-agent exchange: quoting, provenance, and revocation of claims from another agent ([evolution](evolution.md)).
- Per-payload keys, proof of destruction, and multi-party erasure authorisation.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `dm-occasion-restriction` | A later context renders Rowan's direct message. | Its outputs are restricted to `include([rowan, agent])`. |
| `channel-late-joiner` | Sam joins a channel after a message is recorded. | With declared history visibility, Sam clears `channel(C)`; without it, Sam fails the roster at receipt and is denied. |
| `delivered-reply-no-accumulation` | Turn 3 reads a confidence and replies to its teller; turn 7 renders the reply as history and writes a note. | The note's restriction is the reply's delivered audience, without the confidence's. |
| `coined-handle-restricted` | The agent mints `person/quinn` in Rowan's direct message; a channel read lists entities. | The handle is withheld from the channel until used there or published. |
| `dm-fact-stays-in-dm` | Rowan states a fact in a direct message without a grant; Sam asks in a channel. | The principle is the direct message's restriction; the channel read omits it. |
| `grant-other-teller-rejected` | A proposal widens Rowan's testimony citing Sam's "feel free to share". | The widening is rejected; the principle stays at the floor. |
| `free-widening-rejected` | A proposal sets `public` on direct-message testimony with no grant. | Rejected with a teachable error naming the floor. |
| `testimony-takes-teller-principle` | Rowan speaks in a channel whose context also rendered an `include([operator])` note. | The `testimony` principle is `channel(C)`, without the note's restriction. |
| `pre-delivery-regenerates` | A draft's manifest holds a record restricted from one channel member. | The draft is withheld, a second excludes that entry, and both are recorded. |
| `pre-delivery-defers` | Both drafts fail the check. | The turn ends with the fixed non-disclosing deferral. |
| `guard-subject-is-teller` | P is an Assertion's subject and only teller. | `allow` for P's Attestation only. |
| `guard-participation-alone` | Q tells, under `attributed`, an Event with P in a person-valued role; P has no witness evidence. | `deny` under all three policies. |
| `guard-object-position` | Q tells a fact with P as object; the caller is P. | `deny` under all three policies. |
| `guard-public-testimony-policy` | Q tells a `public` fact about P; P asks without witness evidence. | `deny` under `v1` and `v1-own`; `allow` under `v1-relaxed`. |
| `guard-group-contains-subject` | A group includes P; an operator-told `public` observation concerns P. | `deny` under `v1` and `v1-own`; `allow` under `v1-relaxed`. |
| `guard-own-inputs` | A `derivation` about P has only P's testimony as inputs; the caller is P. | `allow` under `v1-own` only. |
| `guard-operator-self-note` | An operator note about the operator was written with the operator as sole audience. | `allow` under `v1-own` only. |
| `guard-per-assertion-scope` | A search for P matches one Assertion about P and unrelated Assertions. | Only the Assertion about P is denied. |
| `guard-event-candidate-set` | An Event has roles for P and R; the caller is P without evidence for R's telling. | The whole Event is denied before projection. |
| `guard-tentative-identity` | A handle is in accepted `[P1,P2]` and candidate `[P2,P3]`; the caller may be P3. | `deny`. |
| `guard-hidden-role` | The caller fills an Event's confidential person role. | `deny`, although the projection would omit the role. |
| `guard-erased-coordinate` | A tombstone records an erased person role. | `deny` through the opaque marker. |
| `guard-agent-observation` | A `public` agent `observation` about P has no witness edge; the caller is P. | `deny` under all three policies. |
| `guard-relayed-source` | Rowan relays Quinn's words about Quinn; the caller is Quinn. | No teller exception; `deny` without witness evidence. |
| `silent-channel-member` | An `in_confidence` claim is told before a silent member. | The member is outside the disclosure set, and their exposure suppresses independence. |
| `compound-partial-witness` | Acknowledgement covers one of two cited text parts. | Teller-only. |
| `unknown-audience-member` | One group member is unresolved. | Every restricted record is denied to the group. |
| `withheld-date-residue` | A visible link row's date comes from a withheld Assertion. | The date is not rendered, and no gap reveals it. |
| `hidden-closure-residue` | A confidence closed a `public` Assertion's validity. | A public read shows open validity and no closure date. |
| `restriction-intersection` | A call renders `include(S)` and `public` Attestations and yields a derived Assertion. | The derived Assertion is restricted to `include(S)`. |
| `rejected-proposal-influences-retry` | A proposal from restricted input is rejected and retried. | The retry's manifest names the rejected content, and its output carries the restriction. |
| `mark-crosses-delivery` | A reply written after reading an episode is rendered into a semantic write. | The episodic wall rejects it. |
| `teller-erases-own` | A teller erases their Attestation of a two-teller Assertion. | The Assertion and the other Attestation survive. |
| `relayed-source-cannot-erase-relay` | Quinn asks to erase Rowan's relay of Quinn's words, as another teller's record. | No destructive action; the request is recorded for the operator. |
| `erasure-request-text-included` | A teller writes "forget that I'm moving to Lisbon". | The request's text part is erased with the original claim. |
| `erasure-online-quiesce` | An erasure arrives during a turn and a worker job. | Both pause at a boundary, and a worker result that read the record is stale. |
| `erasure-typed-dependant` | A derived Assertion has one erased and one surviving input. | The old `derivation` Attestation is invalidated and a new one over the survivor is recorded. |
| `erasure-co-rendered-review` | A note was written in a call that rendered the erased record without citing it. | The call's payload is deleted; the note stays live and is listed for review. |
| `erasure-delivered-reply-reviewed` | A delivered reply's context rendered a later-erased record; a later turn rendered the reply. | The call payload is deleted, the reply is listed for review, and the later turn is untouched. |
| `erasure-cancels-task` | One Task is sourced only from erased input; another already sent a message. | The first is cancelled; the sent message is recorded as a completed effect. |
| `erased-payload-unconfirmable` | An erased payload was a short date. | Hashing guesses cannot match the commitment. |
| `erasure-crash-after-ledger` | A crash follows the ledger append. | The next boot completes the erasure before serving. |
| `pending-live-blob-deletion` | Blob deletion fails after erasure. | Reads stay denied, the report says blocked, and boot retries. |
| `restore-old-backup-reconciles-at-boot` | A backup from before an erasure is booted. | Boot re-applies the erasure before serving. |
| `missing-ledger-refuses-boot` | Both ledger copies are missing or unverifiable. | Boot is refused; the override flag is recorded as an operator Activity. |
| `external-copy-bounded` | An erasure intersects an exported eval package. | The report lists it as `external_copy_unresolved`. |
