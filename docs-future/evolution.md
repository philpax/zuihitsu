# Evolution

The current instance remains on the current model. It is not migrated or dual-read. The successor starts at genesis. This boundary does not permit incompatible change after successor genesis. Every later stage uses additive records, registered definition versions, new projections, or explicit transitions. No stage resets an existing successor instance or invents missing history.

Every stage records its implementation plan when work starts. The plan names concrete modules, event variants, wire formats, migrations, automated tests, and scenario fixtures. This roadmap defines semantic dependencies and evidence gates without freezing those implementation details early.

The mechanical stage audit uses the following exact heading aliases. `Prerequisites`, `Compatibility`, and `Deferred` match literally. `Semantic contracts` is the stage's `Implements` section. `Evidence gates` is its `Gates` section. `Rollback/disable` is its `Rollback` section. `Owning chapters`, `Required decisions`, and `Stop conditions` are additional mandatory sections. An extractor must normalise those three aliases before checking every stage and must require all nine sections. This mapping resolves the two heading vocabularies without duplicating content or weakening either contract.

## Stage -1: permanence review

### Prerequisites

The proposed successor objects, source kinds, and privacy boundaries are available for review.

### Semantic contracts

Enumerate stable IDs, event envelopes and payload versions, source kinds, registered ontology and policy definitions, replay and upcast rules, transition folds, erasure boundaries, and compatibility rules. Run counterfactuals for negation, modality, temporal correction, per-teller retraction, hidden support, Event split, identity severance, schema change, witness-policy change, support-policy change, image reinspection, shared-blob erasure, and Derivation invalidation. A design fails if replay must invent a value or reinterpret a persisted field.

### Owning chapters

[Overview](overview.md#permanence-contract), [assertions](statements.md), [artefacts](artefacts-and-perceptions.md), [privacy](privacy-and-provenance.md), and [relations](relations.md).

### Evidence gates

All counterfactuals preserve source records and stable identities. The evidence is append-only replay behavior in the current system and migration cost observed in surveyed systems. The exact permanence contract remains a design decision ([storage](../docs/events-and-storage.md), [verification](research/2026-07-24/verification/part-b.md), [survey](research/2026-07-24/lanes/survey-giants.md)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | None. This review freezes versioning rules, not implementation names. |
| Pre-stage log readability | Not applicable to successor logs because genesis has not occurred. Current-instance logs remain on the current model and are not migrated. |
| Projection and snapshot rebuild | Every proposed fold must rebuild deterministically from the fixture log; no successor snapshot exists yet. |
| Required raw inputs | The review enumerates every identity, source, witness, influence, selector, time, policy, authorization, and erasure input required at genesis. |
| Disablement and persisted meaning | No runtime policy exists. Rejection revises the design before genesis. |

### Required decisions

The architecture owner must approve the stable-ID inventory, version/upcast contract, selector encoding, and transition folds before Stage -1 closes. The architecture, security, storage, and operator owners must freeze the [erasure storage baseline](privacy-and-provenance.md#storage-baseline): the envelope and payload split, reference-set retention of shared bytes, the tombstone ledger and ledger-first restore ordering, and the recording of exports as outside the erasure boundary. Each decision is an entry blocker, is recorded in the Stage -1 permanence decision record before schema freeze, and maps to the genesis-blocking permanence, erasure, selector, and identity rows in [the unresolved register](confidence.md#unresolved-item-register). No semantic or irreversible-storage decision may be deferred into Stage 1 or Stage 2.

### Rollback/disable

No runtime capability exists. A failed review revises the design before genesis.

### Stop conditions

Stop if any fixture requires destructive merge, stable-ID reuse, old-data invention, hidden-input leakage, or an unbounded erasure promise.

### Deferred

All runtime policy and implementation.

## Stage 0: measurements before commitments

### Prerequisites

Access to current recorded turns, connector metadata, attachment fixtures, and a synthetic-log harness.

### Semantic contracts

Measure: `0a` raw Occasion-window retrieval against generated narrative encoding; `0b` connector witness evidence and disclosure/exposure assurance; `0c` extraction yield, convergence, latency, and economics against canonical Proposition/Assertion/Attestation fixtures; `0d` browser/log/projection cost including ArtefactReference, Perception, Derivation, blob, and eval-package volume; `0e` query-surface classification on real turns; and `0f` multimodal recall using prior Perception, explicit reinspection, and textual memory.

### Owning chapters

[Two traces](two-traces.md), [privacy](privacy-and-provenance.md), [verified write](verified-write.md), [query surface](query-surface.md), [artefacts](artefacts-and-perceptions.md), and [coverage](coverage.md).

### Evidence gates

Measurements publish fixtures, prompts, models, costs, failures, uncertainty, and baselines. Before data collection, the architecture owner and operator sign a `stage-0-measurement-contract` that fixes each metric, minimum sample, confidence treatment, cost and latency budget, privacy failure tolerance, and pass/fail rule. The current implementation baselines each metric. No dependent implementation or activation begins while its threshold is absent or changed after results are visible. Stage 0a preserves the one-study and missing-ablation caveats. Stage 0c defines canonical encoding before convergence is measured ([dual-trace study](research/2026-08-03/dual-trace.md), [log measurements](research/2026-08-06/log-measurements.md), [confidence](confidence.md#evidence-map)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | None; measurement packages are evidence artefacts, not successor semantic records. |
| Pre-stage log readability | Current logs are read-only inputs. No migration or mutation occurs. |
| Projection and snapshot rebuild | None for successor state. Measurement packages name the code, fixture, and source-log sequence used. |
| Required raw inputs | Current turns, connector metadata, attachments, costs, and synthetic fixtures listed in the prerequisites. Missing input blocks the affected measurement. |
| Disablement and persisted meaning | Discarding a flawed measurement changes no persisted successor meaning. |

### Required decisions

The architecture owner and operator own the `stage-0-measurement-contract`; security owns privacy tolerances; the model/eval owner owns sampling and uncertainty. Alternatives, baselines, and thresholds are frozen before each measurement. Missing decisions block only the dependent measurement and stage, not unrelated measurements. These map to the extraction convergence, witness evidence, generated episodes, multimodal retrieval, query classification, and storage-budget rows in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Discard a flawed measurement and rerun it with the flaw recorded.

### Stop conditions

Stop a dependent stage when results do not distinguish the proposed treatment, costs exceed declared budgets, or witness metadata cannot support fail-closed resolution.

### Deferred

Generated episodes, automatic multimodal work, and authoritative extraction.

## Stage 1: modelling and replay spike

### Prerequisites

Stage -1 passes. Stage 0c defines canonical encodings.

### Semantic contracts

Model Occasion, Activity, ArtefactReference, Proposition, Assertion, Attestation, Perception, Event, polarity, modality, source locators, transitions, and first hard critics over recorded logs and synthetic multimodal fixtures.

### Owning chapters

[Assertions](statements.md), [artefacts](artefacts-and-perceptions.md), [events](events-and-roles.md), and [verified write](verified-write.md).

### Evidence gates

Byte-stable canonicalization, deterministic replay, compound and artefact-only Occasions, direct Activity sources, quotation, correction, per-teller audiences, and source-only fallback pass. The corpus study establishes expressiveness problems but not extraction convergence ([modelling study](research/2026-08-03/modelling-study.md)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Disposable candidate versions only; none can become a successor genesis contract without Stage -1 approval. |
| Pre-stage log readability | The spike reads current fixtures through adapters and never changes current log meaning. |
| Projection and snapshot rebuild | Every spike log rebuilds byte-stably without a snapshot; snapshots are disposable test artefacts. |
| Required raw inputs | Canonical encodings, source fixtures, selector definitions, witness scopes, and erasure tombstones from Stages -1 and 0. |
| Disablement and persisted meaning | The spike can be deleted in full; no authoritative persisted meaning depends on it. |

### Required decisions

The architecture owner chooses among candidate canonical encodings and critic boundaries using the Stage 0c baseline. The decision is recorded in `stage-1-model-selection` before the spike may pass and blocks Stage 2 entry. The selected model must close every genesis-blocking row in [the unresolved register](confidence.md#unresolved-item-register); a result that merely chooses the least-bad incomplete model is a stop condition.

### Rollback/disable

Discard the spike implementation. Preserve measurements and fixture results.

### Stop conditions

Stop if canonicalization does not converge sufficiently, critics require unsupported fields, or object boundaries cannot express the fixtures.

### Deferred

Authoritative structured writes and policy automation.

## Stage 2: additive genesis substrate and audience resolution

### Prerequisites

Stages -1 and 1 pass. Stage 0b supplies an assurance model or establishes teller-only fallback.

### Semantic contracts

Implement every permanent identity and transition shape before structured data becomes authoritative: Occasion, Activity, Artefact and ArtefactReference, Perception, Event and resolution hypothesis, Proposition, Assertion, Attestation, Derivation, typed locators, polarity and modality, typed validity with precision and timezone, correction and supersession, candidate and promotion, ontology and policy definition IDs, proposal states, influence envelopes, and audience-resolved views. Occurrence, agent-authored Task, and Trigger are separate. Source-only fallback remains valid.

Stage 2 also implements executable baseline forgetting before any restricted source or Artefact is accepted: the envelope and payload split, audience-checked blob retrieval, the operator authorization record, single-pass closure under the writer lock over the six surfaces, reference-specific withdrawal, dependent invalidation, index and snapshot rebuild, and the tombstone ledger applied on restore. Stage 8 may improve operations or activate richer transmission policy, but it cannot add basic erasure semantics.

### Owning chapters

All normative chapters. [Assertions](statements.md), [privacy](privacy-and-provenance.md), and [time](time.md) own the main contracts.

### Evidence gates

Replay determinism, hidden-input non-interference, subject-guard compilation, crash recovery, erasure inventory, dated-description-never-fires, and shared-blob authorization pass. Durable activities and non-interference have supporting research. Complete influence and erasure closure remain genesis-blocking design obligations ([verification](research/2026-07-24/verification/part-b.md), [privacy research](research/2026-07-24/lanes/provenance-privacy.md), [confidence](confidence.md#adversarial-obligation-register)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Version 1 for every identity, transition, selector, witness, access, influence, erasure, Task, Trigger, ontology, and policy record in the Stage 2 contract. |
| Pre-stage log readability | There are no earlier successor production logs. Current-instance logs remain unmigrated; Stage 1 fixtures remain readable only as test inputs. |
| Projection and snapshot rebuild | All folds rebuild from versioned events. Snapshots carry schema versions and a tombstone-ledger position and are rebuilt when the ledger has advanced past them. |
| Required raw inputs | Every later policy input named by Stage -1 is stored now, including reference-authorized model consumption, typed time, resolution environments, witness scope, and erasure authority. |
| Disablement and persisted meaning | Disabling structured policy returns source-only reads. Baseline access control and erasure remain active; persisted source and typed meanings do not change. |

### Required decisions

The architecture owner, security owner, storage owner, and operator must approve `stage-2-genesis-contract`: event and definition versions, conformance to the Stage -1 storage baseline, audience-checked blob access, the exact audience resolver and subject-guard predicate, authority lattice, source-only behaviour, and operational limits. Stage 2 may choose implementation modules and cryptographic primitives that satisfy the frozen contract; it may not choose a different payload boundary, restore ordering, or erasure semantics. Alternatives and rationale are recorded before the first successor genesis. Any unresolved genesis-blocking row in [the unresolved register](confidence.md#unresolved-item-register) blocks stage entry, not merely activation.

### Rollback/disable

Disable structured projection and use source-only reads. Persisted source and typed records remain valid.

### Stop conditions

Stop before genesis if an identity-bearing field is still expected later, audience resolution is distributed among callers, or erasure cannot identify all influence surfaces.

### Deferred

Rich recurrence, contradiction, support arithmetic, generated narrative, autonomous resolution, and multimodal automation.

## Stage 3: shadow verified writes and structural projections

### Prerequisites

Stage 2 is complete. Stage 0c provides economic baselines.

### Semantic contracts

Run extraction, grounding, and critics in shadow mode through the append-only proposal lifecycle. Publish atomically only after review. Initial authoritative policy is limited to conservative actual-positive Propositions and minimal Event handling. Stage 3 cannot add an identity-bearing field.

### Owning chapters

[Verified write](verified-write.md), [write surface](write-surface.md), [query surface](query-surface.md), and [assertions](statements.md).

### Evidence gates

Fidelity, convergence, completion rate, blocks per write, retries, latency, constraint tax, temporal safety, source-only fallback, and non-interference meet the thresholds in `stage-3-authority-gate`. The eval owner drafts that artefact from Stage 0c baselines; the architecture and operator owners approve and freeze it before shadow data is inspected for activation. Any privacy or source-preservation failure has zero tolerance. Critics establish shape and limited grounding, not truth ([verification research](research/2026-07-24/verification/part-b.md), [welding](research/2026-07-24/lanes/welding.md)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Stage 2 proposal, review, transition, critic, and Activity versions; a new version is registered only if Stage 2 left an additive extension seam. |
| Pre-stage log readability | Stage 2 source-only logs remain readable and require no invented structured output. |
| Projection and snapshot rebuild | Rebuild can include or ignore shadow outputs by recorded authority state. Snapshots record that state and invalidate on policy change. |
| Required raw inputs | Stage 2 source locators, authority, witness, influence, proposal, and erasure fields are sufficient. No new identity-bearing input is permitted. |
| Disablement and persisted meaning | Returning to shadow or source-only changes selection only. Accepted, rejected, amended, and dropped history retains its recorded meaning. |

### Required decisions

The architecture owner and operator approve `stage-3-authority-gate`, the initial authoritative Proposition subset, review authority, retry bounds, and source-only degradation rule before activation. Entry into shadow mode requires no unresolved semantic decisions; activation is blocked until the quantitative gate is frozen and passes. The decisions map to extraction convergence, critic faithfulness, verified-write lifecycle, and source-only fallback in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Return to shadow or source-only mode. Do not delete accepted or rejected proposal history.

### Stop conditions

Stop on junk fill, unstable canonicalization, unacceptable latency or log growth, critic bypass, or source-only failure.

### Deferred

Non-actual inference, autonomous acceptance, broad contradiction, and generated episodes.

## Stage 4: typed time policy

### Prerequisites

Stage 3 passes for relevant Assertion shapes.

### Semantic contracts

Activate correction and refinement, uncertainty and timezone rendering, planned/actual/cancelled semantics, and explicit recurrence policies including last-day, skip, clamp, and business-calendar adjustment.

### Owning chapters

[Time](time.md) and [assertions](statements.md#assertion).

### Evidence gates

Dated-description-never-fires, temporal-correction-preserves-source, vague-time, timezone, cancellation, and month-end fixtures pass. Occurrence/Task/Trigger separation is locally supported; uncertainty ownership and recurrence policy remain design work ([time research](research/2026-07-24/lanes/time-memory.md), [confidence](confidence.md#evidence-map)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Additive temporal-policy, timezone-database, calendar, recurrence, and interpretation versions; no new temporal identity field. |
| Pre-stage log readability | Stage 2 and 3 logs remain readable. Unknown or absent optional interpretations stay unknown rather than being invented. |
| Projection and snapshot rebuild | Rebuild uses each value's recorded policy context. Temporal snapshots invalidate when an activated projection version changes. |
| Required raw inputs | Precision, uncertainty, timezone ownership, validity, occurrence Assertions, modality, Task, and Trigger are already present. |
| Disablement and persisted meaning | Disabling temporal automation suppresses derived interpretations and firing only; source and typed values retain meaning. |

### Required decisions

The architecture owner and operator choose the initial uncertainty rendering, timezone ownership fallback, cancellation representation, and recurrence subset in `stage-4-temporal-policy` before activation. Alternatives include disabling each interpretation. The time/eval owner freezes month-end, vague-time, and timezone oracle expectations before testing. Missing choices block activation, not implementation of the generic versioned evaluator. They map to typed-time and temporal-subalgebra rows in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Disable temporal automation and retain source and typed values.

### Stop conditions

Stop if ordinary intent requires guessing, corrections mutate source, or descriptions can create Triggers.

### Deferred

Unvalidated recurrence policies and broad temporal inference.

## Stage 5: Event, role, and relation policy

### Prerequisites

Stages 3 and 4 pass for Event role Assertions.

### Semantic contracts

Activate typed Event subroles, disclosure-safe projections, conservative co-reference, relation evolution, historical-definition replay, and governed schema activation.

### Owning chapters

[Events and roles](events-and-roles.md), [relations](relations.md), and [privacy](privacy-and-provenance.md).

### Evidence gates

Repeated same-participant Events remain distinct. Explicit re-mention can merge. Merge then sever restores source views and invalidates stamped Derivations. Partial projection never manufactures a stronger Event. Alias-cycle and historical-definition fixtures pass. Event/role structure is supported, but stable Event identity and projection safety are design obligations ([modelling study](research/2026-08-03/modelling-study.md), [fact-shape report](research/2026-07-24/report.md), [confidence](confidence.md#evidence-map)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Additive Event-type, role/subrole, relation, projection-policy, schema-governance, and Event-resolution transition versions. |
| Pre-stage log readability | Earlier Events and Assertions rebuild under their recorded definitions; absent subroles or hypotheses are not inferred. |
| Projection and snapshot rebuild | Rebuild validates historical records in their original context and recomputes current aliases, safe shells, and live composites. Snapshots invalidate on registry or resolution changes. |
| Required raw inputs | Stable Events, attribute/role Assertions, source locators, audience data, and resolution environments are present from Stage 2. |
| Disablement and persisted meaning | Disabling typed subroles or co-reference returns parent-role and source Event views without changing records. |

### Required decisions

The architecture owner and schema reviewer approve `stage-5-event-schema`: the initial Event types, universal role parents, subrole activation governance, safe-shell policies, and Event-resolution authority. Each proposed type or subrole may be omitted. Missing governance blocks stage entry; an unvalidated type blocks only its activation. These choices map to role governance, Event co-reference, schema replay, and partial disclosure in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Disable typed subrole inference and co-reference acceptance. Preserve hypotheses and stable Events.

### Stop conditions

Stop on role-tail inconsistency, unsafe partial visibility, schema activation through hidden conversational syntax, or destructive merge.

### Deferred

Autonomous Event merging and broad role vocabularies.

## Stage 6: identity hypotheses

### Prerequisites

Central audience resolution and resolution-stamped Derivations pass.

### Semantic contracts

Start with operator-confirmed disjoint operational composites. Platform stubs remain permanent. Merge hypotheses record evidence, scope, recall and disclosure clearance, authority, status, and lifecycle. Overlapping candidates do not become one composite.

### Owning chapters

[Identity](identity.md), [privacy](privacy-and-provenance.md), and [query surface](query-surface.md).

### Evidence gates

Overlap conflicts, response-affecting disclosure, one-handle behavior, merge/sever replay equivalence, and sibling-history fixtures pass. Attribute overlap is not identity evidence by itself ([identity research](research/2026-07-24/lanes/identity-belief.md), [report](research/2026-07-24/report.md), [confidence](confidence.md#evidence-map)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Additive identity-hypothesis, acceptance, severance, composite, clearance, and resolution-environment versions. |
| Pre-stage log readability | Permanent platform stubs and pre-stage Assertions remain readable as separate identities; no merge is inferred. |
| Projection and snapshot rebuild | Rebuild folds accepted disjoint composites and invalidates environments after severance. Identity-dependent snapshots invalidate on any resolution transition. |
| Required raw inputs | Stable stubs, identity evidence, witness data, authority, and Derivation resolution dependencies are already present. |
| Disablement and persisted meaning | Disabling resolution exposes source handles to operators and fails closed conversationally; it does not rewrite stubs or Assertions. |

### Required decisions

The operator and security owner approve `stage-6-identity-policy`: confirmation authority, disjointness conflict handling, recall and disclosure clearance, one-handle rendering, and severance response. Operator confirmation is the only initial acceptance alternative; autonomous scoring remains disabled. Missing choices block stage entry and map to identity overlap, response-affecting recall, one-handle behavior, and severance rows in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Withdraw accepted hypotheses and invalidate dependent projections.

### Stop conditions

Stop if composites overlap, tentative recall affects a response below disclosure clearance, or severance loses history.

### Deferred

Autonomous scoring and cross-instance identity.

## Stage 7: support, dependence, and mechanical contradiction

### Prerequisites

Attestations, witness evidence, audience resolution, and transition folds are authoritative.

### Semantic contracts

Activate audience-safe support projections, dependence detection, reliability observations, expression strength, explicit polarity, and the registered mechanical contradiction subset. Support remains an ordinal corroboration projection rather than a truth probability.

### Owning chapters

[Belief](belief.md), [assertions](statements.md#mechanical-contradiction), and [privacy](privacy-and-provenance.md).

### Evidence gates

Shared-room dependence, agent restatement, relay and return, reliability change, hidden support, last independent support withdrawal, opposite polarity, quantities, and contest-versus-contradiction fixtures pass. Provenance and discounting have support; arithmetic and truth-directed interpretation remain unsettled ([identity and belief research](research/2026-07-24/lanes/identity-belief.md), [confidence](confidence.md#evidence-map)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Additive support, dependence, reliability, polarity-normalisation, contradiction-rule, and promotion-policy versions. |
| Pre-stage log readability | Existing Attestations and Propositions retain source meaning. Missing evidence yields unranked or contested state. |
| Projection and snapshot rebuild | Support and contradiction projections rebuild per audience and policy version; snapshots invalidate on Attestation, audience, dependence, or rule changes. |
| Required raw inputs | Teller-specific Attestations, expression strength, dependence lineage, polarity/modality, validity, witness, and influence are present. |
| Disablement and persisted meaning | Disabling policy returns unranked visible Attestations and operator-reviewed conflicts without changing Assertions. |

### Required decisions

The architecture, security, and eval owners approve `stage-7-support-policy`: ordinal vocabulary, dependence rules, reliability use, promotion threshold, and the exact mechanical contradiction registry. Alternatives include no rank and no automatic promotion. Privacy non-interference thresholds are zero tolerance; other activation thresholds are frozen against Stage 0 baselines before testing. Missing policy blocks activation, not projection implementation. These map to support fusion, dependence, reliability, polarity, and contradiction rows in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Return to unranked visible Attestations and operator-reviewed conflicts.

### Stop conditions

Stop if hidden support changes visible output, dependent evidence increases support, or mechanical rules claim linguistic contradiction.

### Deferred

Fusion operators, probabilities, and autonomous contradiction arbitration.

## Stage 8: richer privacy policy and erasure operations

### Prerequisites

Stage 2 baseline subject guards, influence envelopes, audience-checked blob access, witness assurance, executable erasure closure, and ledger-filtered restore pass.

### Semantic contracts

Stage 2 already enforces baseline erasure, audience-checked access, conservative subject guards, and static fail-closed transmission. This stage activates only richer decidable transmission principles and operational erasure improvements with passing connector, influence, package, backup, and non-interference tests. Candidate principles are public, attributed, demonstrated-witness-only/in-confidence, explicit include/exclude, and scoped consent with expiry or revocation. An implementation may activate only the subset whose evidence passes.

### Owning chapters

[Privacy and provenance](privacy-and-provenance.md) and [artefacts](artefacts-and-perceptions.md).

### Evidence gates

Silent-member, multi-subject, hidden aggregate, rejected-proposal taint, shared-blob, derived-artefact, embedding, and snapshot scenarios pass. Non-interference and retract-versus-forget are supported concepts. Exact closure is design work ([privacy research](research/2026-07-24/lanes/provenance-privacy.md), [current leak observation](research/2026-08-06/current-system-fixes.md), [storage](../docs/events-and-storage.md)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Additive transmission-principle evaluators, consent, remote-status, and erasure-operation versions. Baseline Stage 2 tombstones and authorization decisions remain canonical. |
| Pre-stage log readability | Older records retain their transmission expression and assurance evidence. Unknown principles fail closed; no witness scope is invented. |
| Projection and snapshot rebuild | Audience and influence projections rebuild under named evaluator versions. The tombstone ledger continues to govern snapshot rebuild. |
| Required raw inputs | Stage 2 witness scopes, subject closures, access records, influence edges, envelope/payload separation, and the six erasure surfaces are already present. |
| Disablement and persisted meaning | Disabling a richer evaluator falls back to the safest Stage 2 static audience. Completed erasure remains irreversible and meaningful. |

### Required decisions

The security owner, operator, and connector owner approve one `stage-8-policy-activation` record per principle or operational improvement. It names the semantics, evidence source, failure mode, rollback, and zero-residue oracle before activation. No all-or-nothing bundle exists. Missing decisions block only that capability. Reciprocity and purpose limitation remain unresolved gated rows in [the unresolved register](confidence.md#unresolved-item-register) and cannot be activated here.

### Rollback/disable

Disable a transmission evaluator and restrict affected content to its safest static audience. Erasure execution itself is not reversible.

### Stop conditions

Stop if a hash acts as a bearer credential, closure omits a durable surface, or policy depends on unrecorded purpose or presence.

### Deferred

Reciprocal and purpose-based policies until execution-purpose and obligation semantics exist.

## Stage 9: memory kinds and bulk ingestion

### Prerequisites

Artefact, Activity, bounded jobs, per-document audience, and source-only fallback pass.

### Semantic contracts

Activate procedural and working-memory lifecycles and source-first long-document or media ingestion. Ordinary conversational Perception remains separate from bulk jobs and generated episodes.

### Owning chapters

[Memory typology](memory-typology.md), [artefacts](artefacts-and-perceptions.md), and [off-turn work](off-turn.md).

### Evidence gates

Ingestion precision, cost, source-only fallback, per-document audience, influence propagation, cancellation, and bounded-work fixtures pass thresholds in `stage-9-ingestion-gate`. The ingestion owner drafts the metrics and alternatives from Stage 0 baselines; the operator and security owner approve and freeze them before implementation begins. Any audience-lineage or source-loss failure has zero tolerance. Four memory lifecycles are motivated, but storage and ingest economics remain stage-gated ([time and memory research](research/2026-07-24/lanes/time-memory.md), [coverage](coverage.md), [confidence](confidence.md#evidence-map)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Additive memory-kind, ingestion-plan, document-selection, extraction, and bounded-job versions. |
| Pre-stage log readability | Earlier conversational Occasions, Artefacts, Activities, and Perceptions retain their existing kinds; bulk ingestion is never inferred retroactively. |
| Projection and snapshot rebuild | Ingestion projections and job folds rebuild from plans and source heads; snapshots invalidate on source, audience, or pipeline changes. |
| Required raw inputs | Source Artefacts, references, selectors, access decisions, typed Activities, influence, and erasure transitions already exist. |
| Disablement and persisted meaning | Disabling ingestion retains source Artefacts and source-only reads; generated structure becomes inactive without changing source meaning. |

### Required decisions

The ingestion owner, operator, security owner, and storage owner approve `stage-9-ingestion-gate`: enabled memory kinds, document selection, chunk/source locator policy, per-document audience, maximum work and storage, cancellation, and economics. Alternatives include explicit-only ingestion and source-only retention. Missing decisions block stage entry. They map to memory typology, working storage, document selection, and ingest economics in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Disable automatic ingestion and retain source Artefacts for explicit governed use.

### Stop conditions

Stop on unbounded work, missing audience lineage, junk extraction, or inability to retain source-only content.

### Deferred

Automatic broad media ingest and scene graphs.

## Stage 10: optional episodic and multimodal extensions

### Prerequisites

Stage 0a or 0f passes the specific capability's evidence gate. The episodic non-evidence wall and multimodal influence controls pass.

### Semantic contracts

Generated episodes, historical reinspection, OCR, generated captions, region grounding, and visual search are separate capabilities over genesis records. Each records its Activity, Perception or Derivation, source, version, audience, and access.

### Owning chapters

[Two traces](two-traces.md), [artefacts](artefacts-and-perceptions.md), [query surface](query-surface.md), and [privacy](privacy-and-provenance.md).

### Evidence gates

Each capability has an independent `stage-10-<capability>-gate` fixing retrieval-value, cost, latency, grounding, correction, erasure, and privacy thresholds before its experiment runs. The capability owner drafts it from Stage 0a or 0f baselines; the operator and security owner approve it. Privacy, source laundering, and erasure failures have zero tolerance. A pass for one capability supplies no evidence for another. Generated narrative retains the small-sample, automated-judge, missing-ablation, geometry-variance, invention, and non-transferable cost caveats ([dual trace](research/2026-08-03/dual-trace.md), [research index](research/README.md), [confidence](confidence.md#evidence-map)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | One additive pipeline, criterion, projection, and capability-policy version per independently activated extension. |
| Pre-stage log readability | Pre-stage logs remain readable. They acquire no generated episode, OCR, caption, region, or visual-search result unless a new Activity records it. |
| Projection and snapshot rebuild | Capability projections rebuild from recorded Activities, Perceptions, Derivations, selectors, and access decisions; affected snapshots invalidate on pipeline or erasure change. |
| Required raw inputs | Genesis selectors, reference-authorized Activity edges, Perception lifecycle, source locators, influence, and erasure closure supply every required seam. |
| Disablement and persisted meaning | Each capability disables independently. Derived outputs become inactive or are erased under policy; source records retain meaning. |

### Required decisions

The named capability owner, operator, and security owner approve each `stage-10-<capability>-gate`, including whether the capability is declined, experimental, or activation-ready. Generated episodes, reinspection, OCR, captions, region grounding, and visual search are separate decisions with separate deadlines before their experiments. Absence blocks only that capability. Scene graphs remain explicitly declined for this stage and reopen only with a dedicated graph-writer safety plan. These map to the corresponding safely deferred and stage-gating rows in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Disable each capability independently. Preserve generated records for audit or erase them under the applicable policy.

### Stop conditions

Stop on source laundering, hidden reinspection, uncontrolled cost, concrete invention presented as evidence, or capability bundling without separate proof.

### Deferred

Scene graphs remain furthest deferred. Any capability without a passed gate remains disabled.

## Stage 11: drift, exceptions, and off-turn retirement

### Prerequisites

Longitudinal baselines, event-sourced job machinery, and compare-at-commit behavior pass.

### Semantic contracts

Activate drift canaries, an operator exception queue, and idempotent background jobs. Retire superseded maintenance only after equivalence and longitudinal tests. Separate mechanical projection, derived-Assertion, and retraction-proposal authority.

### Owning chapters

[Off-turn work](off-turn.md), [verified write](verified-write.md), and [confidence](confidence.md).

### Evidence gates

Lease expiry, duplicate attempt, crash recovery, cancellation, supersession, poison handling, relevant-change requeue, stale-head compare, and authority-boundary fixtures pass. Queue-by-change follows scaling evidence. Exact concurrency machinery remains design work ([maintenance](../docs/maintenance-passes.md), [welding](research/2026-07-24/lanes/welding.md), [survey](research/2026-07-24/lanes/survey-giants.md)).

### Compatibility

| Field | Contract |
|---|---|
| New record and definition versions | Additive job, lease, attempt, cancellation, supersession, poison, criterion, drift, and exception-queue versions. |
| Pre-stage log readability | Existing semantic records remain readable with no job history. Jobs target only recorded source heads and never invent a prior attempt. |
| Projection and snapshot rebuild | Job state rebuilds from events; worker snapshots are disposable and invalidate on lease, target-head, criterion, or authority change. |
| Required raw inputs | Source heads, policy/schema/time/identity change events, typed authority, influence, and compare-at-commit dependencies already exist. |
| Disablement and persisted meaning | Stopping workers retains pending state and changes no semantic record. Re-enabling cannot duplicate committed output. |

### Required decisions

The operations owner, architecture owner, and operator approve `stage-11-job-policy`: lease duration, retry bound, poison threshold, relevant-change keys, compare-at-commit behavior, exception ownership, and retirement equivalence threshold. Baselines and thresholds are frozen before workers run. Missing concurrency decisions block stage entry; exploration and proactive initiation remain separate unapproved experiments. These map to off-turn idempotency, drift, exception review, and proactive salience in [the unresolved register](confidence.md#unresolved-item-register).

### Rollback/disable

Stop workers and retain pending state. Re-enable from the recorded target head without duplicating committed outputs.

### Stop conditions

Stop on unbounded retry, duplicate publication, stale commit, authority escalation, constant whole-store review, or longitudinal regression.

### Deferred

Exploration and proactive initiation remain optional disabled experiments with separate privacy, cost, and yield gates.

## Executable stage handoff register

This register is complete for roadmap handoff. A later implementation plan may make a threshold stricter, but it cannot weaken or postpone one after results are visible. `Entry` means the stage cannot begin. `Activation` means implementation may run only in shadow or disabled mode until the gate passes.

| Stage | Decision artefact and owners | Alternatives and baseline | Threshold or structural oracle | Freeze point and deadline | Blocker | Automated and scenario evidence IDs |
|---|---|---|---|---|---|---|
| Stage -1 | `stage--1-permanence-decision`; architecture, security, storage, and operator owners | Accept each proposed identity/fold/storage choice or revise before genesis; baseline is the current non-successor model with no compatibility promise | Every counterfactual rebuilds without invented fields, ID reuse, hidden flow, or payload resurrection; all genesis-blocking register rows have an approved rule | Freeze before Stage 1 candidate encoding; deadline is Stage -1 exit | Entry to Stage 1 and every successor schema stage | `perm-negation`, `perm-modality`, `perm-correction`, `perm-retraction`, `perm-hidden-support`, `perm-event-split`, `perm-identity-sever`, `perm-schema-change`, `perm-witness-policy`, `perm-support-policy`, `perm-reinspection`, `perm-shared-erasure`, `perm-derivation-invalidation` |
| Stage 0 | `stage-0-measurement-contract`; architecture/operator, security, and eval owners | Treatments 0a–0f against the named current or source-only baseline; reject an unmeasurable treatment | Each study publishes sample, uncertainty, cost/latency, privacy tolerance, and a pre-registered pass rule; no result is interpreted under a post-result threshold | Freeze each study before its first sample; deadline is start of its dependent stage | Entry to the dependent measurement; activation of its dependent capability | `measure-0a-episode-ablation`, `measure-0b-witness`, `measure-0c-convergence`, `measure-0d-budget`, `measure-0e-query-class`, `measure-0f-multimodal` |
| Stage 1 | `stage-1-model-selection`; architecture owner with security review | Candidate canonical encodings and object boundaries versus source-only baseline | All canonical fixture rows compile without implicit fields, produce exact selector/digest values, and replay equal IDs, folds, probes, surviving payload sets, and digests; critic false requirements are zero | Freeze before Stage 2 schema implementation; deadline is Stage 1 exit | Entry to Stage 2 | `model-fixture-compile`, `model-replay-digest`, `model-source-only`, `model-selector-cbor`, `model-transition-conflict` |
| Stage 2 | `stage-2-genesis-contract`; architecture, security, storage, and operator owners | Implement the frozen complete substrate or do not create a successor; source-only disablement is the operational baseline | Zero hidden-input residue in deterministic probes; every erasure cell has an implemented outcome; all folds replay; dated descriptions fire zero actions; shared-reference erasure preserves exactly the bytes the surviving reference retains | Freeze event/definition v1 before first successor genesis; deadline is creation of the first successor instance | Entry to successor genesis | `genesis-replay`, `genesis-subject-guard`, `genesis-zero-residue`, `genesis-erasure-crossproduct`, `genesis-restore-order`, `genesis-dated-no-fire`, `genesis-shared-wrapper` |
| Stage 3 | `stage-3-write-activation`; architecture, model/eval, and operator owners | Authoritative structured writes versus source-only and shadow proposal baselines | Pre-registered Stage 0c thresholds for fidelity, completion, blocks, retry, latency, cost, false critic rejection, and non-interference all pass; fault injection yields no partial/duplicate/stale publication | Freeze thresholds before the first shadow corpus run; deadline is authoritative-write activation | Activation; Stage 3 may begin only in shadow | `write-fidelity`, `write-convergence`, `write-constraint-tax`, `write-source-fallback`, `write-abort`, `write-retry`, `write-crash`, `write-atomic`, `write-stale-head` |
| Stage 4 | `stage-4-time-policy`; architecture, temporal-policy, and operator owners | Last-day, skip, clamp, and business-calendar choices versus reject-ambiguity baseline | Zero dated-description firings; correction preserves source/IDs; all vague-time/timezone/month-end vectors return their registered result or explicit unknown | Freeze each policy and threshold before temporal scenario execution; deadline is activation of that policy | Activation of each temporal policy | `time-dated-no-fire`, `time-correction`, `time-vague`, `time-timezone`, `time-last-day`, `time-skip`, `time-clamp`, `time-business-calendar` |
| Stage 5 | `stage-5-event-schema-policy`; architecture, schema-governance, privacy, and operator owners | Universal roles only versus each proposed typed subrole; separate Events versus accepted co-reference; suppress versus incomplete shell | Repeated same-participant Events remain distinct absent strong evidence; merge/sever restores exact sources; every partial projection is semantically safe; alias cycles reject; historical definitions replay unchanged | Freeze each definition/policy version before its first authoritative use; deadline is Stage 5 activation for that item | Activation per subrole, Event type, co-reference policy, or definition version | `event-distinct`, `event-merge-sever`, `event-partial-projection`, `schema-alias-cycle`, `schema-historical-replay` |
| Stage 6 | `stage-6-identity-operation`; identity, security, and operator owners | Operator-confirmed disjoint composites versus no composite; autonomous scoring remains disabled | Accepted composites are disjoint; response-affecting use has disclosure clearance; merge/sever replay is equivalent; sibling history does not migrate; overlap always denies operation | Freeze operational policy before first accepted composite; deadline is Stage 6 activation | Activation; autonomous scoring remains separately blocked | `identity-overlap`, `identity-disclosure`, `identity-merge-sever`, `identity-sibling-history`, `identity-one-handle` |
| Stage 7 | `stage-7-support-policy`; belief, privacy, architecture, and operator owners | Visible-attestation counts/dependence discount versus no computed support; fusion remains disabled | Hidden support causes zero visible delta; shared-room and relay dependence discount; last-support withdrawal demotes/omits as registered; mechanical contradiction and contest classify exactly | Freeze support/dependence/contradiction versions before evaluation; deadline is Stage 7 activation | Activation; fusion and general arbitration remain separately blocked | `support-hidden-zero`, `support-shared-room`, `support-relay-return`, `support-reliability`, `support-last-withdrawal`, `support-contest` |
| Stage 8 | `stage-8-privacy-extension`; security, storage, privacy-policy, and operator owners | Each richer principle or erasure operation against the Stage 2 fail-closed baseline | The specific witness, influence, authorization, package/backup, restore, and non-interference vectors pass with zero prohibited flow; purpose/reciprocity require a separately approved execution-purpose model | Freeze each extension before its scenario run; deadline is activation of that extension | Activation per extension | `privacy-principle-*`, `privacy-hidden-influence`, `privacy-package`, `privacy-backup`, `privacy-restore`, `privacy-external-copy` |
| Stage 9 | `stage-9-ingestion-policy`; memory, privacy, model/eval, and operator owners | Source-first bounded ingest versus no automatic ingest; procedural/working lifecycle options | Pre-registered Stage 0c/0d precision, cost, latency, and bounded-work thresholds pass; source-only fallback is lossless; per-document audience and taint produce zero prohibited flow | Freeze corpus, budgets, and thresholds before ingest evaluation; deadline is activation per ingest type | Activation per document/media/procedural capability | `ingest-precision`, `ingest-cost`, `ingest-bounded`, `ingest-source-fallback`, `ingest-audience`, `ingest-taint` |
| Stage 10 | `stage-10-extension-<capability>`; multimodal/episodic, privacy, model/eval, and operator owners | Each of episodes, reinspection, OCR, captions, regions, visual search, and scene graphs independently versus disabled baseline | Its Stage 0a/0f pre-registered benefit exceeds cost/latency budget with zero privacy failures; exact provenance/correction/erasure vectors pass; passing one capability gives no evidence for another | Freeze per-capability model, corpus, budget, and threshold before evaluation; deadline is that capability's activation | Activation per capability | `episode-ablation`, `inspect-authorized`, `inspect-denied`, `ocr-correction`, `caption-provenance`, `region-selector`, `visual-retrieval`, `scenegraph-writer-safety` |
| Stage 11 | `stage-11-background-policy`; architecture, operations, privacy, and operator owners | Event-triggered bounded jobs versus disabled/manual maintenance; exploration/initiation remain separately disabled | One valid outcome under crash/race/retry/supersession/poison/stale head; no authority escalation; longitudinal equivalence passes before old maintenance retires; cost and yield stay within pre-registered budgets | Freeze job policy and budgets before shadow run; deadline is activation or retirement of the affected maintenance path | Activation per job; retirement requires equivalence; exploration/initiation separately blocked | `job-crash`, `job-race`, `job-retry`, `job-supersede`, `job-poison`, `job-stale-head`, `job-authority`, `drift-longitudinal`, `maintenance-equivalence` |

## Handoff requirements

A stage implementation plan must state:

- exact event and definition versions it adds;
- readability of every pre-stage log version;
- projection rebuild and snapshot behavior;
- raw inputs already present and any genesis-blocking omission;
- whether disabling the policy changes persisted meaning;
- automated structural oracles and named scenario tests;
- the named decision artefact, owner, alternatives, baseline, threshold, freeze point, deadline, and whether absence blocks entry or activation;
- rollback operation, stop conditions, and unresolved-register links;
- the chapter contracts implemented by each component.

Composite benchmark scores are not acceptance targets. Structural behavior uses deterministic oracles. Model judges are limited to linguistic behavior. [`confidence.md`](confidence.md) records the evidence and caveat behind every stage gate.

Each completed stage drains its implemented normative contracts into `docs/` as as-built documentation in the same change that activates the implementation. Only material actually implemented and verified is marked or removed from `docs-future/`; unresolved, disabled, experimental, or later-stage material remains here. The future tree can be deleted only after every required and accepted extension has landed in `docs/` or has an explicit, recorded decline decision. A stage completion that leaves its active contract only in `docs-future/` is incomplete.