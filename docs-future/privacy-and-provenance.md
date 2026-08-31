# Privacy and provenance

Privacy constrains information flow. It is not a tag on semantic content. [Attestations and direct SourceAuthorities](statements.md) carry transmission principles and source locators or typed observation edges. [Assertions](statements.md) receive audience-safe testimonial support from visible Attestations, while a direct Assertion remains usable only through its visible live SourceAuthority. [Perceptions and artefact references](artefacts-and-perceptions.md) carry the same restrictions through multimodal work. A [Derivation](statements.md) records the restrictions and resolution environment under which its output was produced.

The design rule is to record witness and influence evidence broadly and immutably, then interpret it under narrow, versioned policies. Contextual integrity supports transmission principles as conditions over an information-flow trace, but the exact vocabulary and compilation rules below are design decisions ([research lane](research/2026-07-24/lanes/provenance-privacy.md#4-contextual-integrity-transmission-principles-as-first-class-data)).

## Transmission principles

Each Attestation carries one registered, versioned principle. Genesis reserves the representation for later principles, but the initial evaluator accepts only semantics it can decide cheaply and fail closed.

| Initial principle | Compiled condition |
|---|---|
| `public` | Any authenticated audience is eligible, subject to other authorisation and erasure rules. |
| `attributed` | Eligible only when the rendered result preserves the teller attribution. |
| `demonstrated_witness_only` | Every audience member must have qualifying witness evidence for the source Occasion. `in_confidence` is an agent-facing name for this principle. |
| `include(S)` | Every audience member must resolve to a member of `S`. |
| `exclude(S)` | No audience member may resolve to a member of `S`. |
| `consent(scope, expiry, revocation)` | A signed consent record must include every audience member and the proposed flow; it must be unexpired and unretracted. |

Principles compose by conjunction. Evaluation is universally quantified over the current audience, and an unknown identity or unresolvable condition denies. A group is eligible only when every member is eligible. Partial rendering is permitted only where the object type defines a disclosure-safe projection; otherwise the whole object is suppressed.

Reciprocity and purpose limitation remain gated extensions. Reciprocity needs a stable definition of comparable disclosure. Purpose limitation needs an execution-purpose and obligation model that constrains downstream use, not a string supplied by the caller. General temporal logic is not on the read path.

## Witness evidence

An Occasion records witness evidence rather than an unqualified participant set. Each item names a person or platform stub, an assurance kind, its connector or operator source, recorded time, and a mandatory witness scope. The scope is one utterance span, an ordered typed-content-part range, an ArtefactReference, a delivery or acknowledgement target, or an explicitly certified whole Occasion. Whole-Occasion certification requires a connector or operator assurance kind registered for that purpose; it is never inferred from participation.

| Assurance kind | Meaning | May widen disclosure? | May suppress independence? |
|---|---|---:|---:|
| `teller` | The person supplied the Attestation. | yes | yes |
| `active_participant` | The connector demonstrates contribution to the relevant span. | yes | yes |
| `explicit_acknowledgement` | The person acknowledged the content or relevant span. | yes | yes |
| `delivered` | The connector reports delivery. | no | yes |
| `channel_member` | The person was a member of the channel. | no | yes |
| `unknown` | Presence cannot be established. | no | no |

A versioned disclosure policy derives the narrow disclosure set. It starts with the teller and widens only for `active_participant` or `explicit_acknowledgement`; a future connector-specific policy may admit another assurance only after its semantics are demonstrated. Widening applies only when the witness scope covers every source locator supporting the Attestation. A compound Occasion with evidence for one span or content part does not license another. Partial coverage falls back to teller-only. A versioned exposure policy derives a conservative upper bound from teller, participation, acknowledgement, delivery, and channel membership. Exposure is read only by dependence analysis and can only suppress apparent independence. Channel membership never licenses disclosure.

This preserves the asymmetric rule established by the earlier design: evidence that only suppresses may be conservative, while evidence that licenses a flow must be demonstrated. Dynamic presence remains an admitted evidence gap, so teller-only is the fallback ([research lane](research/2026-07-24/lanes/provenance-privacy.md#implications-for-zuihitsu); [confidence register](confidence.md#privacy-and-provenance)).

## Subject guard

The subject guard compiles into an additional negative recipient predicate for every Attestation. It never widens an audience.

The compiler is the pure function `source_guard(query_scope, audience_member, source_evidence_id, policy_version, resolution_environment, source_head)`, where `source_evidence_id` resolves to exactly one Attestation, SourceAuthority, or Derivation whose typed output is the Assertion being considered. `query_scope` is one of: an exact Assertion ID; an exact Event ID; an exact composite-resolution ID; or a bounded candidate set already selected by non-semantic keys such as namespace, source sequence interval, and registered relation or Event-type IDs. It may not be narrowed by rendered values, support, rank, visibility, or the guard itself. The function reads the immutable log prefix through `source_head` and returns only `allow` or `deny` plus an operator-only decision trace. A missing, wrong-type, erased, invalidated, or unresolved source-evidence record returns `deny`.

1. Form the candidate record set. For an Assertion scope, include that Assertion. For an Event scope, include every non-erased role and attribute Assertion whose subject or object is that Event ID. For a composite scope, take the union for every member Event in the recorded composite. For a bounded candidate scope, take the union for every selected Assertion, Event, and live composite that the query could return before audience filtering. Include `candidate`, `settled`, `superseded`, `retracted`, and `invalidated` records when their retained coordinates can identify a protected person; exclude only terminally erased coordinate payload, which contributes an opaque deny marker whenever its tombstone says that a protected person position was erased. This set is independent of the final projection.
2. Extract protected subject positions under `policy_version`. They are every Proposition subject whose registered entity-kind constraint admits a person; every person-valued Event role designated `participant`, `actor`, `patient`, `experiencer`, or a registered subrole whose parent is one of those roles; and every person-valued attribute definition marked `subject_guarded`. A definition with unknown kind, missing historical version, or erased coordinates contributes an opaque deny marker. Non-person locations, organisations, quantities, and literals do not.
3. Expand each extracted handle through the identity component of `resolution_environment`. An accepted disclosure-cleared composite contributes all member stubs. Every overlapping, conflicting, or candidate hypothesis touching an extracted handle contributes all possible member stubs to the deny set but never to an allow decision. Unknown membership contributes an opaque deny marker. Expansion is monotone and never removes a subject.
4. Evaluate the named source evidence's transmission principle or, for a Derivation, its output principle and InfluenceEnvelope intersection. For an Attestation, remove its teller from the deny set only for delivery of that teller's own Attestation and only when the principle itself permits that delivery. Remove another protected person only when disclosure-qualifying `active_participant` or `explicit_acknowledgement` evidence identifies that same resolved person and covers every source locator supporting this Attestation. For a SourceAuthority or Derivation, there is no teller exception: an authenticated agent, operator, tool, or producing Activity is provenance and authority, not evidence that a protected subject witnessed or consented to the flow. A non-testimonial source may remove a protected person only through the same explicit scoped witness evidence, and one with no Occasion witness edge removes nobody. The fact that a subject participated in an Occasion is not enough unless the scoped witness evidence demonstrates coverage. Channel membership, delivery alone, and possible identity do not qualify.
5. Return `deny` when `audience_member` resolves to any remaining protected stub, when identity resolution is ambiguous with one, when the source evidence is not live and readable, or when an opaque deny marker applies to the audience's possible identity. Otherwise return the source evidence's transmission-principle result. Only after every Attestation and SourceAuthority is evaluated may the resolver aggregate support and construct a disclosure-safe Event projection. Omission, incomplete-shell, and suppression operate only in this final phase.

| Guard vector | Candidate closure | Audience and exception | Expected result |
|---|---|---|---|
| Subject is the teller | The Assertion subject resolves to `person/P`; its only Attestation teller is `person/P`. | The principle permits teller delivery. | `allow` for this Attestation only. Another teller's Attestation receives no exception. |
| Subject participated | An Event has `person/P` in a protected participant subrole and another person tells the claim. | `person/P` appears in the Occasion, but no scoped active-participant or acknowledgement evidence covers every locator. | `deny`; participation metadata alone does not license disclosure. |
| Multi-subject Event | Protected roles resolve to `person/P` and `person/Q`; a public role would remain after projection. | The caller is `person/Q`; no qualifying witness exception exists. | `deny` before projection. Omitting Q's hidden role cannot turn the Event into an allowed result. |
| Tentative overlapping identity | A protected handle participates in accepted composite `[P1,P2]` and overlapping candidate `[P2,P3]`. | The caller resolves possibly to `P3`. | `deny`; candidate expansion widens only the deny set. |
| Hidden role omitted from final projection | A candidate Event contains a retained confidential participant role and an independently public attribute. | The caller is the confidential participant. | `deny` from the pre-render closure even if the final projection would omit the role. |
| Erased protected coordinate | A retained tombstone says that a person-valued protected role was erased, but its handle is unavailable. | The caller's non-membership cannot be proved. | `deny` through the opaque marker; an authorised operator may inspect only the decision trace and permitted tombstone. |
| Direct agent observation | A SourceAuthority supports an Assertion whose subject is `person/P`; it has no Occasion witness edge. | The caller is `person/P`; actor is the agent and principle is public. | `deny`; the agent actor receives no teller exception and public does not bypass the subject guard. |
| Direct operator assertion | A SourceAuthority supports an Assertion whose subject is not the operator and carries `include([operator])/v1`. | The caller is the authenticated operator. | `allow` only when the operator is not in the protected closure; another protected caller is denied. |
| Derived tool observation | A Derivation outputs an Event role Assertion naming `person/P`; its tool observation and Activity have no person witness evidence. | The caller resolves possibly to `person/P`. | `deny`; tool execution, Derivation authority, and a public output principle do not establish witness or consent. |

The guard is evaluated per Attestation, SourceAuthority, or Assertion-producing Derivation before support aggregation or direct-source use. One teller's clearance cannot expose another teller's restricted support for the same Assertion, and direct actor authority cannot create a subject exception. The candidate-set and expansion rules are monotone, non-rendering, and fail-closed. They remove the circular dependency between subject discovery and audience-safe Event rendering. The policy version, scope, source head, source-evidence ID, and ResolutionEnvironment are recorded because later reinterpretation could disclose existing history.

## Audience-safe support and zero residue

An uncleared input must leave no observable residue. Text, dates, counts, rankings, ordinals, omissions, queue activity, tool calls, and initiated actions are all observations. The current system demonstrated the problem when a visible link row carried a date from a withheld entry even though no hidden text was rendered ([current-system evidence](research/2026-08-06/current-system-fixes.md#redaction-decided-per-read-path)). Central audience resolution is therefore required before rendering.

The store may compute a global epistemic support projection for operator audit, but it is not an audience-safe input to conversational decisions. For an audience, actionable support is recomputed from visible Attestations after subject-guard and transmission evaluation. Any ordinal, ordering, recommendation, Derivation, or initiated action must either:

- depend only on audience-safe inputs; or
- inherit the intersection of every influencing input's transmission restrictions and remain hidden unless that intersection clears.

This rule removes the hidden-endorsement leak. Hidden evidence cannot change a visible rank or ordinal while its existence remains concealed. Global support may schedule restricted internal review, but the review item and every result retain the hidden influence envelope.

Descriptions are derived only from inputs whose principle is `public` and that also pass subject guards, erasure, retention, and every other authorisation check, because their ordinary surface has no attribution or audience gate. `public` is never a bypass. Generated episodes and other audience-gated syntheses inherit the intersection of their inputs and retain structural attribution. Neither is evidence independent of its sources.

Zero residue is this design's application of non-interference. The research supports the information-flow framing; the complete compilation and central resolver are safety synthesis that require scenario testing ([research lane](research/2026-07-24/lanes/provenance-privacy.md#4-contextual-integrity-transmission-principles-as-first-class-data)).

## Influence envelopes

Influence is broader than accepted Assertions. Every Activity and derived result records an InfluenceEnvelope containing the typed input-edge IDs, input transmission-principle versions, intersected restriction expression, audience-decision ID, purpose definition/version, ResolutionEnvironment, source head, and any erasure or retention dependencies. This applies to:

- model prompts and model-call records;
- accepted, rejected, amended, and dropped proposals;
- critic diagnostics and retry context;
- queue items, leases, poison records, and working notes;
- Perceptions, OCR, captions, embeddings, and visual descriptions;
- Derivations, support calculations, and negative query results;
- descriptions, generated episodes, and other summaries;
- transformed or generated Artefacts;
- eval fixtures and packages built from real records.

Rejection does not erase influence: a rejected proposal may change a retry or teach a later decision what not to say. Implementations may compact payloads, but they must preserve enough typed lineage and restriction data to reproduce authorisation and erasure closure. Access is accounted when content is rendered into model or tool context, including retries, not when it merely appears in a hidden candidate set.

## Provenance and derivation

A versioned Activity executes work and can produce zero or more Derivations. Each Derivation is a separate immutable result and lineage record, not a note attached after a conclusion. Its typed inputs may include Assertions, Perceptions, tool or direct observations, aggregates, negative query results, and policy or schema definitions. It records implementation and criterion versions, ontology versions, audience and support policies, identity-resolution environment, assumptions, source head sequence, one typed output, and influence envelope. A retry is a new Activity and cannot overwrite an earlier Derivation. See the canonical [Derivation definition](statements.md#derivation).

This extends the PROV-shaped entity/activity/agent lineage with defeasible dependencies. PROV supports recorded lineage, but resolution environments and complete influence taint are this design's synthesis ([research lane](research/2026-07-24/lanes/provenance-privacy.md#the-derivationprovenance-record-failure-class-11-94-100)). “How do you know?” returns complete recorded lineage where available and an audit trace otherwise; it is not described as a proof.

Identity and Event resolution are not represented by a single merge stamp. A Derivation names the complete resolution environment it read. Withdrawal of any accepted hypothesis invalidates outputs produced under that environment. New evidence can mark an output as owing recomputation; weakened or withdrawn support can mark it for re-evaluation. The fold remains deterministic, while model-driven re-derivation is a later recorded Activity.

## Retraction and erasure

Retraction and erasure are separate appended operations.

- Retraction or supersession changes folded live state while retaining content for audit. Retracting an Attestation removes that teller's support, not the Assertion or other Attestations.
- Erasure deletes governed payloads. The envelope and the authorising erasure record remain, and replay is deterministic over what survives.

Forgettable payloads beside an append-only envelope are an established event-sourcing pattern. Their combination with derivation invalidation is this design's synthesis ([research lane](research/2026-07-24/lanes/provenance-privacy.md#5-forgetting-vs-append-only-reconciling-erasure-with-deterministic-replay)).

The erasure model is sized for the deployment the successor serves: one operator, one agent, a small number of participants reached through connectors, and a server that is not publicly exposed. It keeps every property the agent's behaviour depends on. An erased payload never resurfaces, replay stays deterministic, shared bytes survive for a sharer who did not erase, and derived conclusions are re-evaluated. It omits the machinery that only assures a party the deployment does not have: cryptographic proof of destruction, safe operation while an uncontrolled copy exists, and multi-party authorisation. The envelope and payload split below is the seam a later multi-tenant deployment would need for per-payload keys, so adopting them is additive.

### Authority

| Requester | Authentication | Permitted scope and operation |
|---|---|---|
| teller | Connector or operator binding to the Attestation identity | Retract their own Attestations. Erase payload they supplied: their text parts, ArtefactReferences, and Attestations. Cannot erase another teller's records or references. |
| operator | Authenticated local operator authority | Retract or erase any governed record. Cannot represent an irreversible external effect as undone. |

A request wider than a teller's own supply is recorded as an ordinary Occasion and resolved by the operator. The operator's decision is an immutable authorization record naming requester, resolved scope, decision, and source head. Closure accepts only an `allowed` decision. A `denied` decision causes no destructive action and is the only form of hold.

### Storage classes

The erasure and restore contract distinguishes three storage classes:

1. `managed live state` contains event payload rows, live Artefact bytes, model and proposal records, derived records, indexes, projections, snapshots, caches, and scheduled work controlled by the running deployment;
2. `managed restore material` contains payload and blob backups that grant no read authority and can enter a running deployment only through the restore procedure;
3. `external copies` contain exports, API or console downloads, connector-delivered content, debug captures, eval packages, test captures, and other copies outside restore control.

The genesis freeze fixes this storage baseline. A pre-genesis implementation may replace its candidate representation while preserving the evidence that tests the contract.

- Each event envelope stores type, version, stable IDs, times, routing metadata, and a commitment hash over its payload. The payload lives in separate managed live state keyed by the envelope. Semantic erasure deletes the payload row and appends the tombstone. The hash chain over envelopes remains intact, so tamper evidence survives deletion.
- Artefact bytes are stored once per Artefact. The Artefact's retention projection is the set of its live authorised references. Managed live bytes are deleted when that set becomes empty and never while a member survives.
- Encryption at rest is a deployment option with no semantic content. It does not change erasure or restore semantics.
- The current authoritative ledger advances through monotonically ordered tombstone positions. It is kept beside the log with the serving deployment and is included in every backup. Every snapshot and backup records the ledger position current at creation.
- Managed restore material never grants read authority. The restore procedure filters it through the current authoritative ledger before any payload enters managed live state. An older payload backup is usable only because the current authoritative ledger filters erasures recorded after that backup. A backup's bundled ledger is candidate restore material and never establishes current authority by itself. Restore remains blocked when the serving deployment's current authoritative ledger cannot be verified.
- Managed restore material may retain historical bytes until its ordinary retention process removes them. The operation reports that state explicitly and does not describe backup media as physically deleted. The ledger prevents those bytes from restoring authority or reaching a serving surface.
- External copies participate in bounded accounting rather than managed deletion closure. The deployment records scope, recipient or owner, control class, and notification state. An intersecting erasure reports `external_copy_unresolved` rather than claiming external deletion.

### Restore

A restore acquires the single-writer log lock before reading the current authoritative ledger or touching managed live state. It holds the lock through filtering, rebuilding, and the atomic serving transition. The lock excludes ledger publication from authoritative-ledger acquisition through serving activation, so the applied position cannot go stale between filtering and serving. Failure to acquire the lock, or loss of the lock before serving opens, leaves serving blocked. Restoring one instance into two serving deployments is forbidden: with two stores neither ledger is authoritative, and no protocol recovers one.

1. While holding the lock, the restore obtains the serving deployment's current authoritative ledger at position `N`. A backup's bundled ledger copy is candidate restore material only; the deployment's own ledger, kept beside the log, is the authority. When the current authoritative ledger cannot be verified, the restore applies nothing and produces no rebuild pass. It releases any held lock while serving remains blocked, then records the terminal blocked attempt with no acquired snapshot or applied position.
2. While holding the lock and blocking serving, the restore applies every tombstone through `N`, removes the affected payloads and Artefacts, and rebuilds projections without opening reads, jobs, or model or tool input.
3. Serving opens in one atomic transition after the rebuild completes. The restore releases the lock only after serving is safe and open. The applied position equals the authoritative position at that transition because the lock excluded publication throughout.

The `restore-old-backup-current-ledger` and `restore-missing-current-ledger` fixtures exercise the two reachable restore outcomes: a backup older than the newest erasure is filtered successfully, while an unverifiable ledger leaves serving blocked. A ledger that advances during a restore is not a schedulable state under the lock, so no fixture exercises it.

### Execution and completion

Erasure is an offline operator action. It takes the single-writer log lock, so the agent is stopped or quiesced and nothing appends concurrently. An in-flight turn is aborted by cooperative supersession at its next boundary and recorded as aborted. One pass over the dependency graph therefore reaches a fixed point for the locked source head.

1. The operation takes the lock and resolves the authorisation record and scope at the current head.
2. It traverses the log from the scoped records to every payload that depends on them: consumed-reference edges, Derivation input edges, InfluenceEnvelopes, locators, and job and Task arguments. Closure contains the scoped records and every dependant with no surviving independent lineage.
3. One database transaction appends terminal tombstones and authoritative-ledger entries, removes managed database payload rows, cancels scheduled work, and marks unretained Artefacts inaccessible.
4. Physical blob deletion from managed live stores may occur outside that database transaction. Tombstones and the retention projection deny reads and Activities while deletion is pending. Recovery retries deletion without restoring reference authority.
5. The operation rebuilds indexes, projections, snapshots, and caches from surviving payloads. It releases the lock only after the semantic commit and required rebuild complete.

A crash before the database transaction leaves the store unchanged and the request pending. A crash after it leaves semantic erasure durable. Recovery repeats projection rebuild and any pending physical deletion. The operator action does not report physical deletion complete until every promised managed-live deletion succeeds. An irrecoverable deletion failure for bytes under deployment control is `blocked`, not an external-copy bounded result.

Semantic erasure removes authority and access from managed live state and advances the authoritative ledger. Physical-deletion completion applies to live blob stores and other managed-live copies the deployment promises to delete. Managed restore material remains non-authoritative until ordinary media expiry, and bounded external-copy reporting is reserved for copies outside deployment control.

### Managed-live closure and external-copy accounting

An allowed erasure computes deletion closure over five managed-live surfaces. The final row is a separate external-copy accounting surface and does not participate in deletion closure. The last column also restates audience resolution ([subject guard](#subject-guard), [influence envelopes](#influence-envelopes)): a payload hidden from a caller is treated for that caller exactly as an erased one.

| Surface | Storage class | Payload treatment | Envelope metadata that survives | Dependants and hidden input |
|---|---|---|---|---|
| source payloads: text parts, ArtefactReferences, and Artefact bytes | managed live state | Delete governed text and reference metadata. Delete bytes only when no surviving authorised reference retains them. | Record IDs, types, times, permitted digests, authorisation decision, and tombstone. | Invalidate locators and Attestations over the erased part. Retain independent references unchanged. Deny rendering and source query before support aggregation. |
| model and proposal records: requests, responses, proposals, and critic diagnostics | managed live state | Delete prompt parts, rendered images, reasoning, replies, candidate content, and diagnostic text. | Activity and proposal IDs, versions, timing, outcome class, InfluenceEnvelope, and tombstone. | Invalidate outputs and retry context. A retry starts from surviving inputs only. Deny call construction and retry-context rendering. |
| derived records: Perceptions, Derivations, derived Artefacts, and solely grounded Assertions or Attestations | managed live state | Delete observation output, derived bytes, negative-result detail, and payload with no surviving lineage. | Stable IDs, transition class, pipeline or criterion version, and input tombstones. | Fold to `erased` or `invalidated`. Re-record a survivable conclusion as a new Derivation over independent authorised inputs. Deny output influenced by uncleared input. |
| indexes, projections, snapshots, and caches | managed live state | Delete vectors, lexical rows, thumbnails, materialised rows, snapshot copies, and cache entries. | Projection version, sequence bound, and ledger position. | Rebuild from survivors before serving. Deny insertion into an audience-broader index. Hidden rows cannot affect visible rank. |
| scheduled work: Tasks, Triggers, jobs, and pending actions | managed live state | Delete arguments, conditions, and message bodies derived only from erased input. | Stable IDs, cancellation transition, and outcome class. | Cancel before firing. Restore never re-arms. Record completed external effects and remediation without claiming reversal. Deny initiation from uncleared input. |
| external copies: exports, downloads, connector deliveries, debug captures, eval packages, and test captures | external copies | No system deletion guarantee. Record scope, recipient or owner, control class, notification state, and bounded result. | Copy-accounting record and `external_copy_unresolved` result where applicable. | Deny creation before delivery when a direct input is uncleared. Report bounded failure after delivery instead of claiming deletion closure. |

Managed restore material is not a closure row because it grants no read authority and cannot enter managed live state without filtering through the current authoritative ledger under the restore lock.

An Artefact's minted ID is stable internal identity. Verified digest assertions locate and deduplicate bytes; neither the ID nor a digest grants authorisation. Every byte read checks the caller's audience against an ArtefactReference before serving.

Derived outputs lose erased support and are re-evaluated against what remains. A conclusion that survives from independent authorised inputs may remain only as a newly recorded Derivation with replacement lineage. A conclusion that encodes only erased input cannot remain. The complete contract is required at the genesis freeze before the first real successor genesis. A later richer transmission policy cannot change what erasure means.

## Inter-agent status and revocation

An incoming inter-agent claim is a quoted Assertion with an Attestation naming the sending agent and preserving the travelling transmission principle and provenance chain. A remote content address is identity evidence, not an executable revocation channel.

Retraction uses an authenticated local subscription/status protocol. The local store records the remote issuer, stable remote Attestation identity, verification material, a monotonically increasing issuer notice sequence, the last verified status, and each signed revocation notice. Each subscription records a freshness lease, polling deadline, allowed clock skew, and last successful check. A notice with an old or duplicate sequence is idempotently ignored; a gap quarantines status pending fetch.

When the freshness lease expires, the fold moves from `current` to `stale` at the recorded deadline. Stale remote support is removed from every actionable support, ranking, Derivation, and initiated-action projection by default. Dependent Derivations become invalid or quarantined until recomputed without it. Ordinary conversational reads may show only an explicitly stale, non-actionable quoted record when its transmission principle still clears; they cannot present it as current support. A valid revocation appends a local Attestation-retraction transition. Key rotation is accepted only through a signed continuity or operator-authorised replacement record, and replay applies the same notice sequence and lease deadlines. Polling recovery can restore `current` only with a newer signed status. The remote system cannot rewrite local history, and a bare content hash never causes propagation by itself.
