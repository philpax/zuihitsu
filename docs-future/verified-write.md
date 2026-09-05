# Verified writes

A verified write is a durable proposal transaction. A model or deterministic caller proposes typed records. Hard critics decide whether those records are structurally admissible. The critics do not establish truth.

[The assertion model](statements.md) owns the identities and lifecycles of Occasion, Activity, Proposition, Assertion, Attestation, and Derivation. [Artefacts and perceptions](artefacts-and-perceptions.md) owns Artefact, ArtefactReference, Perception, and source selectors. A proposal refers to those objects. It does not redefine them or mint substitute source objects.

The transaction records nondeterministic work before it uses the result. Replay consumes the recorded Activity result and does not call a model. This follows the current record-at-call-time contract in [`docs/events-and-storage.md`](../docs/events-and-storage.md#event-sourcing). The multi-block review and atomic publication protocol is a permanence-driven design decision rather than an established result.

## Proposal state machine

A proposal transaction has a stable proposal ID and one source Occasion or source Activity. Each transition appends a record. No transition edits or deletes an earlier record.

| State | Appended record | Visible state | Permitted successor |
|---|---|---|---|
| `source_buffered` | the source Occasion or Activity and its source references | the source is available through source recall; no proposed structure is searchable as accepted memory | `extracting`, `source_only`, `aborted` |
| `extracting` | an extraction Activity with its implementation, model, prompt, schema, ontology, and input versions | the Activity is visible to operators; its output is not on conversational reads | `critic_review`, `extraction_retry_wait`, `source_only`, `aborted` |
| `critic_review` | typed Proposition, Assertion, Attestation, Event-role, Perception, or Derivation proposals and critic results | proposals and diagnostics are visible to the reviewing agent and authorised operators only | `awaiting_review`, `source_only`, `aborted` |
| `awaiting_review` | the complete critic-approved proposal set | the reviewer assigns per-item dispositions; accepted reads still return only the source | `reviewing`, `superseded`, `source_only`, `aborted` |
| `reviewing` | append-only per-item `proposed`, `amended`, `dropped`, or `accepted` dispositions; amendments retain lineage | all versions and dispositions remain audit-only | `critic_review` for amended items, `accepted`, `superseded`, `source_only`, `aborted` |
| `accepted` | the reviewer's final complete publish set and every non-published item's terminal disposition | the accepted set is not visible until publication commits | `publishing`, `superseded`, `aborted` |
| `publishing` | compare-at-commit heads, stable ID map, and atomic publication attempt | no accepted structure is visible without the publication marker | `published`, `publication_retry_wait`, `superseded`, `aborted` |
| `published` | one atomic publication record naming all accepted object and transition IDs | accepted Assertions become available through audience-resolved reads; their source and lineage remain reachable | terminal |
| `extraction_retry_wait` | the pre-acceptance extraction failure class, attempt count, retry condition, and next eligible time | the source and failure are visible; partial structure is not published | `extracting`, `source_only`, `aborted` |
| `publication_retry_wait` | the post-acceptance publication failure class, attempt count, retry condition, and next eligible time | the source and accepted publish set remain visible only through audit; no partial structure is published | `publishing`, `superseded`, `aborted` |
| `source_only` | the terminal pre-acceptance reason that structured acceptance did not complete | the source remains durable and searchable on the source lane; no rejected or partial proposal is presented as accepted structure | terminal |
| `superseded` | the replacement proposal ID and reason | the old transaction remains auditable and cannot publish | terminal |
| `aborted` | the actor, reason, and last complete state | durable source records remain available; uncommitted structure remains audit-only | terminal |

`proposed`, `amended`, `dropped`, and item-level `accepted` are dispositions, not transaction states. A dropped item cannot later become accepted; an amendment creates a new item version and returns the affected items to `critic_review`. `published`, `source_only`, `superseded`, and `aborted` are terminal transaction states. `source_only` is available only before final acceptance. A correction after publication uses the append-only Assertion and Attestation transitions defined in [the assertion model](statements.md). It does not reopen the proposal transaction.

The proposal's `source_head` is a replay watermark, not a compare against every later log append. Publication records a dependency compare vector containing the consumed source and target transition-chain heads, governing policy and schema versions, ResolutionEnvironment dependencies, current audience-authorisation dependencies, and expected proposal state. Validation and publication occur under the same writer exclusion. The publication attempt checks its expected predecessor state; its own audit records do not invalidate that comparison. Unrelated appends do not invalidate the proposal. A scoped negative query or aggregate also records the query scope and projection version, and publication validates changes that can affect that scope. If the implementation cannot establish that a change is irrelevant, publication fails closed.

A changed dependency prevents publication of the accepted set. The transaction becomes `superseded` when a replacement proposal is recorded, or `aborted` when no replacement is created. A replacement may reuse retained extraction output, but it must rerun the affected checks and obtain a new acceptance decision. It cannot alter the old accepted set. A transient publication failure enters `publication_retry_wait` and retries the same accepted set against the same dependency vector. Publication appends all accepted records and the publication marker atomically. A crash before that commit exposes none of the accepted structure. A crash after that commit exposes all of it.

Recovery folds the proposal log by proposal ID. Before final acceptance, it consumes any durable extraction Activity result without rerunning that model work. It resumes `extracting` and `extraction_retry_wait` only when their recorded lease has expired, no durable extraction result is available, and the retry bound permits another extraction attempt. It returns `critic_review`, `awaiting_review`, and `reviewing` to the reviewing surface. It preserves an `amended` item disposition without rerunning a recorded model result. After final acceptance, it verifies atomic publication for `accepted`, `publishing`, and `publication_retry_wait`; an incomplete commit exposes no objects and retries publication without rerunning extraction, reminting IDs, or changing the accepted publish set. A post-acceptance failure cannot enter `source_only`. Recovery terminates the transaction as `aborted` or `superseded` only when the recorded policy or actor disposition requires it. `source_only` is permitted only before final acceptance and cannot follow an accepted publish set. The configured attempt bound is part of the policy version. A critic-stage infrastructure retry remains in `critic_review`, reuses the durable extraction result, and records a bounded attempt without starting extraction again. Exhaustion before acceptance appends `source_only` rather than retrying indefinitely. Publication retry exhaustion first checks whether the atomic publication marker already exists. A committed marker resolves to `published`; a confirmed uncommitted attempt appends `aborted` under the retry-policy authority with reason `publication_attempts_exhausted`. An unavailable store leaves recovery pending until commit status can be checked; it never authorises another model call or exposes partial structure.

The `stage:7` fault-injection fixtures include critic retry with no repeated extraction, publication exhaustion before and after a committed marker, proposal audit records and unrelated appends that do not stale publication, and changed source, audience, policy, resolution, or scoped-query dependencies that prevent publication.

## Typed proposals

An extraction Activity can propose the following records:

- a Proposition and an Assertion over it;
- an Attestation grounded in an Occasion;
- an Event and independently addressable role or attribute Assertions;
- a Perception grounded in an ArtefactReference or typed selector;
- a Derivation grounded in typed inputs;
- an explicit `nothing_to_record` result.

The proposal carries stable temporary IDs. Amendments and critic results refer to those IDs. Publication maps each surviving temporary ID to one permanent ID exactly once. Retry and crash recovery reuse the mapping. This prevents duplicate identities after an uncertain commit.

A direct write uses an Activity as its source. Its source kind is one of `agent_observation`, `operator_assertion`, `tool_observation`, or `derivation`. It does not fabricate an Occasion, utterance, teller, or text span. A conversational extraction uses the real Occasion and creates an Attestation only when the Occasion supports one.

## Hard critics

Hard critics are deterministic, versioned, and gating. They report a machine-readable failure and a correction hint. The critic bank evaluates separate concerns so that one valid property cannot conceal another invalid property.

The proposition critic checks registered subjects, relations, typed object variants, referential frames, polarity, modality, domain, range, mutually exclusive kinds, typed quantities, and the prohibition on frame/modality and lexical-negation duplication. The assertion critic checks validity shape, temporal precision, immutable assertion mode, lifecycle-transition authority, and that a published candidate is not silently settled. The attestation critic checks teller authority, expression strength, transmission principle, scoped witness evidence, and the source locator against the Occasion. The grounding critic checks text-part spans, whole artefacts, selectors bound to the cited ArtefactReference, tool observations, direct observations, operator assertions, and Derivation inputs according to their source type. The Event critic checks registered roles and event-specific projection rules. The Task/Trigger critic ensures descriptive occurrence cannot arm scheduling. The Derivation critic checks typed inputs, exactly one output, the direct-observation exception, unified ResolutionEnvironment, and InfluenceEnvelope.

Text grounding verifies that a proposed half-open span lies within the cited stable text part under its registered scalar or tokenizer basis. Multimodal grounding verifies that the selector targets the Artefact resolved through the cited ArtefactReference and that the producing Activity's ordered consumed-reference edge names the selector/version, pipeline/version, and access decision. These checks establish source attachment. They do not establish that the extraction interpreted the source correctly or that the source is true. The current span check reduced unsupported dated occurrences in one local measurement, but the broader critic protocol remains a design synthesis ([`research/2026-08-06/current-system-fixes.md`](research/2026-08-06/current-system-fixes.md)).

Audience critics run over every proposal, critic diagnostic, model prompt, and model response. Hidden inputs cannot produce a wider output. [Privacy and provenance](privacy-and-provenance.md#influence-envelopes) owns the transmission and influence rules. The episodic wall critic also checks the recorded context influence, including notes, retries, and intermediates. It rejects semantic publication when `InfluenceEnvelope.non_evidentiary_marks` contains `episodic_reconstruction/v1`, even when the proposal omits the episode from its declared premises. A fresh source-only Activity must pass the same critics before its output can publish.

The episodic wall remains a hard critic. A generated episode cannot be a premise, gain an Attestation, or become an Assertion through proposal review. [The two traces](two-traces.md) owns the distinction between source evidence and generated narrative.

## Soft critics

Soft critics evaluate linguistic quality and semantic plausibility. They can rank proposals, request review, or attach diagnostics. They cannot publish, reject a structurally valid proposal, settle an Assertion, widen an audience, or override a hard critic. Their prompts, responses, sampled votes, and implementation versions are recorded Activities.

## Derivation records

A derived proposal records typed positive Assertion inputs, negative query results, aggregates, tool observations, Perceptions, and any other admitted input. It also records the implementation or criterion version, ontology definition versions, policy versions, a unified ResolutionEnvironment with explicit identity and Event components, assumptions, and source head sequence. Negative results name the query, audience-resolution context, covered head sequence, and closed-world scope. An absence outside that declared scope is not a premise.

A lineage response can be complete only when every influence is represented by these inputs. Otherwise the system returns an audit trace and labels the unrecorded boundary. It does not call every explanation a proof.

## Candidate and publication status

Proposal acceptance and Assertion settlement are different decisions. Publication admits a well-formed, grounded candidate to the store. The candidate-to-settled transition follows the versioned promotion policy owned by [belief](belief.md) and the append-only transition model owned by [the assertion model](statements.md). A critic cannot settle an Assertion merely because its structure passed.

Independent agreement can contribute to promotion, but no fixed two-signal rule is part of the permanent substrate. Dependence, expression strength, visible support, and policy version affect the projection. `capability:support-fusion` remains disabled until its independent activation gate passes; the `stage:12` initial policy remains conservative and does not treat autonomous promotion as settled truth.

## Drift controls

The ontology and critic versions prevent known malformed writes. Canaries, replay audits, longitudinal rate checks, and re-derivation checks detect regressions. They do not establish correctness. Similarity thresholds are calibrated to their embedding version and cannot silently retain their old meaning after an embedding change.

Persistent critic rejection, an identity-boundary disclosure, an unclear erasure authority, and a drift alarm enter the operator exception queue. The queue is an explicit operational cost. A mechanism whose review cost remains constant per stored fact does not scale.
