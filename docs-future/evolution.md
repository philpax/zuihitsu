# Evolution

The current instance remains outside the successor boundary. It is neither migrated nor used as a compatibility target. The successor begins its permanence contract only at the first real genesis.

## Programme taxonomy

The roadmap has four kinds of programme item.

- A numbered pre-genesis stage is mandatory. Stages 0 through 12 all run. A stage has no selection, skip, or status field.
- A required substrate capability supplies a permanent storage, replay, safety, or operational contract. Its evidence is required for genesis even when no active background or policy capability uses it.
- An initial-policy capability is evaluated in Stages 9 through 12. The versioned genesis-selection record selects or declines each capability independently. The selection record does not select a stage.
- A named activation gate evaluates a capability that is optional, post-genesis, or separately isolated. The gate may pass, fail, be declined, or remain deferred without blocking genesis unless the selection record marks its capability as `initial_policy`.
- A lifecycle phase is the genesis freeze review, the whole-system rehearsal, or the first real genesis. These phases consume the closure of the selected capability entries and the required substrate.

Stages 0 through 7 establish the candidate contract, evidence, reference model, bounded substrate slices, and the first audience-resolved read/write path. Stage 8 establishes the required operational substrate and the disabled-worker contract. Stages 9 through 12 evaluate the initial temporal, Event and relation, identity, and support policies. The numbered sequence is never selected or skipped.

Bulk ingestion, generated episodes, procedural and working memory, historical reinspection, OCR, generated captions, region grounding, visual retrieval, scene-graph writing, richer temporal inference, broad Event vocabularies, support fusion, autonomous identity, richer transmission, exploration, proactive initiation, and subagent spawning are capability entries. Each has a named activation gate. A capability entry does not become active because its owning numbered stage runs.

The stage names and ownership in `docs-future/research/` are dated research terminology. They record the proposal and terminology current at the date of collection. They do not define the current roadmap ownership. The research files remain unchanged.

## Experimental-state rules

Every stage from Stage 0 through Stage 12 is a pre-genesis experimental increment. No pre-genesis successor log, event version, ID encoding, snapshot, or projection is compatibility-bearing. An increment may break and regenerate experimental logs, fixtures, encodings, projections, and snapshots when evidence falsifies the current design. Measurements, fixture inputs, expected results, failures, and decision rationales remain part of the evidence record even when their candidate wire format is discarded.

Each increment leaves the repository buildable and its relevant automated tests passing. It need not leave a usable agent. Each increment reduces uncertainty through a named executable artefact or measurement. The architecture remains provisional until the genesis freeze review. Freezing an experiment's metrics or expected results does not freeze the successor schema.

Each pre-genesis increment contains `Prerequisite node IDs`, `Evidence prerequisite IDs`, `Semantic contracts`, `Owning chapters`, `Evidence produced`, `Required decisions`, `Falsification and stop conditions`, `Experimental-state disposition`, `Blocking scope`, and `Deferred`. Final-design fallback or disablement properties remain part of the semantic contract where applicable. Pre-genesis compatibility and user rollback are not roadmap requirements.

## Canonical identifiers and dependency register

The roadmap uses the following identifier namespaces: `stage:<n>`, `capability:<name>`, `gate:<name>`, `lifecycle:<name>`, `owner:<name>`, `handoff:<name>`, `evidence:<name>`, `oracle:<name>`, `invariant:<name>`, `test:<name>`, `measurement:<name>`, the declared unresolved-ID namespace, and `obligation:<name>`. An identifier is declared once in its owning register. References use the exact identifier and do not depend on row wording.

The dependency register has exactly these fields.

| node_id | node_kind | prerequisite_node_ids | evidence_prerequisite_ids | conditional_activation | initial_activation_required |
|---|---|---|---|---|---|
| `stage:0` | `stage` | none | none | `false` | `true` |
| `stage:1` | `stage` | `stage:0` | `evidence:stage0-contract` | `false` | `true` |
| `stage:2` | `stage` | `stage:0` | `evidence:stage0-contract`, `evidence:1c-contract` | `false` | `true` |
| `stage:3` | `stage` | `stage:2` | `evidence:stage2-reference` | `false` | `true` |
| `stage:4` | `stage` | `stage:3` | `evidence:stage2-reference` | `false` | `true` |
| `stage:5` | `stage` | `stage:4` | `evidence:stage2-reference`, `evidence:1b-witness` | `false` | `true` |
| `stage:6` | `stage` | `stage:5` | `evidence:stage2-reference`, `evidence:1d-budgets` | `false` | `true` |
| `stage:7` | `stage` | `stage:6` | `evidence:stage2-reference`, `evidence:1c-contract`, `evidence:1c-measurement` | `false` | `true` |
| `stage:8` | `stage` | `stage:7` | `evidence:stage7-vertical-slice` | `false` | `true` |
| `stage:9` | `stage` | `stage:8` | `evidence:stage2-reference`, `evidence:stage7-vertical-slice` | `false` | `true` |
| `stage:10` | `stage` | `stage:9` | `evidence:stage2-reference`, `evidence:stage7-vertical-slice` | `false` | `true` |
| `stage:11` | `stage` | `stage:10` | `evidence:stage2-reference`, `evidence:stage7-vertical-slice` | `false` | `true` |
| `stage:12` | `stage` | `stage:11` | `evidence:stage2-reference`, `evidence:stage7-vertical-slice` | `false` | `true` |
| `capability:source-activity-substrate` | `capability` | `stage:3` | `evidence:stage3-source-activity` | `true` | `true` |
| `capability:assertion-definition-substrate` | `capability` | `stage:4` | `evidence:stage4-assertion-definition` | `true` | `true` |
| `capability:audience-influence-substrate` | `capability` | `stage:5` | `evidence:stage5-audience-influence` | `true` | `true` |
| `capability:artefact-erasure-recovery-substrate` | `capability` | `stage:6` | `evidence:stage6-erasure-recovery` | `true` | `true` |
| `capability:read-write-vertical-slice` | `capability` | `stage:7` | `evidence:stage7-vertical-slice` | `true` | `true` |
| `capability:operational-job-substrate` | `capability` | `stage:8` | `evidence:stage8-operational-substrate` | `true` | `true` |
| `capability:disabled-worker-contract` | `capability` | `stage:8` | `evidence:stage8-disabled-worker` | `true` | `true` |
| `capability:inter-agent-status-freshness` | `capability` | `stage:5` | `evidence:stage5-status-freshness` | `true` | `true` |
| `capability:initial-temporal-policy` | `capability` | `stage:9` | `evidence:stage9-temporal-policy` | `true` | `false` |
| `capability:initial-event-role-relation-policy` | `capability` | `stage:10` | `evidence:stage10-event-policy` | `true` | `false` |
| `capability:initial-identity-policy` | `capability` | `stage:11` | `evidence:stage11-identity-policy` | `true` | `false` |
| `capability:initial-support-policy` | `capability` | `stage:12` | `evidence:stage12-support-policy` | `true` | `false` |
| `capability:generated-episodes` | `capability` | `stage:3`, `stage:5`, `stage:8` | `evidence:gate-generated-episodes` | `true` | `false` |
| `capability:bulk-ingestion` | `capability` | `stage:3`, `stage:5`, `stage:8` | `evidence:gate-bulk-ingestion` | `true` | `false` |
| `capability:procedural-memory` | `capability` | `stage:8` | `evidence:gate-procedural-memory` | `true` | `false` |
| `capability:working-memory` | `capability` | `stage:8` | `evidence:gate-working-memory` | `true` | `false` |
| `capability:historical-reinspection` | `capability` | `stage:3`, `stage:5`, `stage:6`, `stage:8` | `evidence:gate-historical-reinspection` | `true` | `false` |
| `capability:ocr` | `capability` | `stage:3`, `stage:5`, `stage:6`, `stage:8` | `evidence:gate-ocr` | `true` | `false` |
| `capability:generated-captions` | `capability` | `stage:3`, `stage:5`, `stage:6`, `stage:8` | `evidence:gate-generated-captions` | `true` | `false` |
| `capability:region-grounding` | `capability` | `stage:3`, `stage:5`, `stage:6`, `stage:8` | `evidence:gate-region-grounding` | `true` | `false` |
| `capability:visual-retrieval` | `capability` | `stage:3`, `stage:5`, `stage:6`, `stage:8` | `evidence:gate-visual-retrieval` | `true` | `false` |
| `capability:scene-graph-writer` | `capability` | `stage:3`, `stage:4`, `stage:5`, `stage:6`, `stage:8` | `evidence:gate-scene-graph-writer` | `true` | `false` |
| `capability:rich-recurrence` | `capability` | `stage:9` | `evidence:gate-rich-recurrence` | `true` | `false` |
| `capability:business-calendar-adjustment` | `capability` | `stage:9` | `evidence:gate-business-calendar-adjustment` | `true` | `false` |
| `capability:volatility-automation` | `capability` | `stage:8`, `stage:9` | `evidence:gate-volatility-automation` | `true` | `false` |
| `capability:habitual-deontic-inference` | `capability` | `stage:9` | `evidence:gate-habitual-deontic-inference` | `true` | `false` |
| `capability:qualitative-temporal-inference` | `capability` | `stage:9` | `evidence:gate-qualitative-temporal-inference` | `true` | `false` |
| `capability:autonomous-recurrence-interpretation` | `capability` | `stage:8`, `stage:9` | `evidence:gate-autonomous-recurrence-interpretation` | `true` | `false` |
| `capability:broad-event-role-vocabulary` | `capability` | `stage:10` | `evidence:gate-broad-event-role-vocabulary` | `true` | `false` |
| `capability:autonomous-event-merging` | `capability` | `stage:8`, `stage:10` | `evidence:gate-autonomous-event-merging` | `true` | `false` |
| `capability:numeric-support` | `capability` | `stage:12` | `evidence:gate-numeric-support` | `true` | `false` |
| `capability:support-fusion` | `capability` | `stage:8`, `stage:12` | `evidence:gate-support-fusion` | `true` | `false` |
| `capability:autonomous-contradiction-arbitration` | `capability` | `stage:8`, `stage:12` | `evidence:gate-autonomous-contradiction-arbitration` | `true` | `false` |
| `capability:autonomous-identity` | `capability` | `stage:8`, `stage:11` | `evidence:gate-autonomous-identity` | `true` | `false` |
| `capability:cross-instance-identity` | `capability` | `stage:5`, `stage:8`, `stage:11` | `evidence:gate-cross-instance-identity` | `true` | `false` |
| `capability:transmission-reciprocity` | `capability` | `stage:5`, `stage:8` | `evidence:gate-transmission-reciprocity` | `true` | `false` |
| `capability:transmission-purpose-limitation` | `capability` | `stage:5`, `stage:8` | `evidence:gate-transmission-purpose-limitation` | `true` | `false` |
| `capability:remote-actionable-status` | `capability` | `stage:5`, `stage:8` | `evidence:gate-remote-actionable-status` | `true` | `false` |
| `capability:general-runtime-faithfulness-checking` | `capability` | `stage:8` | `evidence:gate-general-runtime-faithfulness-checking` | `true` | `false` |
| `capability:worker-policy` | `capability` | `stage:8` | `evidence:gate-worker-policy` | `true` | `false` |
| `capability:maintenance-retirement` | `capability` | `stage:8` | `evidence:gate-maintenance-retirement` | `true` | `false` |
| `capability:drift-response` | `capability` | `stage:8` | `evidence:gate-drift-response` | `true` | `false` |
| `capability:exception-processing` | `capability` | `stage:8` | `evidence:gate-exception-processing` | `true` | `false` |
| `capability:exploration` | `capability` | `stage:8` | `evidence:gate-exploration` | `true` | `false` |
| `capability:proactive-initiation` | `capability` | `stage:8` | `evidence:gate-proactive-initiation` | `true` | `false` |
| `capability:subagent-spawning` | `capability` | `stage:8` | `evidence:gate-subagent-spawning` | `true` | `false` |
| `gate:generated-episodes` | `activation_gate` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:operational-job-substrate` | `evidence:gate-generated-episodes` | `true` | `false` |
| `gate:bulk-ingestion` | `activation_gate` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:operational-job-substrate` | `evidence:gate-bulk-ingestion` | `true` | `false` |
| `gate:procedural-memory` | `activation_gate` | `capability:operational-job-substrate` | `evidence:gate-procedural-memory` | `true` | `false` |
| `gate:working-memory` | `activation_gate` | `capability:operational-job-substrate` | `evidence:gate-working-memory` | `true` | `false` |
| `gate:historical-reinspection` | `activation_gate` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | `evidence:gate-historical-reinspection` | `true` | `false` |
| `gate:ocr` | `activation_gate` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | `evidence:gate-ocr` | `true` | `false` |
| `gate:generated-captions` | `activation_gate` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | `evidence:gate-generated-captions` | `true` | `false` |
| `gate:region-grounding` | `activation_gate` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | `evidence:gate-region-grounding` | `true` | `false` |
| `gate:visual-retrieval` | `activation_gate` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | `evidence:gate-visual-retrieval` | `true` | `false` |
| `gate:scene-graph-writer` | `activation_gate` | `capability:source-activity-substrate`, `capability:assertion-definition-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | `evidence:gate-scene-graph-writer` | `true` | `false` |
| `gate:rich-recurrence` | `activation_gate` | `capability:initial-temporal-policy` | `evidence:gate-rich-recurrence` | `true` | `false` |
| `gate:business-calendar-adjustment` | `activation_gate` | `capability:initial-temporal-policy` | `evidence:gate-business-calendar-adjustment` | `true` | `false` |
| `gate:volatility-automation` | `activation_gate` | `capability:initial-temporal-policy`, `capability:operational-job-substrate` | `evidence:gate-volatility-automation` | `true` | `false` |
| `gate:habitual-deontic-inference` | `activation_gate` | `capability:initial-temporal-policy` | `evidence:gate-habitual-deontic-inference` | `true` | `false` |
| `gate:qualitative-temporal-inference` | `activation_gate` | `capability:initial-temporal-policy` | `evidence:gate-qualitative-temporal-inference` | `true` | `false` |
| `gate:autonomous-recurrence-interpretation` | `activation_gate` | `capability:initial-temporal-policy`, `capability:operational-job-substrate` | `evidence:gate-autonomous-recurrence-interpretation` | `true` | `false` |
| `gate:broad-event-role-vocabulary` | `activation_gate` | `capability:initial-event-role-relation-policy` | `evidence:gate-broad-event-role-vocabulary` | `true` | `false` |
| `gate:autonomous-event-merging` | `activation_gate` | `capability:initial-event-role-relation-policy`, `capability:operational-job-substrate` | `evidence:gate-autonomous-event-merging` | `true` | `false` |
| `gate:numeric-support` | `activation_gate` | `capability:initial-support-policy` | `evidence:gate-numeric-support` | `true` | `false` |
| `gate:support-fusion` | `activation_gate` | `capability:initial-support-policy`, `capability:operational-job-substrate` | `evidence:gate-support-fusion` | `true` | `false` |
| `gate:autonomous-contradiction-arbitration` | `activation_gate` | `capability:initial-support-policy`, `capability:operational-job-substrate` | `evidence:gate-autonomous-contradiction-arbitration` | `true` | `false` |
| `gate:autonomous-identity` | `activation_gate` | `capability:initial-identity-policy`, `capability:operational-job-substrate` | `evidence:gate-autonomous-identity` | `true` | `false` |
| `gate:cross-instance-identity` | `activation_gate` | `capability:audience-influence-substrate`, `capability:initial-identity-policy`, `capability:operational-job-substrate` | `evidence:gate-cross-instance-identity` | `true` | `false` |
| `gate:transmission-reciprocity` | `activation_gate` | `capability:audience-influence-substrate`, `capability:operational-job-substrate` | `evidence:gate-transmission-reciprocity` | `true` | `false` |
| `gate:transmission-purpose-limitation` | `activation_gate` | `capability:audience-influence-substrate`, `capability:operational-job-substrate` | `evidence:gate-transmission-purpose-limitation` | `true` | `false` |
| `gate:remote-actionable-status` | `activation_gate` | `capability:inter-agent-status-freshness`, `capability:operational-job-substrate` | `evidence:gate-remote-actionable-status` | `true` | `false` |
| `gate:general-runtime-faithfulness-checking` | `activation_gate` | `capability:operational-job-substrate` | `evidence:gate-general-runtime-faithfulness-checking` | `true` | `false` |
| `gate:worker-policy` | `activation_gate` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | `evidence:gate-worker-policy` | `true` | `false` |
| `gate:maintenance-retirement` | `activation_gate` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | `evidence:gate-maintenance-retirement` | `true` | `false` |
| `gate:drift-response` | `activation_gate` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | `evidence:gate-drift-response` | `true` | `false` |
| `gate:exception-processing` | `activation_gate` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | `evidence:gate-exception-processing` | `true` | `false` |
| `gate:exploration` | `activation_gate` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | `evidence:gate-exploration` | `true` | `false` |
| `gate:proactive-initiation` | `activation_gate` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | `evidence:gate-proactive-initiation` | `true` | `false` |
| `gate:subagent-spawning` | `activation_gate` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | `evidence:gate-subagent-spawning` | `true` | `false` |
| `lifecycle:genesis-freeze` | `lifecycle` | `stage:12` | `evidence:freeze-record`, `evidence:all-genesis-blockers` | `false` | `true` |
| `lifecycle:whole-system-rehearsal` | `lifecycle` | `lifecycle:genesis-freeze` | `evidence:freeze-record` | `false` | `true` |
| `lifecycle:first-real-genesis` | `lifecycle` | `lifecycle:whole-system-rehearsal` | `evidence:rehearsal-report` | `false` | `true` |

Stage prerequisites in this register refer only to earlier numbered stages or named evidence packages. The Stage 1 evidence packages `evidence:1a-episode-ablation`, `evidence:1b-witness`, `evidence:1c-contract`, `evidence:1c-measurement`, `evidence:1d-budgets`, `evidence:1e-query-classification`, and `evidence:1f-multimodal-recall` may run in parallel after `stage:0` has preregistered their inputs. Stage 2 depends on `evidence:1c-contract`, not on `evidence:1c-measurement`; the measurement consumes the reference model only for the comparison phase. Stage 7 remains shadow-only until `evidence:1c-measurement` and `oracle:stage7-authority` pass. Every capability or gate that uses bounded work names `stage:8` explicitly. Stages 9 through 12 use synchronous, job-free evidence for their initial-policy entries.

The audit compares the structured prerequisite sets in every stage and gate body with the corresponding dependency-register row in both directions. A prerequisite concept that appears only in prose fails the audit. A reference to a prerequisite uses its canonical ID.

## Genesis-selection-record schema

Stage 0 owns the versioned `genesis-selection-record`. The record contains capability entries only. It contains no stage-status, stage-selection, stage-skip, or stage-version field. Numbered stages remain mandatory regardless of the entries.

The schema is:

```text
GenesisSelectionRecord {
    record_id: selection:<versioned-id>,
    schema_version: <version>,
    decision_source_head: <immutable decision reference>,
    entries: [
        GenesisCapabilitySelection {
            capability_id: capability:<name>,
            status: required_substrate | initial_policy | activation_gate | declined,
            selected_policy: <policy id or none>,
            selected_policy_version: <version or none>,
            prerequisite_capability_ids: [capability:<name>],
            evidence_ids: [evidence:<name>],
            executable_oracle_ids: [oracle:<name>],
            disabled_behaviour_oracle: oracle:<name>,
            additive_seam_reference: <stage, contract, or artefact reference>,
            operator_decision: <select, decline, defer, or reject with reason>,
            decision_source_head: <immutable decision reference>
        }
    ]
}
```

The `status` enum is exact. `required_substrate` entries cannot be declined. `initial_policy` entries identify capabilities selected for initial policy. `activation_gate` entries identify capabilities that remain independently gated. `declined` entries identify capabilities that the operator rejects for the candidate. The operator fixes the candidate selection before dependent initial-policy evidence runs. A selection change recomputes the transitive prerequisite closure and invalidates affected evidence.

A freeze record computes `required_substrate` entries, `initial_policy` entries, and their transitive `prerequisite_capability_ids`. The record also requires the `disabled_behaviour_oracle` and `additive_seam_reference` for every `activation_gate` and `declined` entry. The freeze does not require an activation gate to pass merely because the gate exists. A selected activation-gate capability is represented as `initial_policy` in a new selection record and then requires its gate evidence.

## Stage 0: candidate contract and programme setup

### Prerequisite node IDs

`stage:0` has no prerequisite node IDs.

### Evidence prerequisite IDs

`stage:0` has no evidence prerequisite IDs.

### Semantic contracts

Stage 0 absorbs the candidate permanence inventory and the preregistration portion of the former measurement stage. It records candidate genesis-bearing stable identities, raw inputs, source kinds, lifecycle axes, registered definitions, replay and upcast rules, transition folds, selector semantics, storage boundaries, privacy boundaries, and counterfactuals.

The candidate inventory exercises negation, modality, temporal correction, per-teller retraction, hidden support, Event splitting, identity severance, schema change, witness-policy change, support-policy change, image reinspection, shared-reference erasure, and Derivation invalidation. The inventory remains revisable until the genesis freeze.

Stage 0 defines the schema-neutral fixture vocabulary, dependency graph, capability classification, evidence contracts, responsible repository areas, artefact paths, and decision records. Alternatives cover stable IDs, selectors, transition folds, seed definitions, storage partitions, authoritative erasure-ledger design, restore authority, and external-copy accounting.

The erasure ledger remains an unresolved candidate authority. Stage 0 compares replication, loss, rollback, disaster recovery, authority continuity, and fail-closed serving consequences. The normative requirement remains unchanged: erased data cannot regain authority.

Stage 0 creates one versioned genesis-selection-record entry for every capability. The entry records the capability ID, status, selected policy and version where applicable, prerequisite capability IDs, evidence IDs, executable oracle IDs, disabled-behaviour oracle, additive seam, operator decision, and decision source head. Stage 0 does not select or skip a numbered stage.

The inventory executable is `stage-0-candidate-permanence-inventory`. No legacy executable name identifies a current artefact.

### Owning chapters

[Overview](overview.md#permanence-contract), [assertions](statements.md), [artefacts](artefacts-and-perceptions.md), [privacy](privacy-and-provenance.md), [relations](relations.md), and [confidence](confidence.md).

### Evidence produced

Stage 0 produces `evidence:stage0-contract`, the candidate permanence inventory, the schema-neutral fixture vocabulary, the dependency register, the capability census contract, the genesis-selection-record schema, and the decision records. The inventory records counterfactual outcomes and surveyed migration costs. Append-only replay behaviour remains supporting evidence rather than a schema freeze ([storage](../docs/events-and-storage.md), [verification](research/2026-07-24/verification/part-b.md), [survey](research/2026-07-24/lanes/survey-giants.md)).

### Required decisions

The architecture, security, storage, and operator owners record candidate alternatives for stable IDs, selector encoding, transition folds, seed definitions, authoritative-ledger restore filtering, five managed-live deletion surfaces, managed-restore non-authority, and external-copy accounting. Each unresolved choice links to a declared unresolved-item entry and an `obligation:<name>` entry where applicable. No candidate becomes compatibility-bearing at Stage 0.

### Falsification and stop conditions

Stage 0 returns the relevant design to revision if a counterfactual requires destructive merge, stable-ID reuse, invented historical values, hidden-input leakage, payload resurrection, or an unbounded erasure promise. The dependency audit also fails on an undeclared node, duplicate capability ID, stage-level selection, or a capability without a disabled-behaviour oracle and additive seam.

### Experimental-state disposition

Generated candidate logs, projections, snapshots, and encodings are disposable. The inventory inputs, expected counterfactual results, failures, measurements, and decision rationales are retained as evidence.

### Blocking scope

Stage 0 blocks every later stage and every lifecycle phase. It does not block an independently run planning task that produces no candidate evidence.

### Deferred

Implementation, policy activation, and every irreversible genesis choice remain deferred.

## Stage 1: baseline evidence

### Prerequisite node IDs

`stage:0`.

### Evidence prerequisite IDs

`evidence:stage0-contract`.

### Semantic contracts

Stage 1 recasts the baseline studies as independently executable evidence packages. The packages are `evidence:1a-episode-ablation`, `evidence:1b-witness`, `evidence:1c-contract`, `evidence:1c-measurement`, `evidence:1d-budgets`, `evidence:1e-query-classification`, and `evidence:1f-multimodal-recall`. They may run in parallel after `stage:0` has fixed their inputs, metrics, thresholds, and privacy tolerances. A dependent trial does not start until the `stage-1-measurement-contract` records its fixtures, baseline, measures, applicable budgets, and pass rule.

`1a-episode-ablation` measures generated-episode retrieval value and the non-evidence wall. `1b-witness` measures connector witness evidence, availability, presence, disclosure, and exposure assurance. `1c-contract` fixes the canonical fixture encoding and the extraction metrics before observation. `1c-measurement` measures extraction yield, convergence, source-only retrieval usefulness, operator schema burden on ordinary facts, review rounds, latency, and per-write economics against canonical Proposition, Assertion, and Attestation fixtures. `1d-budgets` measures browser, log, projection, blob, replay, and eval-package cost. `1e-query-classification` classifies query surfaces on real turns. `1f-multimodal-recall` measures recall using prior Perception, explicit reinspection, and textual memory. A usefulness or economics measure is an acceptance criterion for the treatment when that treatment changes the corresponding read or write path; it is not reporting-only.

Issue #123 witness, availability, and presence evidence is an explicit genesis blocker wherever it affects required substrate. Issue #66 console and log budget evidence is an explicit genesis blocker wherever it affects required substrate. The issue references describe evidence requirements and do not change current-system behaviour.

### Owning chapters

[Two traces](two-traces.md), [privacy](privacy-and-provenance.md), [verified write](verified-write.md), [query surface](query-surface.md), [artefacts](artefacts-and-perceptions.md), and [coverage](coverage.md).

### Evidence produced

Each package publishes executable fixtures, prompts, model and code versions, samples, measurements, costs, failures, uncertainty, baselines, and the preregistered pass rule. The `stage-1-measurement-contract` fixes each metric, minimum sample, confidence treatment, cost and latency budget, privacy failure tolerance, and pass rule before results are observed. For `evidence:1c-measurement` and `evidence:1d-budgets`, the pass rule treats each applicable usefulness and economics measure as an acceptance criterion, including source-only retrieval usefulness, operator schema burden on ordinary facts, review rounds, latency, log growth, and bounded console and replay costs. A failed applicable criterion is a negative result, not a reporting observation; the treatment is simplified or revised, or authority is withheld. The contract does not freeze the successor schema ([dual-trace study](research/2026-08-03/dual-trace.md), [log measurements](research/2026-08-06/log-measurements.md)).

### Required decisions

The architecture owner and operator own the measurement contract. Security owns privacy tolerances. The model and eval owner owns sampling and uncertainty. Missing or post-result thresholds invalidate only the dependent experiment. The architecture owner records whether each treatment is required substrate, an initial-policy candidate, an activation gate, or declined.

### Falsification and stop conditions

A treatment or dependent design returns to revision when results fail to distinguish it from the baseline, costs exceed the declared budget, or witness metadata cannot support fail-closed resolution. A result that is observed before its contract is fixed is not a valid gate result.

### Experimental-state disposition

Flawed measurement outputs and generated experimental stores are disposable. Inputs, preregistered thresholds, raw measurements, failures, uncertainty, and the rationale for rerunning or rejecting the treatment are retained.

### Blocking scope

`evidence:1c-contract`, `evidence:1b-witness`, and `evidence:1d-budgets` block the substrate stages that consume them. `evidence:1c-measurement` blocks authoritative publication in `stage:7` and the initial-policy writes that depend on it. A failed optional study blocks only its capability gate unless it exposes a shared substrate fault.

### Deferred

Generated episodes, automatic multimodal work, and authoritative extraction remain disabled.

## Stage 2: executable reference model

### Prerequisite node IDs

`stage:0`.

### Evidence prerequisite IDs

`evidence:stage0-contract`, `evidence:1c-contract`.

### Semantic contracts

Stage 2 builds only the schema-neutral fixture compiler, correctness-first reference materialiser, canonicalisation rules, transition properties, audience probes, semantic erasure expectations, replay digests, and counterfactuals. It does not assign production storage, connector integration, or agent-facing behaviour to the stage. A disposable, nonauthoritative synthetic model-facing experiment may use the reference model to test whether a proposed read or write surface is usable before full storage implementation. Such an experiment produces usability evidence only and cannot publish authoritative structure or substitute for the Stage 6 safety prerequisite or the Stage 7 authority gate.

The model covers Occasion, Activity, ArtefactReference, Proposition, Assertion, Attestation, Perception, Event, polarity, modality, source locators, transitions, and the first hard critics. Candidate encodings are disposable and are not compatibility-bearing version 1 records.

The cumulative reference oracle defines expected folds, selectors, surviving payload sets, audience probes, and digests. Later production-shaped stages compare against Stage 2 rather than against a legacy Stage 1 reference.

### Owning chapters

[Assertions](statements.md), [artefacts](artefacts-and-perceptions.md), [events](events-and-roles.md), [privacy](privacy-and-provenance.md), and [verified write](verified-write.md).

### Evidence produced

Stage 2 produces `evidence:stage2-reference` and the canonical schema-neutral fixture grammar specified by [assertions](statements.md#schema-neutral-fixture-record-grammar). The compiler handles byte-stable canonicalisation, compound and artefact-only Occasions, direct Activity sources, quotation, correction, per-teller audiences, source-only fallback, and generated transition chains. It emits replay digests, audience probes, and property-test counterexamples.

### Required decisions

The architecture owner records the compared candidate encodings, object boundaries, critic boundaries, and reference-fold semantics. Each decision links to the existing `stage:2`, `evidence:stage2-reference`, and affected register entries. The model remains provisional until the genesis freeze.

### Falsification and stop conditions

Stage 2 returns to revision if canonicalisation fails the preregistered convergence threshold, the fixture grammar requires implicit fields, transition property tests produce an invalid chain, critics require unsupported values, or object boundaries cannot express a canonical fixture.

### Experimental-state disposition

Candidate logs, projections, snapshots, implementation encodings, and failed materialiser implementations are disposable. Canonical fixture inputs, expected folds, audience probes, replay digests, generated counterexamples, measurements, and decision rationales are retained.

### Blocking scope

Stage 2 blocks the production-shaped substrate stages and all lifecycle phases. It does not make any candidate encoding permanent.

### Deferred

Production-shaped storage, authoritative structured writes, and policy automation remain deferred.

## Stage 3: source and Activity substrate

### Prerequisite node IDs

`stage:2`.

### Evidence prerequisite IDs

`evidence:stage2-reference`.

### Semantic contracts

Stage 3 isolates Occasion, Activity, ordered typed content parts, observed and recorded time, durable nondeterministic outcomes, source locators, direct source authority seams, and source-only retention. Source identities remain durable and distinct from generated interpretations. The slice supplies the minimum typed seams required by later stages without activating broad Event handling, audience-sensitive publication, or autonomous policy.

Every production-shaped fold and digest compares with `evidence:stage2-reference`. A source remains available when extraction fails or an optional interpreter is disabled. The source-only fallback is a final-design safety property.

### Owning chapters

[Two traces](two-traces.md), [artefacts](artefacts-and-perceptions.md), [verified write](verified-write.md), and [events and roles](events-and-roles.md).

### Evidence produced

Stage 3 produces `evidence:stage3-source-activity`, source and Activity fixtures, ordered-part tests, observed-versus-recorded-time tests, durable-outcome tests, locator tests, and source-only retention traces. Direct Activity sources and source authority are compared with the reference fold.

### Required decisions

The architecture and storage owners record source identity, content-part ordering, time fields, nondeterministic outcome storage, locator policy, and the direct source authority seam. The decision links to the existing `evidence:stage3-source-activity`, `evidence:stage2-reference`, and relevant invariant and obligation IDs.

### Falsification and stop conditions

Stage 3 returns to revision if a source identity is lost, content parts reorder, recorded nondeterminism is regenerated during replay, source locators cannot be retained, source-only fallback loses content, or the production-shaped fold diverges from the reference.

### Experimental-state disposition

Generated logs, projections, snapshots, and candidate encodings are disposable. Source fixtures, exact expected folds, replay digests, failure traces, and decision rationales are retained.

### Blocking scope

Stage 3 blocks all later substrate stages and every capability that consumes source or Activity records. It supplies the additive seams for generated episodes, bulk ingestion, historical reinspection, OCR, captions, region grounding, and visual retrieval without activating them.

### Deferred

Audience-sensitive publication, broad Event handling, autonomous identity, support fusion, generated episodes, and bulk ingestion remain deferred.

## Stage 4: Assertion and definition substrate

### Prerequisite node IDs

`stage:3`.

### Evidence prerequisite IDs

`evidence:stage2-reference`.

### Semantic contracts

Stage 4 isolates Proposition, Assertion, Attestation, SourceAuthority, Derivation record shapes, transition chains, polarity, modality, frames, stable definition IDs and versions, historical-definition replay, and governed schema activation seams. Correction and supersession preserve source records. A transition chain cannot invent an omitted historical value.

The stage includes the `principal` redirect evidence and the explicit successor seed-definition decision. Seed definitions remain versioned inputs. A later definition cannot reinterpret an earlier recorded definition during replay.

### Owning chapters

[Assertions](statements.md), [verified write](verified-write.md), [relations](relations.md), and [events and roles](events-and-roles.md).

### Evidence produced

Stage 4 produces `evidence:stage4-assertion-definition`, canonical-key fixtures, polarity and modality fixtures, transition property tests, stable definition replay tests, historical-definition fixtures, governed schema activation traces, and the `principal` redirect evidence. Production-shaped folds compare with `evidence:stage2-reference`.

### Required decisions

The architecture and schema owners record assertion and attestation shapes, source authority, derivation records, transition semantics, frame ownership, definition identity and versioning, historical replay, and the successor seed-definition set. The decisions link to `unresolved:successor-seed-definitions`, `unresolved:definition-replay-authority`, and `obligation:no-invented-history`.

### Falsification and stop conditions

Stage 4 returns to revision if a transition mutates a source, a correction invents a value, modality has no stable identity, definition replay changes a historical meaning, schema activation bypasses governance, the seed set is implicit, or production-shaped results diverge from the reference.

### Experimental-state disposition

Generated assertion logs, projections, snapshots, and candidate definition encodings are disposable. Fixture sources, exact expected folds, canonical keys, transition counterexamples, definition replay traces, and decision rationales are retained.

### Blocking scope

Stage 4 blocks Event, identity, support, and initial-policy evidence. It supplies stable definitions and transition seams for optional broad role vocabularies and autonomous Event merging without activating them.

### Deferred

Non-actual inference, autonomous acceptance, broad contradiction, and generated episodes remain deferred.

## Stage 5: audience and influence substrate

### Prerequisite node IDs

`stage:4`.

### Evidence prerequisite IDs

`evidence:stage2-reference`, `evidence:1b-witness`.

### Semantic contracts

Stage 5 isolates connector witness evidence, availability and presence semantics, central audience resolution, transmission evaluation, subject guards, complete InfluenceEnvelopes, audience-safe negative results, and zero-residue behaviour. The connector supplies evidence to a central resolver. Callers do not independently implement audience decisions.

Stage 5 resolves #123 before freezing the witness input shape. Teller-only fallback remains valid when witness evidence is absent. A hash is not a bearer credential. A denied result cannot reveal hidden support, hidden membership, or an existence fact through timing, counts, or error shape.

The stage defines and tests the inter-agent status freshness record and replay fold as a required genesis seam. The record contains issuer identity, a monotonic notice sequence, a freshness lease, signed status, key continuity, and stale state. Remote actionable use is the separate `capability:remote-actionable-status` activation gate.

### Owning chapters

[Privacy and provenance](privacy-and-provenance.md), [query surface](query-surface.md), [two traces](two-traces.md), and [coverage](coverage.md).

### Evidence produced

Stage 5 produces `evidence:stage5-audience-influence` and `evidence:stage5-status-freshness`. It publishes audience probes, influence-envelope folds, subject-guard traces, denied-result tests, zero-residue tests, connector witness fixtures, availability and presence fixtures, and status freshness replay traces. Outcomes compare with `evidence:stage2-reference`.

### Required decisions

The security, connector, architecture, and operator owners record witness inputs, availability semantics, audience resolver ownership, subject guards, InfluenceEnvelope fields, denied-result behaviour, teller-only fallback, status freshness, key continuity, and stale-state handling. Decisions link to `unresolved:connector-witness-assurance`, `unresolved:inter-agent-freshness-record`, and the relevant privacy obligations.

### Falsification and stop conditions

Stage 5 returns to revision if audience resolution is distributed among callers, hidden input changes a visible result, a denied result discloses hidden state, a witness cannot support fail-closed resolution, stale status is treated as actionable, key continuity is absent, or production-shaped outcomes diverge from the reference.

### Experimental-state disposition

Generated audience logs, projections, snapshots, status records, and candidate policy encodings are disposable. Witness fixtures, audience probes, exact expected influence envelopes, stale-state traces, failures, measurements, and decision rationales are retained.

### Blocking scope

Stage 5 blocks the read/write vertical slice, baseline erasure access checks, identity resolution, support projections, and all transmission-policy gates. The status freshness seam is required for genesis even when remote actionable status remains disabled.

### Deferred

Remote actionable status, reciprocity, purpose limitation, autonomous identity, and support fusion remain deferred.

## Stage 6: Artefact, erasure, and recovery substrate

### Prerequisite node IDs

`stage:5`.

### Evidence prerequisite IDs

`evidence:stage2-reference`, `evidence:1d-budgets`.

### Semantic contracts

Stage 6 is the sole numbered owner of the complete baseline runtime erasure and restore contract. It isolates Artefact and ArtefactReference identity, selectors, reference-authorised consumption, Perception and derived-Artefact seams, envelope and payload storage, reference-set retention, managed-live erasure closure, authoritative-ledger behaviour selected at Stage 0, restore filtering, backup handling, pending deletion, projection rebuilding, external-copy accounting, and console and log budgets.

Erasure closes over the five managed-live deletion surfaces under the writer lock. Reference-specific erasure preserves a blob while another live reference remains. Dependent projections and indexes are invalidated and rebuilt. The current authoritative ledger has monotonic positions. Managed restore acquires the writer lock before reading the current authoritative ledger, filters old backup material against that ledger, rebuilds projections while serving is blocked, and opens serving only after a verified current position is applied. Missing or unverifiable current authority keeps serving blocked. Pending physical deletion denies reads and Activity.

The lock-ownership proof is executable. Either an integration fault-injection test attempts concurrent ledger publication and proves that publication cannot enter the protected restore interval, or a structural or type-level test mechanically verifies shared lock ownership and scope for both paths and fails when either path changes. A prose explanation is not a passing result.

Richer transmission principles and optional reinspection remain outside this stage. Stage 6 does not make an erasure-ledger candidate permanent before the freeze.

### Owning chapters

[Privacy and provenance](privacy-and-provenance.md), [artefacts](artefacts-and-perceptions.md), [query surface](query-surface.md), and [coverage](coverage.md).

### Evidence produced

Stage 6 produces `evidence:stage6-erasure-recovery`. It includes production/reference comparisons and fault tests for reference retention, managed-live erasure closure, authoritative-ledger filtering, old-backup restoration, missing-current-ledger blocking, pending deletion, projection rebuild, external-copy accounting, and the shared writer lock. It retains `restore-old-backup-current-ledger`, `restore-missing-current-ledger`, `pending-live-blob-deletion`, `external-copy-bounded`, `erase-one-reference-retain-another`, and `privacy-restore-writer-lock` as named evidence. Supporting research includes [privacy research](research/2026-07-24/lanes/provenance-privacy.md), [current leak observation](research/2026-08-06/current-system-fixes.md), and [storage](../docs/events-and-storage.md).

### Required decisions

The security, storage, architecture, and operator owners record Artefact identity, selector policy, reference authorisation, derived-Artefact invalidation, storage partition, five-surface erasure closure, authoritative-ledger architecture, restore authority, lock scope, pending deletion, projection rebuilding, and external-copy accounting. Decisions link to `unresolved:authoritative-ledger-architecture`, `unresolved:recovery-authority-continuity`, `unresolved:erasure-authority-storage-restore`, `obligation:five-managed-live-surfaces`, `obligation:restore-writer-lock`, and `obligation:pending-deletion-denial`.

### Falsification and stop conditions

Stage 6 returns to revision if a managed-live surface is omitted, a shared reference is deleted incorrectly, managed restore grants authority to old material, restore reads authority before acquiring the writer lock, publication occurs while the lock is held, serving opens before filtering and rebuilding complete, the lock is released before serving opens, unverifiable authority opens serving, pending physical deletion permits a read or Activity, or external copies are reported as deleted. The candidate ledger architecture remains unresolved until these consequences are compared.

### Experimental-state disposition

Generated payload stores, blobs, backups, projections, snapshots, and policy encodings are disposable. Operational fixture JSON, expected states, ledger traces, lock-ownership traces, failure traces, measurements, and decision rationales are retained.

### Blocking scope

Stage 6 blocks the vertical slice, genesis freeze, rehearsal, and first real genesis. It supplies the permanent erasure, restore, selector, and additive Perception and Derivation seams for optional capabilities.

### Deferred

Richer transmission principles, optional reinspection, reciprocal policy, purpose limitation, and autonomous multimodal work remain deferred.

## Stage 7: first audience-resolved read/write vertical slice

### Prerequisite node IDs

`stage:6`.

### Evidence prerequisite IDs

`evidence:stage2-reference`, `evidence:1c-contract`, `evidence:1c-measurement`.

### Semantic contracts

Stage 7 defines the smallest complete experimental source and candidate transport path. An Occasion is retained. Extraction creates a durable proposal. Hard critics evaluate the proposal. Atomic publication creates conservative actual-positive candidate structure or source-only fallback. Central audience resolution produces a rendered source read or an explicit candidate-inspection read. Access accounting records delivered content. The stage does not validate default settled-only recall. That behaviour depends on [the selected initial support policy](belief.md#promotion-and-withdrawal) evaluated in Stage 12.

The path integrates only the object and safety seams from Stages 3 through 6. It keeps broad Event handling, autonomous identity, support fusion, generated episodes, and bulk ingestion disabled. It remains shadow-only until `evidence:1c-measurement` and `oracle:stage7-authority` pass. Authoritative publication requires both results as explicit prerequisites.

The extraction, grounding, and critic lifecycle remains append-only and proposal-based. Critics establish shape and limited grounding. They do not establish truth. Source-only fallback remains available when extraction, witness evidence, or critics fail.

### Owning chapters

[Verified write](verified-write.md), [write surface](write-surface.md), [query surface](query-surface.md), [assertions](statements.md), and [privacy and provenance](privacy-and-provenance.md).

### Evidence produced

Stage 7 produces `evidence:stage7-vertical-slice`. The harness compares production-shaped proposal folds and structural projections with `evidence:stage2-reference`. It measures fidelity, convergence, completion rate, blocks per write, retries, latency, constraint tax, temporal safety, source-only retrieval usefulness, operator schema burden on ordinary facts, review rounds, log growth, bounded console and replay cost, and non-interference under the Stage 1 thresholds. The slice passes only when the preregistered usefulness criteria hold, the operator schema burden remains within its declared budget, review and latency remain within their declared budgets, log growth and console/replay costs remain within their declared budgets, and the source-only path remains useful when extraction or review does not publish structure. A failure of any applicable criterion is a failed `oracle:stage7-authority` result, returns the candidate to revision, and prevents authoritative publication. Fault injection covers abort, retry, crash, atomic publication, stale-head handling, and hidden-input residue. Named tests retain `write-fidelity`, `write-constraint-tax`, `write-source-fallback`, and `write-fault-injection` ([verification research](research/2026-07-24/verification/part-b.md), [welding](research/2026-07-24/lanes/welding.md)).

### Required decisions

The architecture, model and eval, security, and operator owners preregister the Stage 7 authority gate, candidate review authority, retry bounds, source-only degradation, atomic publication, and stale-head behaviour. Decisions link to `unresolved:initial-policy-matrix`, `obligation:source-only-fallback`, and `obligation:central-audience-no-residue`.

### Falsification and stop conditions

Stage 7 returns to revision on junk fill, unstable canonicalisation, unacceptable latency or log growth, critic bypass, partial publication, hidden residue, source loss, stale publication, production/reference divergence, or source-only failure. Authoritative publication remains disabled when `evidence:1c-measurement` or `oracle:stage7-authority` is absent.

### Experimental-state disposition

Shadow logs, proposal stores, projections, snapshots, and candidate policy encodings are disposable. Source fixtures, preregistered thresholds, exact oracle results, failure traces, measurements, and decision rationales are retained.

### Blocking scope

Stage 7 blocks Stage 8 and all initial-policy evidence. The `capability:read-write-vertical-slice` entry is required substrate. Its authoritative mode cannot be replaced by a stage-level selection.

### Deferred

Non-actual inference, autonomous acceptance, broad contradiction, generated episodes, and bulk ingestion remain deferred.

## Stage 8: operational substrate and disabled-worker contract

### Prerequisite node IDs

`stage:7`.

### Evidence prerequisite IDs

`evidence:stage7-vertical-slice`.

### Semantic contracts

Stage 8 builds the minimum event-sourced job substrate. It defines stable job keys, target heads, leases, attempts, outcomes, cancellation, supersession, compare-at-commit, bounded retry, poison state, source lineage, authority classes, drift-canary records, and exception records.

Stage 8 is required substrate even when the genesis-selection-record selects no active background capability. The disabled-worker contract proves zero scheduling and zero execution, preserves pending-state representation, and supplies an additive activation path that cannot reinterpret historical records. A disabled worker does not erase pending work or create a hidden retry path.

Worker policy, maintenance retirement, drift response, exception processing, exploration, proactive initiation, and subagent spawning are separate capability entries. A job-dependent gate names `stage:8` as an operational prerequisite. Stages 9 through 12 use job-free evidence for their initial-policy entries and cannot obtain initial-policy authority from a worker.

### Owning chapters

[Off-turn work](off-turn.md), [verified write](verified-write.md), [confidence](confidence.md), [privacy and provenance](privacy-and-provenance.md), and [coverage](coverage.md).

### Evidence produced

Stage 8 produces `evidence:stage8-operational-substrate` and `evidence:stage8-disabled-worker`. The event-sourced runner and disabled-worker harness cover stable keys, leases, duplicate attempts, crash recovery, cancellation, supersession, compare-at-commit, bounded retry, poison state, source lineage, authority classes, drift records, exceptions, zero scheduling, preserved pending state, and additive activation. The test names include `job-crash-race`, `job-retry-poison`, `job-stale-head`, and `job-authority` ([maintenance](../docs/maintenance-passes.md), [welding](research/2026-07-24/lanes/welding.md), [survey](research/2026-07-24/lanes/survey-giants.md)).

### Required decisions

The operations, architecture, privacy, storage, eval, and operator owners record job identity, leases, retry bounds, poison thresholds, relevant-change keys, compare-at-commit behaviour, cancellation, supersession, authority classes, drift records, exceptions, disabled state, and budget limits. Decisions link to `unresolved:job-semantics`, `obligation:disabled-worker-additive-seam`, and the off-turn obligations.

### Falsification and stop conditions

Stage 8 returns to revision on unbounded retry, duplicate publication, stale commit, authority escalation, lost pending state, hidden scheduling, worker execution while disabled, longitudinal regression, or production/reference divergence. Worker policies do not pass by claiming that a later gate will address a substrate failure.

### Experimental-state disposition

Generated job logs, worker state, projections, snapshots, and candidate policy encodings are disposable. Exact schedules, expected folds, race and crash traces, disabled-state traces, longitudinal measurements, failures, and decision rationales are retained.

### Blocking scope

Stage 8 blocks the genesis freeze and every job-dependent capability or activation gate. It does not block job-free initial-policy evaluation in Stages 9 through 12 after the numbered dependency sequence reaches those stages.

### Deferred

Worker policy, maintenance retirement, drift response, exception processing, exploration, proactive initiation, subagent spawning, and all autonomous off-turn interpretation remain separately gated.

## Stage 9: initial temporal policy

### Prerequisite node IDs

`stage:8`.

### Evidence prerequisite IDs

`evidence:stage2-reference`, `evidence:stage7-vertical-slice`.

### Semantic contracts

Stage 9 evaluates typed-time rendering and correction. It covers uncertainty and timezone rendering, planned, actual, and cancelled semantics, and the explicit separation of Occurrence, agent-authored Task, and Trigger. A dated description cannot arm a Trigger. Initial-policy evidence is synchronous and job-free.

The candidate initial policy may include only the temporal behaviour selected for `capability:initial-temporal-policy`. Rich recurrence, business-calendar adjustment, volatility automation, habitual and deontic inference, qualitative temporal inference, and autonomous recurrence interpretation are separate activation gates.

### Owning chapters

[Time](time.md), [assertions](statements.md#assertion), and [off-turn work](off-turn.md).

### Evidence produced

Stage 9 produces `evidence:stage9-temporal-policy`. Dated-description, temporal-correction, vague-time, timezone, cancellation, and month-end fixtures compare production-shaped folds with `evidence:stage2-reference`. Generated transition tests cover correction and terminal Task and Trigger chains. `time-dated-no-fire`, `time-correction`, `time-vague-timezone`, and `time-recurrence-properties` remain named tests ([time research](research/2026-07-24/lanes/time-memory.md), [confidence](confidence.md#evidence-map)).

### Required decisions

The architecture, temporal-policy, eval, and operator owners record uncertainty rendering, timezone ownership, cancellation representation, initial recurrence scope, and explicit unknown outcomes. Decisions link to `unresolved:initial-policy-matrix`, `unresolved:qualitative-temporal-subalgebra`, and the temporal obligations.

### Falsification and stop conditions

Stage 9 returns to revision if ordinary intent requires guessing, corrections mutate source, descriptions create Triggers, generated transition chains violate the fold, or production-shaped results diverge from the reference. A selected optional recurrence capability requires its own activation gate and does not inherit this pass.

### Experimental-state disposition

Temporal logs, projections, snapshots, and candidate policy encodings are disposable. Exact fixture inputs, expected states, generated counterexamples, measurements, failures, and decision rationales are retained.

### Blocking scope

Stage 9 blocks Stage 10. It blocks `capability:initial-temporal-policy` only when the selection record selects it as `initial_policy`, and it does not block genesis for unselected temporal extensions.

### Deferred

Rich recurrence, business-calendar adjustment, volatility automation, habitual and deontic inference, qualitative temporal inference, and autonomous recurrence interpretation remain deferred.

## Stage 10: initial Event, role, and relation policy

### Prerequisite node IDs

`stage:9`.

### Evidence prerequisite IDs

`evidence:stage2-reference`, `evidence:stage7-vertical-slice`.

### Semantic contracts

Stage 10 evaluates the initial Event, universal-role, relation, partial-projection, co-reference, alias, and historical-definition policies. Distinct Events remain distinct unless an explicit reversible policy permits a relation. Severance restores the source history. Partial projections remain disclosure-safe. Historical definitions replay under the definition version recorded at the time.

The initial-policy evidence is job-free. Broad role vocabularies and autonomous Event merging remain activation gates. Relation evolution does not make a relation definition authoritative without a versioned definition record.

### Owning chapters

[Events and roles](events-and-roles.md), [relations](relations.md), [privacy and provenance](privacy-and-provenance.md), and [confidence](confidence.md).

### Evidence produced

Stage 10 produces `evidence:stage10-event-policy`. Distinct-Event, explicit re-mention, merge and severance, partial-projection, alias-cycle, and historical-definition fixtures compare production-shaped behaviour with the reference folds and audience probes. Tests include `event-distinct`, `event-merge-sever`, `event-partial-projection`, and `schema-historical-replay` ([modelling study](research/2026-08-03/modelling-study.md), [fact-shape report](research/2026-07-24/report.md), [confidence](confidence.md#evidence-map)).

### Required decisions

The architecture and schema owners record Event types, universal role parents, subrole governance, safe-shell policy, Event-resolution authority, relation definitions, alias handling, and historical replay. Decisions link to `unresolved:event-subrole-governance`, `unresolved:event-coreference-policy`, `unresolved:definition-replay-authority`, and the disclosure obligations.

### Falsification and stop conditions

Stage 10 returns to revision on role-tail inconsistency, unsafe partial visibility, hidden conversational schema activation, destructive merge, failed severance restoration, alias cycles, historical-definition drift, or production/reference divergence.

### Experimental-state disposition

Generated Event logs, composites, projections, snapshots, and definition encodings are disposable. Fixture sources, expected source views, audience probes, replay digests, failures, and decision rationales are retained.

### Blocking scope

Stage 10 blocks Stage 11. It blocks `capability:initial-event-role-relation-policy` only when the selection record selects it as `initial_policy`. Optional role vocabularies and autonomous merging remain isolated.

### Deferred

Autonomous Event merging and broad role vocabularies remain deferred.

## Stage 11: initial identity policy

### Prerequisite node IDs

`stage:10`.

### Evidence prerequisite IDs

`evidence:stage2-reference`, `evidence:stage7-vertical-slice`.

### Semantic contracts

Stage 11 evaluates operator-confirmed, disclosure-cleared, disjoint identity composites, one-handle rendering, merge and severance replay, and dependant invalidation. Platform stubs remain permanent. Merge hypotheses record evidence, scope, recall, disclosure clearance, authority, status, and lifecycle. Overlapping candidates do not become one composite.

The initial-policy evidence is job-free. Autonomous identity scoring and cross-instance identity remain activation gates. Attribute overlap is not identity evidence by itself.

### Owning chapters

[Identity](identity.md), [privacy and provenance](privacy-and-provenance.md), [query surface](query-surface.md), and [confidence](confidence.md).

### Evidence produced

Stage 11 produces `evidence:stage11-identity-policy`. Overlap, response-affecting disclosure, one-handle, merge and severance replay, and sibling-history fixtures compare production-shaped resolution with reference folds and audience probes. Tests include `identity-overlap`, `identity-disclosure`, `identity-merge-sever`, and `identity-one-handle` ([identity research](research/2026-07-24/lanes/identity-belief.md), [report](research/2026-07-24/report.md), [confidence](confidence.md#evidence-map)).

### Required decisions

The identity, security, and operator owners record confirmation authority, disjointness handling, recall and disclosure clearance, one-handle rendering, severance response, dependant invalidation, and stub naming. Decisions link to `unresolved:identity-thresholds`, and the identity obligations.

### Falsification and stop conditions

Stage 11 returns to revision if composites overlap, tentative recall affects a response without disclosure clearance, severance loses source history, a dependant survives an invalid environment, a platform stub is treated as temporary, or production-shaped results diverge from the reference.

### Experimental-state disposition

Generated identity logs, composite projections, snapshots, and candidate encodings are disposable. Hypothesis fixtures, exact expected source views, audience probes, replay digests, failures, and decision rationales are retained.

### Blocking scope

Stage 11 blocks Stage 12. It blocks `capability:initial-identity-policy` only when the selection record selects it as `initial_policy`. Autonomous scoring and cross-instance identity have independent gates.

### Deferred

Autonomous scoring and cross-instance identity remain deferred.

## Stage 12: initial support, dependence, and contradiction policy

### Prerequisite node IDs

`stage:11`.

### Evidence prerequisite IDs

`evidence:stage2-reference`, `evidence:stage7-vertical-slice`.

### Semantic contracts

Stage 12 evaluates audience-safe ordinal support projections, dependence suppression, selected reliability observations, expression strength, explicit polarity, and the registered mechanical contradiction subset. Support remains an ordinal corroboration projection. It is not a truth probability.

The initial-policy evidence is job-free. Numeric probability, support fusion, autonomous contradiction arbitration, and general runtime faithfulness checking remain activation gates. Mechanical rules do not claim linguistic contradiction.

### Owning chapters

[Belief](belief.md), [assertions](statements.md#mechanical-contradiction), [privacy and provenance](privacy-and-provenance.md), and [confidence](confidence.md).

### Evidence produced

Stage 12 produces `evidence:stage12-support-policy`. Shared-room, agent-restatement, relay and return, reliability, hidden-support, last-independent-support withdrawal, opposite-polarity, quantity, and contest-versus-contradiction fixtures compare production-shaped projections with the reference oracle. The `support-last-withdrawal` fixture also exercises the selected initial support policy end to end. It publishes an ordinary single-source candidate under an active definition. A policy that makes the source eligible must append settlement through its named runtime authority, return the Assertion through a default settled-only conversational read, append the selected demotion or invalidation transition after the last eligible support is withdrawn, and omit the Assertion from the next default read. A policy that deliberately keeps single-source Assertions candidate-only must omit the Assertion from both default reads and return it only through explicit candidate inspection. After withdrawal, candidate inspection must report an unsupported candidate with no live testimonial support. In both branches, an undisclosable Attestation changes no visible result, transition decision, rank, count, or error shape. Tests also include `support-hidden-zero`, `support-dependence`, and `support-contest` ([belief fixtures](belief.md#deterministic-fixtures), [identity and belief research](research/2026-07-24/lanes/identity-belief.md), [confidence](confidence.md#evidence-map)).

### Required decisions

The belief, privacy, architecture, eval, and operator owners record ordinal vocabulary, dependence rules, reliability use, promotion criteria, contradiction registry, no-rank alternative, and the zero-tolerance privacy oracle. The selected `capability:initial-support-policy` names the runtime settlement actor and authority. It states whether an ordinary candidate with one audience-visible live support lineage is eligible for settlement or deliberately remains candidate-only. It also states the explicit demotion, invalidation, or continued-candidate result after withdrawal of the last eligible support. These decisions do not select numeric support or fusion. Decisions link to `unresolved:support-arithmetic-fusion`, `unresolved:initial-policy-matrix`, and the support obligations.

### Falsification and stop conditions

Stage 12 returns to revision if hidden support changes visible output, dependent evidence increases support, mechanical rules claim linguistic contradiction, last-support withdrawal violates the expected fold, support becomes a truth probability, or production/reference comparison fails.

### Experimental-state disposition

Generated support logs, projections, snapshots, and candidate policy encodings are disposable. Source fixtures, audience probes, exact expected ordinals and classifications, failures, measurements, and decision rationales are retained. Unranked visible Attestations remain the final fail-closed fallback.

### Blocking scope

Stage 12 blocks the genesis freeze. It blocks `capability:initial-support-policy` only when the selection record selects it as `initial_policy`. Optional fusion and arbitration do not block genesis when disabled.

### Deferred

Fusion operators, numeric probabilities, autonomous contradiction arbitration, and general runtime faithfulness checking remain deferred.

## Capability census and allowlist model

Stage 0 defines and versions a finite programme-status marker inventory before the census runs. The inventory includes the literal terms `gated`, `gate`, `optional`, `deferred`, `safely deferred`, `disabled`, `inactive`, `remains off`, `not enabled`, `later activation`, `post-genesis`, and `open experiment`. It also includes explicit constructions of the form `until <condition>` and `only after <condition>` when they govern capability activation.

The census covers every normative occurrence outside `docs-future/research/` that describes a capability with a programme-status marker. The machine scan and the manual semantic pass both exclude all paths below `docs-future/research/`. The manual pass reads every status sentence and records whether it identifies a capability, identifies a non-capability statement, or requires a new capability ID. No unmatched occurrence is permitted.

Each census row has these fields: `occurrence_id`, `source_path`, `source_heading`, `stable_quoted_excerpt_or_digest`, `matched_marker_or_rule_id`, `canonical_capability_id_or_non_capability_allowlist_id`, `status`, `owner_id`, and `activation_gate_id`. Each allowlist row uses the same occurrence identity and records `allowlist_id`, `rationale`, and the reviewer decision. A non-capability allowlist entry does not hide a capability. It records why the matched words describe a safety property, an evidence condition, a historical statement, or another non-activation concept.

The canonical capability census is:

| capability_id | status | owner_id | activation_gate_id |
|---|---|---|---|
| `capability:source-activity-substrate` | `required_substrate` | `owner:architecture` | none |
| `capability:assertion-definition-substrate` | `required_substrate` | `owner:schema` | none |
| `capability:audience-influence-substrate` | `required_substrate` | `owner:security` | none |
| `capability:artefact-erasure-recovery-substrate` | `required_substrate` | `owner:storage` | none |
| `capability:read-write-vertical-slice` | `required_substrate` | `owner:architecture` | none |
| `capability:operational-job-substrate` | `required_substrate` | `owner:operations` | none |
| `capability:disabled-worker-contract` | `required_substrate` | `owner:operations` | none |
| `capability:inter-agent-status-freshness` | `required_substrate` | `owner:connector` | none |
| `capability:initial-temporal-policy` | `initial_policy` | `owner:temporal-policy` | none |
| `capability:initial-event-role-relation-policy` | `initial_policy` | `owner:schema` | none |
| `capability:initial-identity-policy` | `initial_policy` | `owner:identity` | none |
| `capability:initial-support-policy` | `initial_policy` | `owner:belief` | none |
| `capability:generated-episodes` | `activation_gate` | `owner:model-eval` | `gate:generated-episodes` |
| `capability:bulk-ingestion` | `activation_gate` | `owner:ingestion` | `gate:bulk-ingestion` |
| `capability:procedural-memory` | `activation_gate` | `owner:ingestion` | `gate:procedural-memory` |
| `capability:working-memory` | `activation_gate` | `owner:ingestion` | `gate:working-memory` |
| `capability:historical-reinspection` | `activation_gate` | `owner:model-eval` | `gate:historical-reinspection` |
| `capability:ocr` | `activation_gate` | `owner:model-eval` | `gate:ocr` |
| `capability:generated-captions` | `activation_gate` | `owner:model-eval` | `gate:generated-captions` |
| `capability:region-grounding` | `activation_gate` | `owner:model-eval` | `gate:region-grounding` |
| `capability:visual-retrieval` | `activation_gate` | `owner:model-eval` | `gate:visual-retrieval` |
| `capability:scene-graph-writer` | `activation_gate` | `owner:model-eval` | `gate:scene-graph-writer` |
| `capability:rich-recurrence` | `activation_gate` | `owner:temporal-policy` | `gate:rich-recurrence` |
| `capability:business-calendar-adjustment` | `activation_gate` | `owner:temporal-policy` | `gate:business-calendar-adjustment` |
| `capability:volatility-automation` | `activation_gate` | `owner:temporal-policy` | `gate:volatility-automation` |
| `capability:habitual-deontic-inference` | `activation_gate` | `owner:temporal-policy` | `gate:habitual-deontic-inference` |
| `capability:qualitative-temporal-inference` | `activation_gate` | `owner:temporal-policy` | `gate:qualitative-temporal-inference` |
| `capability:autonomous-recurrence-interpretation` | `activation_gate` | `owner:temporal-policy` | `gate:autonomous-recurrence-interpretation` |
| `capability:broad-event-role-vocabulary` | `activation_gate` | `owner:schema` | `gate:broad-event-role-vocabulary` |
| `capability:autonomous-event-merging` | `activation_gate` | `owner:schema` | `gate:autonomous-event-merging` |
| `capability:numeric-support` | `activation_gate` | `owner:belief` | `gate:numeric-support` |
| `capability:support-fusion` | `activation_gate` | `owner:belief` | `gate:support-fusion` |
| `capability:autonomous-contradiction-arbitration` | `activation_gate` | `owner:belief` | `gate:autonomous-contradiction-arbitration` |
| `capability:autonomous-identity` | `activation_gate` | `owner:identity` | `gate:autonomous-identity` |
| `capability:cross-instance-identity` | `activation_gate` | `owner:identity` | `gate:cross-instance-identity` |
| `capability:transmission-reciprocity` | `activation_gate` | `owner:privacy-policy` | `gate:transmission-reciprocity` |
| `capability:transmission-purpose-limitation` | `activation_gate` | `owner:privacy-policy` | `gate:transmission-purpose-limitation` |
| `capability:remote-actionable-status` | `activation_gate` | `owner:connector` | `gate:remote-actionable-status` |
| `capability:general-runtime-faithfulness-checking` | `activation_gate` | `owner:model-eval` | `gate:general-runtime-faithfulness-checking` |
| `capability:worker-policy` | `activation_gate` | `owner:operations` | `gate:worker-policy` |
| `capability:maintenance-retirement` | `activation_gate` | `owner:operations` | `gate:maintenance-retirement` |
| `capability:drift-response` | `activation_gate` | `owner:operations` | `gate:drift-response` |
| `capability:exception-processing` | `activation_gate` | `owner:operations` | `gate:exception-processing` |
| `capability:exploration` | `activation_gate` | `owner:operations` | `gate:exploration` |
| `capability:proactive-initiation` | `activation_gate` | `owner:operations` | `gate:proactive-initiation` |
| `capability:subagent-spawning` | `activation_gate` | `owner:operations` | `gate:subagent-spawning` |

The census classifies worker policy, maintenance retirement, drift response, and exception processing explicitly. It classifies the minimum job machinery as `capability:operational-job-substrate` and `capability:disabled-worker-contract`. It does not leave operational status wording outside the register.

A grouped gate lists every subcapability ID and records independent pass, fail, decline, and activation decisions. The grouped gate cannot use one pass to activate a different subcapability.

### Non-capability allowlist

The finite scan records non-capability matches explicitly. These allowlist IDs are classifications, not exclusions from the scan.

| allowlist_id | rationale |
|---|---|
| `non_capability_allowlist:ordinary-optional-schema` | The marker describes an optional field, argument, bound, or schema member, not an activatable capability. |
| `non_capability_allowlist:lifecycle-precondition` | The condition orders a lifecycle, publication, restore, transition, or commit; it does not gate a capability. |
| `non_capability_allowlist:evidence-status-vocabulary` | The marker describes evidence, a threshold, a fixture, a review status, or an oracle rather than capability activation. |
| `non_capability_allowlist:historical-claim` | The marker records a historical, dated, current-system, or comparative claim rather than current programme status. |
| `non_capability_allowlist:per-result-disablement` | The marker applies to an individual result, projection, lane, or record, not to a selectable capability. |
| `non_capability_allowlist:safety-condition` | The marker describes an authority, privacy, failure, fallback, or safety condition rather than capability activation. |
| `non_capability_allowlist:programme-taxonomy-language` | The marker defines programme vocabulary or a generic gate/status concept without naming a capability. |
| `non_capability_allowlist:lifecycle-state-vocabulary` | The marker names an object state or state-machine vocabulary, not programme activation. |

### Occurrence census

The following table records the finite scan over every `docs-future/*.md` file except `docs-future/evolution.md` and every path below `docs-future/research/`. The scan uses the word-bounded marker alternatives `safely deferred`, `not enabled`, `later activation`, `post-genesis`, `open experiment`, `remains off`, `optional`, `deferred`, `disabled`, `inactive`, `gated`, and `gate`, plus the bare rules `until` and `only after`. It records one row per matching source line. The digest covers the source path, nearest source heading, and exact stripped line, so the identity does not depend on a line number.

The committed table is a hand-built snapshot of that scan, retained as the worked example of the census contract. The `stage:0` census executable regenerates it, and the regenerated output supersedes this snapshot. A digest goes stale whenever its source line changes; staleness in this snapshot is expected before `stage:0` runs and does not fail the audit.

| occurrence_id | source_path | source_heading | stable_quoted_excerpt_or_digest | matched_marker_or_rule_id | canonical_capability_id_or_non_capability_allowlist_id | status | owner_id | activation_gate_id | allowlist rationale |
|---|---|---|---|---|---|---|---|---|---|
| `occurrence:readme-scope-and-permanence-7ff589cb813b15b3` | `docs-future/README.md` | Scope and permanence | `sha256:7ff589cb813b15b3` | marker:post-genesis; marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | The marker records a historical, dated, current-system, or comparative claim rather than current programme status. |
| `occurrence:readme-scope-and-permanence-d968a4157c6a55ea` | `docs-future/README.md` | Scope and permanence | `sha256:d968a4157c6a55ea` | marker:gate; rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | The condition orders a lifecycle, publication, restore, transition, or commit; it does not gate a capability. |
| `occurrence:readme-canonical-glossary-71d8d1a5099eda1e` | `docs-future/README.md` | Canonical glossary | `sha256:71d8d1a5099eda1e` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | The marker describes an optional field, argument, bound, or schema member, not an activatable capability. |
| `occurrence:artefacts-and-perceptions-source-selectors-dc9731fd4c20e4ab` | `docs-future/artefacts-and-perceptions.md` | Source selectors | `sha256:dc9731fd4c20e4ab` | marker:later-activation | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | The marker records a historical, dated, current-system, or comparative claim rather than current programme status. |
| `occurrence:artefacts-and-perceptions-activation-gate-capabilities-217cec315821d03f` | `docs-future/artefacts-and-perceptions.md` | Activation-gate capabilities | `sha256:217cec315821d03f` | marker:gate | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | The marker defines programme vocabulary or a generic gate/status concept without naming a capability. |
| `occurrence:artefacts-and-perceptions-activation-gate-capabilities-a2d731f3f6815ac8` | `docs-future/artefacts-and-perceptions.md` | Activation-gate capabilities | `sha256:a2d731f3f6815ac8` | marker:gate; marker:disabled | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | The marker describes evidence, a threshold, a fixture, a review status, or an oracle rather than capability activation. |
| `occurrence:artefacts-and-perceptions-activation-gate-capabilities-f9bc6d47ffad0aee` | `docs-future/artefacts-and-perceptions.md` | Activation-gate capabilities | `sha256:f9bc6d47ffad0aee` | marker:gate; marker:disabled | `capability:scene-graph-writer` | activation_gate | `owner:model-eval` | gate:scene-graph-writer | — |
| `occurrence:artefacts-and-perceptions-multimodal-fixtures-d7a8fa20e42c2a2b` | `docs-future/artefacts-and-perceptions.md` | Multimodal fixtures | `sha256:d7a8fa20e42c2a2b` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | The marker records a historical, dated, current-system, or comparative claim rather than current programme status. |
| `occurrence:belief-belief-891ec52df8892dab` | `docs-future/belief.md` | Belief | `sha256:891ec52df8892dab` | marker:gated; rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | The condition orders a lifecycle, publication, restore, transition, or commit; it does not gate a capability. |
| `occurrence:belief-contest-and-contradiction-f4c060abefa66bfb` | `docs-future/belief.md` | Contest and contradiction | `sha256:f4c060abefa66bfb` | marker:gated | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | The marker defines programme vocabulary or a generic gate/status concept without naming a capability. |
| `occurrence:belief-deterministic-fixtures-a312fa836217d933` | `docs-future/belief.md` | Deterministic fixtures | `sha256:a312fa836217d933` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | The condition orders a lifecycle, publication, restore, transition, or commit; it does not gate a capability. |
| `occurrence:belief-deterministic-fixtures-76c50fc26b78cf51` | `docs-future/belief.md` | Deterministic fixtures | `sha256:76c50fc26b78cf51` | rule:until-condition; marker:gate; marker:disabled | `capability:support-fusion` | activation_gate | `owner:belief` | gate:support-fusion | — |
| `occurrence:confidence-status-vocabulary-fa00e76c33bf5b37` | `docs-future/confidence.md` | Status vocabulary | `sha256:fa00e76c33bf5b37` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:confidence-status-vocabulary-52b3b212e9d6986e` | `docs-future/confidence.md` | Status vocabulary | `sha256:52b3b212e9d6986e` | marker:open-experiment | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-status-vocabulary-e06083746738141b` | `docs-future/confidence.md` | Status vocabulary | `sha256:e06083746738141b` | marker:safely-deferred | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-status-vocabulary-922f01d218260829` | `docs-future/confidence.md` | Status vocabulary | `sha256:922f01d218260829` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:confidence-status-vocabulary-f23c94b9c2c3759a` | `docs-future/confidence.md` | Status vocabulary | `sha256:f23c94b9c2c3759a` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:confidence-status-vocabulary-7fff0dd2df1b6ef7` | `docs-future/confidence.md` | Status vocabulary | `sha256:7fff0dd2df1b6ef7` | marker:gated;marker:disabled;rule:until-condition;marker:gate | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:confidence-status-vocabulary-3c75929b606255c9` | `docs-future/confidence.md` | Status vocabulary | `sha256:3c75929b606255c9` | marker:safely-deferred;marker:disabled | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-status-vocabulary-34d72787dd628504` | `docs-future/confidence.md` | Status vocabulary | `sha256:34d72787dd628504` | marker:open-experiment;marker:safely-deferred;rule:until-condition;marker:gate | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:confidence-status-vocabulary-4d0782d5bb21a757` | `docs-future/confidence.md` | Status vocabulary | `sha256:4d0782d5bb21a757` | marker:gate;marker:disabled | `non_capability_allowlist:lifecycle-state-vocabulary` | non_capability | `owner:architecture` | none | object lifecycle state, not programme activation |
| `occurrence:confidence-belief-f36e3aa203c2a28d` | `docs-future/confidence.md` | Belief | `sha256:f36e3aa203c2a28d` | marker:safely-deferred | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:confidence-unresolved-item-register-75c73cfb8512dc3b` | `docs-future/confidence.md` | Unresolved-item register | `sha256:75c73cfb8512dc3b` | marker:disabled;rule:until-condition;marker:gate;marker:safely-deferred | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:confidence-unresolved-item-register-9d66a0c8319070d5` | `docs-future/confidence.md` | Unresolved-item register | `sha256:9d66a0c8319070d5` | marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-unresolved-item-register-0eb7d8944f453802` | `docs-future/confidence.md` | Unresolved-item register | `sha256:0eb7d8944f453802` | marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-unresolved-item-register-5072fc82375eecb9` | `docs-future/confidence.md` | Unresolved-item register | `sha256:5072fc82375eecb9` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:confidence-unresolved-item-register-4d6edc0f85db9407` | `docs-future/confidence.md` | Unresolved-item register | `sha256:4d6edc0f85db9407` | marker:gate | `capability:bulk-ingestion` | activation_gate | `owner:ingestion` | gate:bulk-ingestion | — |
| `occurrence:confidence-unresolved-item-register-1cb9caaa1037367a` | `docs-future/confidence.md` | Unresolved-item register | `sha256:1cb9caaa1037367a` | marker:safely-deferred | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-unresolved-item-register-ac01e1bb7abacf32` | `docs-future/confidence.md` | Unresolved-item register | `sha256:ac01e1bb7abacf32` | marker:gate | `capability:autonomous-identity` | activation_gate | `owner:identity` | gate:autonomous-identity | — |
| `occurrence:confidence-unresolved-item-register-baef11612f3ef131` | `docs-future/confidence.md` | Unresolved-item register | `sha256:baef11612f3ef131` | marker:safely-deferred;marker:gate | `capability:support-fusion` | activation_gate | `owner:belief` | gate:support-fusion | — |
| `occurrence:confidence-unresolved-item-register-e27994080ea392b1` | `docs-future/confidence.md` | Unresolved-item register | `sha256:e27994080ea392b1` | marker:safely-deferred;marker:gate | `capability:proactive-initiation` | activation_gate | `owner:operations` | gate:proactive-initiation | — |
| `occurrence:confidence-unresolved-item-register-aa96bc4c0900b90f` | `docs-future/confidence.md` | Unresolved-item register | `sha256:aa96bc4c0900b90f` | marker:safely-deferred;marker:gate | `capability:exploration` | activation_gate | `owner:operations` | gate:exploration | — |
| `occurrence:confidence-unresolved-item-register-28822f9446017412` | `docs-future/confidence.md` | Unresolved-item register | `sha256:28822f9446017412` | marker:safely-deferred;marker:gate | `capability:habitual-deontic-inference` | activation_gate | `owner:temporal-policy` | gate:habitual-deontic-inference | — |
| `occurrence:confidence-unresolved-item-register-ac9c385ec0990d25` | `docs-future/confidence.md` | Unresolved-item register | `sha256:ac9c385ec0990d25` | marker:gate | `capability:broad-event-role-vocabulary` | activation_gate | `owner:schema` | gate:broad-event-role-vocabulary | — |
| `occurrence:confidence-unresolved-item-register-75cfb53c5d5150d8` | `docs-future/confidence.md` | Unresolved-item register | `sha256:75cfb53c5d5150d8` | marker:gate | `capability:autonomous-event-merging` | activation_gate | `owner:schema` | gate:autonomous-event-merging | — |
| `occurrence:confidence-unresolved-item-register-5ebd5e1da0a222ec` | `docs-future/confidence.md` | Unresolved-item register | `sha256:5ebd5e1da0a222ec` | marker:gate | `capability:rich-recurrence` | activation_gate | `owner:temporal-policy` | gate:rich-recurrence | — |
| `occurrence:confidence-unresolved-item-register-52b3727f7e1172b2` | `docs-future/confidence.md` | Unresolved-item register | `sha256:52b3727f7e1172b2` | marker:safely-deferred;marker:gate | `capability:historical-reinspection` | activation_gate | `owner:model-eval` | gate:historical-reinspection | — |
| `occurrence:confidence-unresolved-item-register-0fc766e96b0250f6` | `docs-future/confidence.md` | Unresolved-item register | `sha256:0fc766e96b0250f6` | marker:safely-deferred;marker:gate | `capability:ocr; capability:generated-captions; capability:region-grounding; capability:visual-retrieval; capability:scene-graph-writer` | activation_gate | `owner:model-eval` | gate:ocr; gate:generated-captions; gate:region-grounding; gate:visual-retrieval; gate:scene-graph-writer | — |
| `occurrence:confidence-unresolved-item-register-f99e87d111ca710e` | `docs-future/confidence.md` | Unresolved-item register | `sha256:f99e87d111ca710e` | marker:gate | `capability:proactive-initiation` | activation_gate | `owner:operations` | gate:proactive-initiation | — |
| `occurrence:confidence-unresolved-item-register-1f19ede864dd9673` | `docs-future/confidence.md` | Unresolved-item register | `sha256:1f19ede864dd9673` | marker:safely-deferred;marker:gate | `capability:general-runtime-faithfulness-checking` | activation_gate | `owner:model-eval` | gate:general-runtime-faithfulness-checking | — |
| `occurrence:confidence-unresolved-item-register-2c62943d46968358` | `docs-future/confidence.md` | Unresolved-item register | `sha256:2c62943d46968358` | marker:safely-deferred | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-unresolved-item-register-b826a5600afac451` | `docs-future/confidence.md` | Unresolved-item register | `sha256:b826a5600afac451` | marker:gate | `capability:qualitative-temporal-inference` | activation_gate | `owner:temporal-policy` | gate:qualitative-temporal-inference | — |
| `occurrence:confidence-unresolved-item-register-5816c5300fc46901` | `docs-future/confidence.md` | Unresolved-item register | `sha256:5816c5300fc46901` | marker:gate | `capability:remote-actionable-status; capability:inter-agent-status-freshness` | activation_gate | `owner:connector` | gate:remote-actionable-status | — |
| `occurrence:confidence-unresolved-item-register-8794ff4901eb9713` | `docs-future/confidence.md` | Unresolved-item register | `sha256:8794ff4901eb9713` | marker:disabled | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-adversarial-obligation-register-f673d76e7e4c425e` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:f673d76e7e4c425e` | marker:gate;marker:gated | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-adversarial-obligation-register-29df6bd84f64a63e` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:29df6bd84f64a63e` | marker:gated;marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-adversarial-obligation-register-6e81ab818331c1bb` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:6e81ab818331c1bb` | marker:disabled;marker:safely-deferred;marker:gate | `capability:transmission-reciprocity; capability:transmission-purpose-limitation` | activation_gate | `owner:privacy-policy` | gate:transmission-reciprocity; gate:transmission-purpose-limitation | — |
| `occurrence:confidence-adversarial-obligation-register-f504ba4da5891b2d` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:f504ba4da5891b2d` | marker:gated | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-adversarial-obligation-register-298ff0d77b17a42b` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:298ff0d77b17a42b` | marker:safely-deferred;marker:disabled;rule:until-condition | `capability:historical-reinspection; capability:ocr; capability:generated-captions; capability:region-grounding; capability:visual-retrieval; capability:scene-graph-writer` | activation_gate | `owner:model-eval` | gate:historical-reinspection; gate:ocr; gate:generated-captions; gate:region-grounding; gate:visual-retrieval; gate:scene-graph-writer | — |
| `occurrence:confidence-adversarial-obligation-register-7bef8c877c10e18b` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:7bef8c877c10e18b` | marker:optional | `capability:read-write-vertical-slice` | optional | `owner:architecture` | none | — |
| `occurrence:confidence-adversarial-obligation-register-6973896c968ee6bb` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:6973896c968ee6bb` | rule:only-after-condition | `capability:artefact-erasure-recovery-substrate` | programme_status | `owner:storage` | none | — |
| `occurrence:confidence-adversarial-obligation-register-9ef855ae76949c3d` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:9ef855ae76949c3d` | rule:until-condition | `capability:artefact-erasure-recovery-substrate` | programme_status | `owner:storage` | none | — |
| `occurrence:confidence-adversarial-obligation-register-8b8be815faa03312` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:8b8be815faa03312` | marker:disabled;marker:later-activation | `capability:disabled-worker-contract` | disabled | `owner:operations` | none | — |
| `occurrence:confidence-adversarial-obligation-register-b7660ed28bc705be` | `docs-future/confidence.md` | Adversarial obligation register | `sha256:b7660ed28bc705be` | marker:optional | `capability:source-activity-substrate` | optional | `owner:architecture` | none | — |
| `occurrence:confidence-candidate-authoritative-ledger-architect-6e6ab73a077046f7` | `docs-future/confidence.md` | Candidate authoritative-ledger architecture and recovery authority | `sha256:6e6ab73a077046f7` | rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:confidence-candidate-authoritative-ledger-architect-8d63fe80e00d9cb1` | `docs-future/confidence.md` | Candidate authoritative-ledger architecture and recovery authority | `sha256:8d63fe80e00d9cb1` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:confidence-successor-seed-definitions-c1b3a0b760d6a06d` | `docs-future/confidence.md` | Successor seed definitions | `sha256:c1b3a0b760d6a06d` | marker:disabled | `non_capability_allowlist:lifecycle-state-vocabulary` | non_capability | `owner:architecture` | none | object lifecycle state, not programme activation |
| `occurrence:confidence-successor-seed-definitions-d2de895cb9b91b8e` | `docs-future/confidence.md` | Successor seed definitions | `sha256:d2de895cb9b91b8e` | marker:inactive | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-inter-agent-freshness-versus-remote-acti-9862a1c67fbac8f4` | `docs-future/confidence.md` | Inter-agent freshness versus remote activation | `sha256:9862a1c67fbac8f4` | marker:gate;rule:only-after-condition;marker:disabled;marker:later-activation | `capability:remote-actionable-status` | activation_gate | `owner:connector` | gate:remote-actionable-status | — |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-23874269c93368fc` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:23874269c93368fc` | marker:gate | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | programme vocabulary without capability subject |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-ede5432e32e26228` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:ede5432e32e26228` | marker:disabled | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-2320404f71fc1fb7` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:2320404f71fc1fb7` | marker:disabled;marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-159c95f776ad58bb` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:159c95f776ad58bb` | marker:gate | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | programme vocabulary without capability subject |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-7844e168ef7f5e1b` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:7844e168ef7f5e1b` | marker:gate | `capability:read-write-vertical-slice` | activation_gate | `owner:architecture` | none | — |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-b431d11a05a7efd7` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:b431d11a05a7efd7` | marker:gate | `capability:rich-recurrence; capability:qualitative-temporal-inference; capability:habitual-deontic-inference` | activation_gate | `owner:temporal-policy` | gate:rich-recurrence; gate:qualitative-temporal-inference; gate:habitual-deontic-inference | — |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-629eadef46825b61` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:629eadef46825b61` | marker:gate | `capability:broad-event-role-vocabulary; capability:autonomous-event-merging; capability:autonomous-identity` | activation_gate | `owner:schema` | gate:broad-event-role-vocabulary; gate:autonomous-event-merging; gate:autonomous-identity | — |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-f7e74e83a9b6acfa` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:f7e74e83a9b6acfa` | marker:gate | `capability:support-fusion; capability:remote-actionable-status` | activation_gate | `owner:belief` | gate:support-fusion; gate:remote-actionable-status | — |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-4a8b34ea32c7e73b` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:4a8b34ea32c7e73b` | marker:gate;marker:disabled | `capability:bulk-ingestion; capability:historical-reinspection; capability:ocr; capability:generated-captions; capability:region-grounding; capability:visual-retrieval; capability:scene-graph-writer` | activation_gate | `owner:ingestion` | gate:bulk-ingestion; gate:historical-reinspection; gate:ocr; gate:generated-captions; gate:region-grounding; gate:visual-retrieval; gate:scene-graph-writer | — |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-9eb13df0a6bf1016` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:9eb13df0a6bf1016` | marker:gate | `capability:exploration; capability:proactive-initiation; capability:general-runtime-faithfulness-checking` | activation_gate | `owner:operations` | gate:exploration; gate:proactive-initiation; gate:general-runtime-faithfulness-checking | — |
| `occurrence:confidence-initial-policy-matrix-and-selection-reco-bba64be1937ef481` | `docs-future/confidence.md` | Initial-policy matrix and selection record | `sha256:bba64be1937ef481` | marker:disabled;marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-ids-that-evolution-md-must-declare-f5366042a5b00b4a` | `docs-future/confidence.md` | IDs that `evolution.md` must declare | `sha256:f5366042a5b00b4a` | marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-ids-that-evolution-md-must-declare-b1d5a3ca0953fdb4` | `docs-future/confidence.md` | IDs that `evolution.md` must declare | `sha256:b1d5a3ca0953fdb4` | marker:disabled | `capability:source-activity-substrate; capability:assertion-definition-substrate; capability:audience-influence-substrate; capability:artefact-erasure-recovery-substrate; capability:read-write-vertical-slice; capability:operational-job-substrate; capability:disabled-worker-contract; capability:inter-agent-status-freshness; capability:initial-temporal-policy; capability:initial-event-role-relation-policy; capability:initial-identity-policy; capability:initial-support-policy; capability:generated-episodes; capability:bulk-ingestion; capability:procedural-memory; capability:working-memory; capability:historical-reinspection; capability:ocr; capability:generated-captions; capability:region-grounding; capability:visual-retrieval; capability:scene-graph-writer; capability:rich-recurrence; capability:business-calendar-adjustment; capability:volatility-automation; capability:habitual-deontic-inference; capability:qualitative-temporal-inference; capability:autonomous-recurrence-interpretation; capability:broad-event-role-vocabulary; capability:autonomous-event-merging; capability:numeric-support; capability:support-fusion; capability:autonomous-contradiction-arbitration; capability:autonomous-identity; capability:cross-instance-identity; capability:transmission-reciprocity; capability:transmission-purpose-limitation; capability:remote-actionable-status; capability:general-runtime-faithfulness-checking; capability:worker-policy; capability:maintenance-retirement; capability:drift-response; capability:exception-processing; capability:exploration; capability:proactive-initiation; capability:subagent-spawning` | disabled | `owner:architecture` | gate:generated-episodes; gate:bulk-ingestion; gate:procedural-memory; gate:working-memory; gate:historical-reinspection; gate:ocr; gate:generated-captions; gate:region-grounding; gate:visual-retrieval; gate:scene-graph-writer; gate:rich-recurrence; gate:business-calendar-adjustment; gate:volatility-automation; gate:habitual-deontic-inference; gate:qualitative-temporal-inference; gate:autonomous-recurrence-interpretation; gate:broad-event-role-vocabulary; gate:autonomous-event-merging; gate:numeric-support; gate:support-fusion; gate:autonomous-contradiction-arbitration; gate:autonomous-identity; gate:cross-instance-identity; gate:transmission-reciprocity; gate:transmission-purpose-limitation; gate:remote-actionable-status; gate:general-runtime-faithfulness-checking; gate:worker-policy; gate:maintenance-retirement; gate:drift-response; gate:exception-processing; gate:exploration; gate:proactive-initiation; gate:subagent-spawning | — |
| `occurrence:confidence-ids-that-evolution-md-must-declare-26a0003f6ba5ab57` | `docs-future/confidence.md` | IDs that `evolution.md` must declare | `sha256:26a0003f6ba5ab57` | marker:gate | `capability:generated-episodes; capability:bulk-ingestion; capability:procedural-memory; capability:working-memory; capability:historical-reinspection; capability:ocr; capability:generated-captions; capability:region-grounding; capability:visual-retrieval; capability:scene-graph-writer; capability:rich-recurrence; capability:business-calendar-adjustment; capability:volatility-automation; capability:habitual-deontic-inference; capability:qualitative-temporal-inference; capability:autonomous-recurrence-interpretation; capability:broad-event-role-vocabulary; capability:autonomous-event-merging; capability:numeric-support; capability:support-fusion; capability:autonomous-contradiction-arbitration; capability:autonomous-identity; capability:cross-instance-identity; capability:transmission-reciprocity; capability:transmission-purpose-limitation; capability:remote-actionable-status; capability:general-runtime-faithfulness-checking; capability:worker-policy; capability:maintenance-retirement; capability:drift-response; capability:exception-processing; capability:exploration; capability:proactive-initiation; capability:subagent-spawning` | activation_gate | `owner:model-eval` | gate:generated-episodes; gate:bulk-ingestion; gate:procedural-memory; gate:working-memory; gate:historical-reinspection; gate:ocr; gate:generated-captions; gate:region-grounding; gate:visual-retrieval; gate:scene-graph-writer; gate:rich-recurrence; gate:business-calendar-adjustment; gate:volatility-automation; gate:habitual-deontic-inference; gate:qualitative-temporal-inference; gate:autonomous-recurrence-interpretation; gate:broad-event-role-vocabulary; gate:autonomous-event-merging; gate:numeric-support; gate:support-fusion; gate:autonomous-contradiction-arbitration; gate:autonomous-identity; gate:cross-instance-identity; gate:transmission-reciprocity; gate:transmission-purpose-limitation; gate:remote-actionable-status; gate:general-runtime-faithfulness-checking; gate:worker-policy; gate:maintenance-retirement; gate:drift-response; gate:exception-processing; gate:exploration; gate:proactive-initiation; gate:subagent-spawning | — |
| `occurrence:confidence-ids-that-evolution-md-must-declare-1076ce18f71b324c` | `docs-future/confidence.md` | IDs that `evolution.md` must declare | `sha256:1076ce18f71b324c` | marker:disabled | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:confidence-ids-that-evolution-md-must-declare-f511bcd81d5246f2` | `docs-future/confidence.md` | IDs that `evolution.md` must declare | `sha256:f511bcd81d5246f2` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:confidence-evidence-map-f787764d29c0caa0` | `docs-future/confidence.md` | Evidence map | `sha256:f787764d29c0caa0` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:confidence-evidence-map-8fc2849b32834555` | `docs-future/confidence.md` | Evidence map | `sha256:8fc2849b32834555` | marker:post-genesis | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:confidence-evidence-map-7d839886d9487766` | `docs-future/confidence.md` | Evidence map | `sha256:7d839886d9487766` | marker:optional;marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:confidence-evidence-map-813ac6a48a620dec` | `docs-future/confidence.md` | Evidence map | `sha256:813ac6a48a620dec` | marker:gated;marker:gate | `capability:bulk-ingestion` | activation_gate | `owner:ingestion` | gate:bulk-ingestion | — |
| `occurrence:confidence-evidence-map-840bf8e35f5d2ac7` | `docs-future/confidence.md` | Evidence map | `sha256:840bf8e35f5d2ac7` | marker:gated | `capability:visual-retrieval; capability:historical-reinspection` | activation_gate | `owner:model-eval` | gate:visual-retrieval; gate:historical-reinspection | — |
| `occurrence:coverage-ownership-vocabulary-53120255f3d7690e` | `docs-future/coverage.md` | Ownership vocabulary | `sha256:53120255f3d7690e` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:coverage-ownership-vocabulary-a1850a0df1de7a07` | `docs-future/coverage.md` | Ownership vocabulary | `sha256:a1850a0df1de7a07` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-the-eleven-classes-2042e60d291811b1` | `docs-future/coverage.md` | The eleven classes | `sha256:2042e60d291811b1` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-the-eleven-classes-c17f7b6cd0c8be7c` | `docs-future/coverage.md` | The eleven classes | `sha256:c17f7b6cd0c8be7c` | marker:disabled;rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:coverage-the-eleven-classes-666078f93f1df174` | `docs-future/coverage.md` | The eleven classes | `sha256:666078f93f1df174` | marker:gate;marker:disabled | `capability:autonomous-identity` | activation_gate | `owner:identity` | gate:autonomous-identity | — |
| `occurrence:coverage-the-eleven-classes-7aee883e7e2885b9` | `docs-future/coverage.md` | The eleven classes | `sha256:7aee883e7e2885b9` | marker:gate | `capability:support-fusion` | activation_gate | `owner:belief` | gate:support-fusion | — |
| `occurrence:coverage-the-eleven-classes-172ca925595b5eb7` | `docs-future/coverage.md` | The eleven classes | `sha256:172ca925595b5eb7` | marker:gate | `capability:general-runtime-faithfulness-checking` | activation_gate | `owner:model-eval` | gate:general-runtime-faithfulness-checking | — |
| `occurrence:coverage-directly-addressed-3eb8d36746e73f47` | `docs-future/coverage.md` | Directly addressed | `sha256:3eb8d36746e73f47` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-directly-addressed-c388b608b80b0837` | `docs-future/coverage.md` | Directly addressed | `sha256:c388b608b80b0837` | marker:gate;marker:optional;marker:gated | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:coverage-directly-addressed-613d59d14751edb3` | `docs-future/coverage.md` | Directly addressed | `sha256:613d59d14751edb3` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:coverage-directly-addressed-24c494cd6ab05834` | `docs-future/coverage.md` | Directly addressed | `sha256:24c494cd6ab05834` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:coverage-directly-addressed-b871b461c44aba0a` | `docs-future/coverage.md` | Directly addressed | `sha256:b871b461c44aba0a` | marker:gate;marker:gated | `capability:volatility-automation` | activation_gate | `owner:temporal-policy` | gate:volatility-automation | — |
| `occurrence:coverage-directly-addressed-3c4e7e3f1592fd7b` | `docs-future/coverage.md` | Directly addressed | `sha256:3c4e7e3f1592fd7b` | marker:gate;marker:optional | `capability:bulk-ingestion` | activation_gate | `owner:ingestion` | gate:bulk-ingestion | — |
| `occurrence:coverage-directly-addressed-5c49e10a491a541c` | `docs-future/coverage.md` | Directly addressed | `sha256:5c49e10a491a541c` | marker:gate;marker:gated | `capability:autonomous-identity` | activation_gate | `owner:identity` | gate:autonomous-identity | — |
| `occurrence:coverage-directly-addressed-92b44f4955156b21` | `docs-future/coverage.md` | Directly addressed | `sha256:92b44f4955156b21` | marker:gated | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:coverage-directly-addressed-0134ae1f7f113acd` | `docs-future/coverage.md` | Directly addressed | `sha256:0134ae1f7f113acd` | marker:gate | `capability:procedural-memory` | activation_gate | `owner:ingestion` | gate:procedural-memory | — |
| `occurrence:coverage-directly-addressed-6fa09a0dc21f6af4` | `docs-future/coverage.md` | Directly addressed | `sha256:6fa09a0dc21f6af4` | marker:gate | `capability:working-memory` | activation_gate | `owner:ingestion` | gate:working-memory | — |
| `occurrence:coverage-directly-addressed-ac57794348052669` | `docs-future/coverage.md` | Directly addressed | `sha256:ac57794348052669` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:coverage-fixed-in-the-current-system-9add2924ec5c315a` | `docs-future/coverage.md` | Fixed in the current system | `sha256:9add2924ec5c315a` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-answered-obliquely-5d37f2f8384472ab` | `docs-future/coverage.md` | Answered obliquely | `sha256:5d37f2f8384472ab` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-answered-obliquely-9795ee4cca886fbd` | `docs-future/coverage.md` | Answered obliquely | `sha256:9795ee4cca886fbd` | marker:gate;marker:open-experiment | `capability:proactive-initiation` | activation_gate | `owner:operations` | gate:proactive-initiation | — |
| `occurrence:coverage-answered-obliquely-edda72b477f7a04f` | `docs-future/coverage.md` | Answered obliquely | `sha256:edda72b477f7a04f` | marker:gate;marker:gated;marker:disabled;rule:until-condition | `capability:historical-reinspection` | activation_gate | `owner:model-eval` | gate:historical-reinspection | — |
| `occurrence:coverage-answered-obliquely-72d6e6f12813f5fe` | `docs-future/coverage.md` | Answered obliquely | `sha256:72d6e6f12813f5fe` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current programme status. |
| `occurrence:coverage-made-worse-ae8df5755aeaa4ab` | `docs-future/coverage.md` | Made worse | `sha256:ae8df5755aeaa4ab` | marker:deferred;marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:coverage-inherited-not-solved-c005ee7db290a266` | `docs-future/coverage.md` | Inherited, not solved | `sha256:c005ee7db290a266` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-additional-issue-records-50cd66287e6a61e9` | `docs-future/coverage.md` | Additional issue records | `sha256:50cd66287e6a61e9` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-issues-whose-right-fix-changes-18304c1e0a2ef62f` | `docs-future/coverage.md` | Issues whose right fix changes | `sha256:18304c1e0a2ef62f` | marker:gate | `non_capability_allowlist:lifecycle-state-vocabulary` | non_capability | `owner:architecture` | none | object lifecycle state, not programme activation |
| `occurrence:coverage-issues-whose-right-fix-changes-255073fcab0e4042` | `docs-future/coverage.md` | Issues whose right fix changes | `sha256:255073fcab0e4042` | marker:gate | `capability:bulk-ingestion; capability:historical-reinspection; capability:ocr; capability:generated-captions; capability:region-grounding; capability:visual-retrieval; capability:scene-graph-writer` | activation_gate | `owner:ingestion` | gate:bulk-ingestion; gate:historical-reinspection; gate:ocr; gate:generated-captions; gate:region-grounding; gate:visual-retrieval; gate:scene-graph-writer | — |
| `occurrence:coverage-not-addressed-b00849557e88e23a` | `docs-future/coverage.md` | Not addressed | `sha256:b00849557e88e23a` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-not-addressed-ca754e2e323e7f68` | `docs-future/coverage.md` | Not addressed | `sha256:ca754e2e323e7f68` | marker:optional;marker:gate;marker:disabled;rule:until-condition | `capability:subagent-spawning` | activation_gate | `owner:operations` | gate:subagent-spawning | — |
| `occurrence:coverage-obligation-register-f49a932857375d24` | `docs-future/coverage.md` | Obligation register | `sha256:f49a932857375d24` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-obligation-register-ceb9bd138be66d10` | `docs-future/coverage.md` | Obligation register | `sha256:ceb9bd138be66d10` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:coverage-obligation-register-f5b5e640e6093e5c` | `docs-future/coverage.md` | Obligation register | `sha256:f5b5e640e6093e5c` | marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:coverage-obligation-register-63c241d00d5e4217` | `docs-future/coverage.md` | Obligation register | `sha256:63c241d00d5e4217` | marker:disabled | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:coverage-obligation-register-ca63c6709f200ffc` | `docs-future/coverage.md` | Obligation register | `sha256:ca63c6709f200ffc` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:events-and-roles-roles-and-attributes-37c4e9778c94e376` | `docs-future/events-and-roles.md` | Roles and attributes | `sha256:37c4e9778c94e376` | marker:gate;rule:only-after-condition | `capability:broad-event-role-vocabulary` | activation_gate | `owner:schema` | gate:broad-event-role-vocabulary | — |
| `occurrence:events-and-roles-co-reference-fixtures-a5bd6c8f147e75cb` | `docs-future/events-and-roles.md` | Co-reference fixtures | `sha256:a5bd6c8f147e75cb` | marker:gate | `capability:autonomous-event-merging` | activation_gate | `owner:schema` | gate:autonomous-event-merging | — |
| `occurrence:identity-resolution-environments-and-derivation-58cb35a199ec816e` | `docs-future/identity.md` | Resolution environments and derivation | `sha256:58cb35a199ec816e` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:identity-evidence-and-authority-5e1ae43a9780d4fc` | `docs-future/identity.md` | Evidence and authority | `sha256:5e1ae43a9780d4fc` | marker:gate;marker:disabled;rule:until-condition | `capability:autonomous-identity` | activation_gate | `owner:identity` | gate:autonomous-identity | — |
| `occurrence:lineage-lineage-6906b8021ab17651` | `docs-future/lineage.md` | Lineage | `sha256:6906b8021ab17651` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:lineage-cardinality-moved-from-the-class-to-the--505ed84a7b46709f` | `docs-future/lineage.md` | Cardinality moved from the class to the individual | `sha256:505ed84a7b46709f` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:lineage-cardinality-moved-from-the-class-to-the--4b783885360e3817` | `docs-future/lineage.md` | Cardinality moved from the class to the individual | `sha256:4b783885360e3817` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:memory-typology-episodic-3384e1111ab673df` | `docs-future/memory-typology.md` | Episodic | `sha256:3384e1111ab673df` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:memory-typology-episodic-07981d758f52703a` | `docs-future/memory-typology.md` | Episodic | `sha256:07981d758f52703a` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:memory-typology-procedural-81a36c15b1dea577` | `docs-future/memory-typology.md` | Procedural | `sha256:81a36c15b1dea577` | marker:gate | `capability:procedural-memory` | activation_gate | `owner:ingestion` | gate:procedural-memory | — |
| `occurrence:memory-typology-conversational-artefacts-are-not-bulk-in-91072f22cbdfeac6` | `docs-future/memory-typology.md` | Conversational artefacts are not bulk ingestion | `sha256:91072f22cbdfeac6` | marker:gate | `capability:historical-reinspection; capability:ocr; capability:generated-captions; capability:region-grounding; capability:visual-retrieval` | activation_gate | `owner:model-eval` | gate:historical-reinspection; gate:ocr; gate:generated-captions; gate:region-grounding; gate:visual-retrieval | — |
| `occurrence:memory-typology-conversational-artefacts-are-not-bulk-in-cc9bbae96b6bd4ea` | `docs-future/memory-typology.md` | Conversational artefacts are not bulk ingestion | `sha256:cc9bbae96b6bd4ea` | marker:gate | `capability:bulk-ingestion` | activation_gate | `owner:ingestion` | gate:bulk-ingestion | — |
| `occurrence:off-turn-job-state-machine-dc4ae63b19dfe79e` | `docs-future/off-turn.md` | Job state machine | `sha256:dc4ae63b19dfe79e` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:off-turn-exploration-893017b842c5f0d2` | `docs-future/off-turn.md` | Exploration | `sha256:893017b842c5f0d2` | marker:gate;marker:disabled | `capability:exploration` | activation_gate | `owner:operations` | gate:exploration | — |
| `occurrence:off-turn-exploration-8ed41073ab2d27bd` | `docs-future/off-turn.md` | Exploration | `sha256:8ed41073ab2d27bd` | marker:gate;marker:disabled | `capability:exploration` | activation_gate | `owner:operations` | gate:exploration | — |
| `occurrence:off-turn-agent-initiated-work-86b9828402c97bb2` | `docs-future/off-turn.md` | Agent-initiated work | `sha256:86b9828402c97bb2` | marker:gate;rule:until-condition | `capability:proactive-initiation` | activation_gate | `owner:operations` | gate:proactive-initiation | — |
| `occurrence:overview-permanence-contract-deccc26a0ae16a70` | `docs-future/overview.md` | Permanence contract | `sha256:deccc26a0ae16a70` | marker:gate | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | programme vocabulary without capability subject |
| `occurrence:overview-capability-statuses-82fa134fa782ccb9` | `docs-future/overview.md` | Capability statuses | `sha256:82fa134fa782ccb9` | marker:gate | `non_capability_allowlist:lifecycle-state-vocabulary` | non_capability | `owner:architecture` | none | object lifecycle state, not programme activation |
| `occurrence:overview-capability-statuses-f30cd1b3741b7712` | `docs-future/overview.md` | Capability statuses | `sha256:f30cd1b3741b7712` | marker:gate | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | programme vocabulary without capability subject |
| `occurrence:overview-system-commitments-951170d02e86ceba` | `docs-future/overview.md` | System commitments | `sha256:951170d02e86ceba` | marker:gate;marker:disabled;rule:only-after-condition | `capability:exploration` | activation_gate | `owner:operations` | gate:exploration | — |
| `occurrence:privacy-and-provenance-transmission-principles-7b1c9ce4e8e50923` | `docs-future/privacy-and-provenance.md` | Transmission principles | `sha256:7b1c9ce4e8e50923` | marker:gate | `capability:transmission-reciprocity; capability:transmission-purpose-limitation` | activation_gate | `owner:privacy-policy` | gate:transmission-reciprocity; gate:transmission-purpose-limitation | — |
| `occurrence:privacy-and-provenance-witness-evidence-80484c72ae470a6d` | `docs-future/privacy-and-provenance.md` | Witness evidence | `sha256:80484c72ae470a6d` | rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:privacy-and-provenance-subject-guard-2b6150ecb8956595` | `docs-future/privacy-and-provenance.md` | Subject guard | `sha256:2b6150ecb8956595` | rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:privacy-and-provenance-audience-safe-support-and-zero-residue-a2e502ee1bf5d6c3` | `docs-future/privacy-and-provenance.md` | Audience-safe support and zero residue | `sha256:a2e502ee1bf5d6c3` | marker:gate;marker:gated | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:privacy-and-provenance-storage-classes-b4f04398afa762a2` | `docs-future/privacy-and-provenance.md` | Storage classes | `sha256:b4f04398afa762a2` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:privacy-and-provenance-restore-7f0d7dde9aad272c` | `docs-future/privacy-and-provenance.md` | Restore | `sha256:7f0d7dde9aad272c` | rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:privacy-and-provenance-execution-and-completion-ab13d9dd5b9960c7` | `docs-future/privacy-and-provenance.md` | Execution and completion | `sha256:ab13d9dd5b9960c7` | rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:privacy-and-provenance-execution-and-completion-3e5541fe0228bef9` | `docs-future/privacy-and-provenance.md` | Execution and completion | `sha256:3e5541fe0228bef9` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:privacy-and-provenance-execution-and-completion-35219e1e0e017dfb` | `docs-future/privacy-and-provenance.md` | Execution and completion | `sha256:35219e1e0e017dfb` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:privacy-and-provenance-inter-agent-status-and-revocation-20072f7c427232a2` | `docs-future/privacy-and-provenance.md` | Inter-agent status and revocation | `sha256:20072f7c427232a2` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:query-surface-read-state-machine-5f9c7485d430dd0d` | `docs-future/query-surface.md` | Read state machine | `sha256:5f9c7485d430dd0d` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:query-surface-search-result-kinds-7b00558b251ec575` | `docs-future/query-surface.md` | Search result kinds | `sha256:7b00558b251ec575` | marker:gate | `capability:visual-retrieval` | activation_gate | `owner:model-eval` | gate:visual-retrieval | — |
| `occurrence:query-surface-search-result-kinds-f9210522a2742b63` | `docs-future/query-surface.md` | Search result kinds | `sha256:f9210522a2742b63` | marker:gate | `capability:visual-retrieval` | activation_gate | `owner:model-eval` | gate:visual-retrieval | — |
| `occurrence:query-surface-search-result-kinds-ca4957bb4b7361f1` | `docs-future/query-surface.md` | Search result kinds | `sha256:ca4957bb4b7361f1` | marker:gate | `non_capability_allowlist:historical-claim` | non_capability | `owner:architecture` | none | historical or comparative claim, not current status |
| `occurrence:query-surface-source-retrieval-and-reinspection-ebc0a7040563c637` | `docs-future/query-surface.md` | Source retrieval and reinspection | `sha256:ebc0a7040563c637` | marker:gate;marker:disabled | `capability:historical-reinspection; capability:ocr; capability:region-grounding; capability:visual-retrieval` | activation_gate | `owner:model-eval` | gate:historical-reinspection; gate:ocr; gate:region-grounding; gate:visual-retrieval | — |
| `occurrence:relations-schema-governance-19b5c8f20b5ebe74` | `docs-future/relations.md` | Schema governance | `sha256:19b5c8f20b5ebe74` | marker:inactive | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:relations-seed-vocabulary-and-extension-38f2d8da57f2e7f3` | `docs-future/relations.md` | Seed vocabulary and extension | `sha256:38f2d8da57f2e7f3` | marker:disabled | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | programme vocabulary without capability subject |
| `occurrence:relations-required-fixtures-fb383016c90373ee` | `docs-future/relations.md` | Required fixtures | `sha256:fb383016c90373ee` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:statements-object-ownership-8d45c330c23cd02d` | `docs-future/statements.md` | Object ownership | `sha256:8d45c330c23cd02d` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:statements-object-ownership-0e99cda0679a18f1` | `docs-future/statements.md` | Object ownership | `sha256:0e99cda0679a18f1` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:statements-object-ownership-4b014b16c2e7afdd` | `docs-future/statements.md` | Object ownership | `sha256:4b014b16c2e7afdd` | marker:inactive | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-proposition-530296a36617d0b8` | `docs-future/statements.md` | Proposition | `sha256:530296a36617d0b8` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:statements-schema-neutral-fixture-record-grammar-8a3b4a6fdbb4fb78` | `docs-future/statements.md` | Schema-neutral fixture record grammar | `sha256:8a3b4a6fdbb4fb78` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:statements-schema-neutral-fixture-record-grammar-24a75caadb932607` | `docs-future/statements.md` | Schema-neutral fixture record grammar | `sha256:24a75caadb932607` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:statements-operational-fixture-namespace-41c68a11127e3a82` | `docs-future/statements.md` | Operational fixture namespace | `sha256:41c68a11127e3a82` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-operational-fixture-namespace-44372b498897aac6` | `docs-future/statements.md` | Operational fixture namespace | `sha256:44372b498897aac6` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-operational-fixture-namespace-15cb54f6d1b8453e` | `docs-future/statements.md` | Operational fixture namespace | `sha256:15cb54f6d1b8453e` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-operational-fixture-namespace-9104131f14782cf9` | `docs-future/statements.md` | Operational fixture namespace | `sha256:9104131f14782cf9` | marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:statements-operational-fixture-namespace-6dfa45247f0c64df` | `docs-future/statements.md` | Operational fixture namespace | `sha256:6dfa45247f0c64df` | marker:gate;rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:statements-restore-old-backup-current-ledger-1cd244010d22c9a5` | `docs-future/statements.md` | `restore-old-backup-current-ledger` | `sha256:1cd244010d22c9a5` | rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:statements-restore-old-backup-current-ledger-4c095fd0dc20a88b` | `docs-future/statements.md` | `restore-old-backup-current-ledger` | `sha256:4c095fd0dc20a88b` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-restore-missing-current-ledger-40fe7e15eb86fa42` | `docs-future/statements.md` | `restore-missing-current-ledger` | `sha256:40fe7e15eb86fa42` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-erase-one-reference-retain-another-ada6530e81b20eb5` | `docs-future/statements.md` | `erase-one-reference-retain-another` | `sha256:ada6530e81b20eb5` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-pending-live-blob-deletion-a996b8abcb51ed66` | `docs-future/statements.md` | `pending-live-blob-deletion` | `sha256:a996b8abcb51ed66` | marker:gate | `non_capability_allowlist:safety-condition` | non_capability | `owner:architecture` | none | safety or authority condition, not activation |
| `occurrence:statements-external-copy-bounded-501a894eee208cc4` | `docs-future/statements.md` | `external-copy-bounded` | `sha256:501a894eee208cc4` | marker:gate | `non_capability_allowlist:evidence-status-vocabulary` | non_capability | `owner:architecture` | none | evidence or review vocabulary, not activation |
| `occurrence:statements-external-copy-bounded-46fd3ac088254a81` | `docs-future/statements.md` | `external-copy-bounded` | `sha256:46fd3ac088254a81` | marker:optional | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:statements-external-copy-bounded-76b09a49bb2bfab9` | `docs-future/statements.md` | `external-copy-bounded` | `sha256:76b09a49bb2bfab9` | rule:only-after-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:statements-external-copy-bounded-0c6ae1fc7450bb6b` | `docs-future/statements.md` | `external-copy-bounded` | `sha256:0c6ae1fc7450bb6b` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:time-occurrence-task-and-trigger-b7f3e85ba195aa27` | `docs-future/time.md` | Occurrence, Task, and Trigger | `sha256:b7f3e85ba195aa27` | marker:optional;marker:inactive | `non_capability_allowlist:ordinary-optional-schema` | non_capability | `owner:architecture` | none | optional schema field, not capability |
| `occurrence:time-modality-4f84c46e4a32e59f` | `docs-future/time.md` | Modality | `sha256:4f84c46e4a32e59f` | marker:gate | `capability:qualitative-temporal-inference; capability:habitual-deontic-inference` | activation_gate | `owner:temporal-policy` | gate:qualitative-temporal-inference; gate:habitual-deontic-inference | — |
| `occurrence:time-genesis-and-policy-boundary-fbb70eab70ac643d` | `docs-future/time.md` | Genesis and policy boundary | `sha256:fbb70eab70ac643d` | marker:gate | `capability:qualitative-temporal-inference; capability:business-calendar-adjustment; capability:volatility-automation; capability:habitual-deontic-inference; capability:autonomous-recurrence-interpretation` | activation_gate | `owner:temporal-policy` | gate:qualitative-temporal-inference; gate:business-calendar-adjustment; gate:volatility-automation; gate:habitual-deontic-inference; gate:autonomous-recurrence-interpretation | — |
| `occurrence:two-traces-the-two-traces-6b83dc98eef4f0bb` | `docs-future/two-traces.md` | The two traces | `sha256:6b83dc98eef4f0bb` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:two-traces-the-generated-trace-activation-gate-412d5919e05653b9` | `docs-future/two-traces.md` | The generated trace activation gate | `sha256:412d5919e05653b9` | marker:gate | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | programme vocabulary without capability subject |
| `occurrence:two-traces-the-generated-trace-activation-gate-275f31ac966146a6` | `docs-future/two-traces.md` | The generated trace activation gate | `sha256:275f31ac966146a6` | marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:two-traces-enablement-gate-944227b486e64f79` | `docs-future/two-traces.md` | Enablement gate | `sha256:944227b486e64f79` | marker:gate | `non_capability_allowlist:programme-taxonomy-language` | non_capability | `owner:architecture` | none | programme vocabulary without capability subject |
| `occurrence:two-traces-enablement-gate-2664fe7577b7d8f0` | `docs-future/two-traces.md` | Enablement gate | `sha256:2664fe7577b7d8f0` | marker:disabled;marker:gate | `capability:generated-episodes` | activation_gate | `owner:model-eval` | gate:generated-episodes | — |
| `occurrence:verified-write-proposal-state-machine-f9cc45bab900c3c6` | `docs-future/verified-write.md` | Proposal state machine | `sha256:f9cc45bab900c3c6` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:verified-write-candidate-and-publication-status-57794a3ba3eb5d57` | `docs-future/verified-write.md` | Candidate and publication status | `sha256:57794a3ba3eb5d57` | marker:disabled;rule:until-condition;marker:gate | `capability:support-fusion` | activation_gate | `owner:belief` | gate:support-fusion | — |
| `occurrence:write-surface-record-f53603d58d62faac` | `docs-future/write-surface.md` | `record` | `sha256:f53603d58d62faac` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:write-surface-required-caller-decisions-c6b3eb972c92e9d0` | `docs-future/write-surface.md` | Required caller decisions | `sha256:c6b3eb972c92e9d0` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |
| `occurrence:write-surface-block-and-retry-behaviour-11473fabf4a724fd` | `docs-future/write-surface.md` | Block and retry behaviour | `sha256:11473fabf4a724fd` | rule:until-condition | `non_capability_allowlist:lifecycle-precondition` | non_capability | `owner:architecture` | none | lifecycle or commit condition, not capability |

## Activation-gate register

An activation-gate row has these fields: `gate_id`, `capability_ids`, `prerequisite_capability_ids`, `required_genesis_inputs`, `produced_outputs`, `independent_evidence_ids`, `executable_oracle_ids`, `activation_decision_and_policy_version`, `disabled_behaviour_oracle`, `additive_seam_reference`, `falsification_and_stop_conditions`, `failure_isolation`, `disposable_artefacts`, `retained_evidence`, and `blocking_scope`.

Every gate is conditional. A gate failure isolates the listed capability unless it proves a shared substrate fault. Every job-dependent gate names `stage:8` in its dependency-register row. Every gate records disabled behaviour and an additive seam even when the operator declines it.

| gate_id | capability_ids | prerequisite_capability_ids | required_genesis_inputs | produced_outputs | independent_evidence_ids | executable_oracle_ids | activation_decision_and_policy_version | disabled_behaviour_oracle | additive_seam_reference | falsification_and_stop_conditions | failure_isolation | disposable_artefacts | retained_evidence | blocking_scope |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `gate:generated-episodes` | `capability:generated-episodes` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:operational-job-substrate` | non-evidence wall, source and audience policy | attributed episode records | `evidence:gate-generated-episodes` | `oracle:gate-generated-episodes` | independent version | `oracle:disabled-generated-episodes` | `separate Activity and Derivation records` | invention, hidden source, cost, or privacy failure | capability only | episodes, indexes, projections | prompts, source fixtures, judge traces | post-genesis unless selected |
| `gate:bulk-ingestion` | `capability:bulk-ingestion` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:operational-job-substrate` | source limits, document audience, work budget | bounded source-first jobs | `evidence:gate-bulk-ingestion` | `oracle:gate-bulk-ingestion` | independent version | `oracle:disabled-bulk-ingestion` | `stage:8` job and source lineage | unbounded work, source loss, audience loss, or cancellation failure | capability only | ingestion jobs, derived structure | source fixtures, schedules, measurements | post-genesis unless selected |
| `gate:procedural-memory` | `capability:procedural-memory` | `capability:operational-job-substrate` | lifecycle and retention policy | procedural memory records | `evidence:gate-procedural-memory` | `oracle:gate-procedural-memory` | independent version | `oracle:disabled-procedural-memory` | typed memory lifecycle | unbounded retention or authority escalation | capability only | memory records and indexes | lifecycle fixtures and replay traces | post-genesis unless selected |
| `gate:working-memory` | `capability:working-memory` | `capability:operational-job-substrate` | lifecycle, expiry, and audience policy | working memory records | `evidence:gate-working-memory` | `oracle:gate-working-memory` | independent version | `oracle:disabled-working-memory` | typed memory lifecycle | expiry, audience, or source loss | capability only | memory records and indexes | expiry fixtures and replay traces | post-genesis unless selected |
| `gate:historical-reinspection` | `capability:historical-reinspection` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | source version, reinspection authority, audience | versioned Perception or Derivation | `evidence:gate-historical-reinspection` | `oracle:gate-historical-reinspection` | independent version | `oracle:disabled-historical-reinspection` | source-preserving Perception seam | hidden reinspection, laundering, or erasure failure | capability only | new perceptions and indexes | source fixtures, access traces | post-genesis unless selected |
| `gate:ocr` | `capability:ocr` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | OCR model, source locator, audience | attributed OCR Perception | `evidence:gate-ocr` | `oracle:gate-ocr` | independent version | `oracle:disabled-ocr` | versioned Perception seam | invented text, source loss, or privacy failure | capability only | OCR records and indexes | image fixtures, corrections, measurements | post-genesis unless selected |
| `gate:generated-captions` | `capability:generated-captions` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | caption model, source, audience | attributed caption Perception | `evidence:gate-generated-captions` | `oracle:gate-generated-captions` | independent version | `oracle:disabled-generated-captions` | versioned Perception seam | invention, laundering, or erasure failure | capability only | captions and indexes | media fixtures, provenance traces | post-genesis unless selected |
| `gate:region-grounding` | `capability:region-grounding` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | geometry, source, audience | bounded region selectors | `evidence:gate-region-grounding` | `oracle:gate-region-grounding` | independent version | `oracle:disabled-region-grounding` | selector and Perception seam | geometry variance, invention, or privacy failure | capability only | regions, indexes, projections | geometry fixtures and access traces | post-genesis unless selected |
| `gate:visual-retrieval` | `capability:visual-retrieval` | `capability:source-activity-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | retrieval corpus, audience, cost budget | authorised retrieval results | `evidence:gate-visual-retrieval` | `oracle:gate-visual-retrieval` | independent version | `oracle:disabled-visual-retrieval` | reference-authorised index seam | hidden source, cost, or erasure failure | capability only | embeddings and indexes | corpus fixtures, access probes | post-genesis unless selected |
| `gate:scene-graph-writer` | `capability:scene-graph-writer` | `capability:source-activity-substrate`, `capability:assertion-definition-substrate`, `capability:audience-influence-substrate`, `capability:artefact-erasure-recovery-substrate`, `capability:operational-job-substrate` | graph writer safety plan, source and audience | bounded graph proposals | `evidence:gate-scene-graph-writer` | `oracle:gate-scene-graph-writer` | independent version | `oracle:disabled-scene-graph-writer` | proposal-only graph writer | source laundering, graph invention, or unsafe disclosure | capability only | graph proposals and indexes | graph fixtures and safety review | declined unless reopened |
| `gate:rich-recurrence` | `capability:rich-recurrence` | `capability:initial-temporal-policy` | recurrence policy and calendar inputs | typed recurrence records | `evidence:gate-rich-recurrence` | `oracle:gate-rich-recurrence` | independent version | `oracle:disabled-rich-recurrence` | separate recurrence policy | invalid or guessed recurrence | capability only | recurrence plans | transition fixtures and decisions | post-genesis unless selected |
| `gate:business-calendar-adjustment` | `capability:business-calendar-adjustment` | `capability:initial-temporal-policy` | named calendar and timezone | adjusted occurrences | `evidence:gate-business-calendar-adjustment` | `oracle:gate-business-calendar-adjustment` | independent version | `oracle:disabled-business-calendar-adjustment` | explicit calendar policy | silent calendar substitution or source mutation | capability only | adjusted occurrences | calendar fixtures and measurements | post-genesis unless selected |
| `gate:volatility-automation` | `capability:volatility-automation` | `capability:initial-temporal-policy`, `capability:operational-job-substrate` | volatility evidence and budget | bounded update jobs | `evidence:gate-volatility-automation` | `oracle:gate-volatility-automation` | independent version | `oracle:disabled-volatility-automation` | `stage:8` job seam | unbounded work, false urgency, or authority escalation | capability only | update jobs and projections | schedules, failures, budgets | post-genesis unless selected |
| `gate:habitual-deontic-inference` | `capability:habitual-deontic-inference` | `capability:initial-temporal-policy` | policy and evidence threshold | qualified temporal hypotheses | `evidence:gate-habitual-deontic-inference` | `oracle:gate-habitual-deontic-inference` | independent version | `oracle:disabled-habitual-deontic-inference` | hypothesis-only assertion seam | normative invention or source mutation | capability only | hypotheses and projections | fixture inputs, judgements | post-genesis unless selected |
| `gate:qualitative-temporal-inference` | `capability:qualitative-temporal-inference` | `capability:initial-temporal-policy` | qualitative vocabulary and uncertainty policy | qualified temporal hypotheses | `evidence:gate-qualitative-temporal-inference` | `oracle:gate-qualitative-temporal-inference` | independent version | `oracle:disabled-qualitative-temporal-inference` | hypothesis-only assertion seam | guessed time or invalid transition | capability only | hypotheses and projections | fixtures and counterexamples | post-genesis unless selected |
| `gate:autonomous-recurrence-interpretation` | `capability:autonomous-recurrence-interpretation` | `capability:initial-temporal-policy`, `capability:operational-job-substrate` | autonomy policy and budget | bounded interpretation jobs | `evidence:gate-autonomous-recurrence-interpretation` | `oracle:gate-autonomous-recurrence-interpretation` | independent version | `oracle:disabled-autonomous-recurrence-interpretation` | `stage:8` job seam | unbounded inference or accidental Trigger creation | capability only | jobs and hypotheses | schedules, failures, measurements | post-genesis unless selected |
| `gate:broad-event-role-vocabulary` | `capability:broad-event-role-vocabulary` | `capability:initial-event-role-relation-policy` | versioned role registry | governed role definitions | `evidence:gate-broad-event-role-vocabulary` | `oracle:gate-broad-event-role-vocabulary` | independent version | `oracle:disabled-broad-event-role-vocabulary` | versioned definition seam | unsafe role, alias cycle, or disclosure failure | capability only | definitions and projections | role fixtures and replay traces | post-genesis unless selected |
| `gate:autonomous-event-merging` | `capability:autonomous-event-merging` | `capability:initial-event-role-relation-policy`, `capability:operational-job-substrate` | merge authority and review policy | reversible merge proposals | `evidence:gate-autonomous-event-merging` | `oracle:gate-autonomous-event-merging` | independent version | `oracle:disabled-autonomous-event-merging` | proposal-only merge seam | destructive merge or stale commit | capability only | proposals and composites | merge and severance traces | post-genesis unless selected |
| `gate:numeric-support` | `capability:numeric-support` | `capability:initial-support-policy` | numeric semantics and calibration | numeric support observations | `evidence:gate-numeric-support` | `oracle:gate-numeric-support` | independent version | `oracle:disabled-numeric-support` | separate support projection | probability overclaim or hidden influence | capability only | numeric projections | calibration fixtures and judgements | post-genesis unless selected |
| `gate:support-fusion` | `capability:support-fusion` | `capability:initial-support-policy`, `capability:operational-job-substrate` | fusion operators and dependence model | fused support projections | `evidence:gate-support-fusion` | `oracle:gate-support-fusion` | independent version | `oracle:disabled-support-fusion` | additive support projection | dependence gain or privacy leak | capability only | fused projections and jobs | dependence fixtures and traces | post-genesis unless selected |
| `gate:autonomous-contradiction-arbitration` | `capability:autonomous-contradiction-arbitration` | `capability:initial-support-policy`, `capability:operational-job-substrate` | arbitration authority and review | contradiction proposals | `evidence:gate-autonomous-contradiction-arbitration` | `oracle:gate-autonomous-contradiction-arbitration` | independent version | `oracle:disabled-autonomous-contradiction-arbitration` | proposal-only contradiction seam | linguistic overclaim or authority escalation | capability only | proposals and jobs | mechanical fixtures and review | post-genesis unless selected |
| `gate:autonomous-identity` | `capability:autonomous-identity` | `capability:initial-identity-policy`, `capability:operational-job-substrate` | scoring and disclosure policy | identity hypotheses | `evidence:gate-autonomous-identity` | `oracle:gate-autonomous-identity` | independent version | `oracle:disabled-autonomous-identity` | hypothesis-only identity seam | overlap, disclosure effect, or history loss | capability only | hypotheses and composites | identity fixtures and replay traces | post-genesis unless selected |
| `gate:cross-instance-identity` | `capability:cross-instance-identity` | `capability:audience-influence-substrate`, `capability:initial-identity-policy`, `capability:operational-job-substrate` | instance authority and consent | scoped identity hypotheses | `evidence:gate-cross-instance-identity` | `oracle:gate-cross-instance-identity` | independent version | `oracle:disabled-cross-instance-identity` | scoped hypothesis seam | cross-instance disclosure or authority escalation | capability only | hypotheses and indexes | consent fixtures and access traces | post-genesis unless selected |
| `gate:transmission-reciprocity` | `capability:transmission-reciprocity` | `capability:audience-influence-substrate`, `capability:operational-job-substrate` | execution purpose and reciprocity policy | evaluated transmission decisions | `evidence:gate-transmission-reciprocity` | `oracle:gate-transmission-reciprocity` | independent version | `oracle:disabled-transmission-reciprocity` | transmission policy seam | hidden obligation or disclosure failure | capability only | policy evaluations | connector fixtures and decisions | post-genesis unless selected |
| `gate:transmission-purpose-limitation` | `capability:transmission-purpose-limitation` | `capability:audience-influence-substrate`, `capability:operational-job-substrate` | purpose and obligation records | purpose-scoped transmission | `evidence:gate-transmission-purpose-limitation` | `oracle:gate-transmission-purpose-limitation` | independent version | `oracle:disabled-transmission-purpose-limitation` | purpose policy seam | purpose laundering or unbounded obligation | capability only | policy evaluations | purpose fixtures and failures | post-genesis unless selected |
| `gate:remote-actionable-status` | `capability:remote-actionable-status` | `capability:inter-agent-status-freshness`, `capability:operational-job-substrate` | remote authority and freshness lease | bounded remote actions | `evidence:gate-remote-actionable-status` | `oracle:gate-remote-actionable-status` | independent version | `oracle:disabled-remote-actionable-status` | status freshness seam | stale action, key break, or authority escalation | capability only | notices and jobs | status fixtures and race traces | post-genesis unless selected |
| `gate:general-runtime-faithfulness-checking` | `capability:general-runtime-faithfulness-checking` | `capability:operational-job-substrate` | checker scope and budget | faithfulness observations | `evidence:gate-general-runtime-faithfulness-checking` | `oracle:gate-general-runtime-faithfulness-checking` | independent version | `oracle:disabled-general-runtime-faithfulness-checking` | non-authoritative observation seam | checker authority or hidden influence | capability only | observations and jobs | checker fixtures and measurements | post-genesis unless selected |
| `gate:worker-policy` | `capability:worker-policy` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | worker policy and budgets | worker decisions | `evidence:gate-worker-policy` | `oracle:gate-worker-policy` | independent version | `oracle:disabled-worker-policy` | disabled-worker seam | duplicate output, unbounded retry, or authority escalation | capability only | worker state and schedules | race traces and budgets | post-genesis unless selected |
| `gate:maintenance-retirement` | `capability:maintenance-retirement` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | equivalence and retirement policy | retired maintenance paths | `evidence:gate-maintenance-retirement` | `oracle:gate-maintenance-retirement` | independent version | `oracle:disabled-maintenance-retirement` | additive maintenance seam | lost pending state or non-equivalence | capability only | maintenance jobs and projections | longitudinal traces | post-genesis unless selected |
| `gate:drift-response` | `capability:drift-response` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | drift thresholds and response policy | drift response records | `evidence:gate-drift-response` | `oracle:gate-drift-response` | independent version | `oracle:disabled-drift-response` | drift record seam | hidden policy change or uncontrolled response | capability only | drift jobs and records | longitudinal measurements | post-genesis unless selected |
| `gate:exception-processing` | `capability:exception-processing` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | exception authority and queue policy | operator exception records | `evidence:gate-exception-processing` | `oracle:gate-exception-processing` | independent version | `oracle:disabled-exception-processing` | exception record seam | authority bypass or lost audit trail | capability only | exception queues and jobs | exception fixtures and decisions | post-genesis unless selected |
| `gate:exploration` | `capability:exploration` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | exploration scope, privacy, and budget | bounded exploration proposals | `evidence:gate-exploration` | `oracle:gate-exploration` | independent version | `oracle:disabled-exploration` | proposal-only job seam | unsolicited disclosure, unbounded work, or authority escalation | capability only | exploration jobs and proposals | scope fixtures and budgets | post-genesis unless selected |
| `gate:proactive-initiation` | `capability:proactive-initiation` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | initiation policy, purpose, and budget | bounded initiation proposals | `evidence:gate-proactive-initiation` | `oracle:gate-proactive-initiation` | independent version | `oracle:disabled-proactive-initiation` | proposal-only job seam | unsolicited action, stale commit, or purpose failure | capability only | initiation jobs and proposals | purpose fixtures and race traces | post-genesis unless selected |
| `gate:subagent-spawning` | `capability:subagent-spawning` | `capability:operational-job-substrate`, `capability:disabled-worker-contract` | spawning authority, isolation, and budget | bounded subagent records | `evidence:gate-subagent-spawning` | `oracle:gate-subagent-spawning` | independent version | `oracle:disabled-subagent-spawning` | additive job and authority seam | authority escape, unbounded cost, or duplicate publication | capability only | subagent jobs and records | isolation tests and budgets | post-genesis unless selected |

## Owner register

| owner_id | repository area or decision authority |
|---|---|
| `owner:architecture` | candidate contract, object boundaries, dependency closure, and cross-domain design |
| `owner:security` | audience non-interference, witness assurance, subject guards, and disclosure safety |
| `owner:storage` | payloads, blobs, authoritative ledger, restore, projection rebuild, and erasure closure |
| `owner:operator` | selection record, operator policy, freeze approval, rehearsal approval, and genesis authorisation |
| `owner:model-eval` | model evidence, sampling, uncertainty, judges, and optional interpretation gates |
| `owner:temporal-policy` | typed time, recurrence, timezone, and temporal activation gates |
| `owner:schema` | assertions, definitions, Events, roles, relations, and schema replay |
| `owner:identity` | identity hypotheses, composites, severance, and identity activation gates |
| `owner:belief` | support, dependence, contradiction, and belief activation gates |
| `owner:ingestion` | bulk ingestion and memory lifecycle activation gates |
| `owner:operations` | jobs, disabled workers, maintenance, drift, exceptions, and initiation gates |
| `owner:connector` | witness inputs, availability, presence, and inter-agent status |
| `owner:privacy-policy` | richer transmission principles and purpose obligations |
| `owner:console` | browser, log, projection, and eval-package budget evidence |
| `owner:artefacts` | Artefact identity, selectors, Perceptions, lineage, and media access |
| `owner:assertions` | Proposition and Assertion identity, transitions, and candidate-state folds |
| `owner:current-system-components` | Current implementation and connector components outside the successor architecture |
| `owner:current-system-research` | Current-system research and evaluations outside successor evidence |
| `owner:events` | Event identity, role projections, co-reference, and severance |
| `owner:memory` | Memory kinds, episode boundaries, retention, and generated-memory safeguards |
| `owner:privacy` | Audience, witness, influence, transmission, and erasure privacy contracts |
| `owner:query` | Query classification, retrieval sufficiency, and audience-safe query projection |
| `owner:source` | Source and Occasion capture, connector provenance, and source retention |
| `owner:statements` | Referential frames, polarity, modality, and statement presentation |
| `owner:write-path` | Proposal transactions, critics, atomic publication, and source-only fallback |

## Evidence register

| evidence_id | owner_id | producer_node_id | retained artefact | consumers |
|---|---|---|---|---|
| `evidence:stage0-contract` | `owner:architecture` | `stage:0` | candidate contract, dependency register, selection schema, and census contract | all stages, freeze |
| `evidence:1a-episode-ablation` | `owner:model-eval` | `stage:1` | episode ablation package | `gate:generated-episodes` |
| `evidence:1b-witness` | `owner:security` | `stage:1` | witness and availability assurance package | `stage:5`, `stage:7`, freeze |
| `evidence:1c-contract` | `owner:architecture` | `stage:1` | preregistered extraction fixture contract | `stage:2`, `stage:7` |
| `evidence:1c-measurement` | `owner:model-eval` | `stage:1` | extraction convergence and economics package | `stage:7`, initial writes |
| `evidence:1d-budgets` | `owner:console` | `stage:1` | storage, browser, log, blob, and package budgets | `stage:6`, freeze |
| `evidence:1e-query-classification` | `owner:architecture` | `stage:1` | query-surface classification | `stage:7`, freeze |
| `evidence:1f-multimodal-recall` | `owner:model-eval` | `stage:1` | multimodal recall package | optional multimodal gates |
| `evidence:stage2-reference` | `owner:architecture` | `stage:2` | fixture compiler, reference materialiser, folds, probes, and digests | stages 3 through 12, gates, freeze |
| `evidence:stage3-source-activity` | `owner:architecture` | `stage:3` | source and Activity substrate package | later stages, gates |
| `evidence:stage4-assertion-definition` | `owner:schema` | `stage:4` | assertion, definition, and transition package | stages 5 through 12, gates |
| `evidence:stage5-audience-influence` | `owner:security` | `stage:5` | audience and InfluenceEnvelope package | stage 6, stage 7, gates |
| `evidence:stage5-status-freshness` | `owner:connector` | `stage:5` | signed status freshness replay package | genesis, `gate:remote-actionable-status` |
| `evidence:stage6-erasure-recovery` | `owner:storage` | `stage:6` | erasure, ledger, restore, lock, and budget package | stage 7, freeze, rehearsal |
| `evidence:stage7-vertical-slice` | `owner:architecture` | `stage:7` | source-to-render vertical-slice report | stage 8, stages 9 through 12, freeze |
| `evidence:stage8-operational-substrate` | `owner:operations` | `stage:8` | event-sourced job substrate report | job-dependent gates, freeze |
| `evidence:stage8-disabled-worker` | `owner:operations` | `stage:8` | disabled-worker and additive-activation report | all optional worker gates, freeze |
| `evidence:gate-generated-episodes` | `owner:model-eval` | `gate:generated-episodes` | generated episode activation package | gate, freeze, rehearsal |
| `evidence:gate-bulk-ingestion` | `owner:ingestion` | `gate:bulk-ingestion` | bulk ingestion activation package | gate, freeze, rehearsal |
| `evidence:gate-procedural-memory` | `owner:ingestion` | `gate:procedural-memory` | procedural memory activation package | gate, freeze, rehearsal |
| `evidence:gate-working-memory` | `owner:ingestion` | `gate:working-memory` | working memory activation package | gate, freeze, rehearsal |
| `evidence:gate-historical-reinspection` | `owner:model-eval` | `gate:historical-reinspection` | historical reinspection activation package | gate, freeze, rehearsal |
| `evidence:gate-ocr` | `owner:model-eval` | `gate:ocr` | OCR activation package | gate, freeze, rehearsal |
| `evidence:gate-generated-captions` | `owner:model-eval` | `gate:generated-captions` | generated captions activation package | gate, freeze, rehearsal |
| `evidence:gate-region-grounding` | `owner:model-eval` | `gate:region-grounding` | region grounding activation package | gate, freeze, rehearsal |
| `evidence:gate-visual-retrieval` | `owner:model-eval` | `gate:visual-retrieval` | visual retrieval activation package | gate, freeze, rehearsal |
| `evidence:gate-scene-graph-writer` | `owner:model-eval` | `gate:scene-graph-writer` | scene graph writer activation package | gate, freeze, rehearsal |
| `evidence:gate-rich-recurrence` | `owner:temporal-policy` | `gate:rich-recurrence` | rich recurrence activation package | gate, freeze, rehearsal |
| `evidence:gate-business-calendar-adjustment` | `owner:temporal-policy` | `gate:business-calendar-adjustment` | business calendar activation package | gate, freeze, rehearsal |
| `evidence:gate-volatility-automation` | `owner:temporal-policy` | `gate:volatility-automation` | volatility automation activation package | gate, freeze, rehearsal |
| `evidence:gate-habitual-deontic-inference` | `owner:temporal-policy` | `gate:habitual-deontic-inference` | habitual and deontic inference activation package | gate, freeze, rehearsal |
| `evidence:gate-qualitative-temporal-inference` | `owner:temporal-policy` | `gate:qualitative-temporal-inference` | qualitative temporal inference activation package | gate, freeze, rehearsal |
| `evidence:gate-autonomous-recurrence-interpretation` | `owner:temporal-policy` | `gate:autonomous-recurrence-interpretation` | autonomous recurrence activation package | gate, freeze, rehearsal |
| `evidence:gate-broad-event-role-vocabulary` | `owner:schema` | `gate:broad-event-role-vocabulary` | broad Event role activation package | gate, freeze, rehearsal |
| `evidence:gate-autonomous-event-merging` | `owner:schema` | `gate:autonomous-event-merging` | autonomous Event merging activation package | gate, freeze, rehearsal |
| `evidence:gate-numeric-support` | `owner:belief` | `gate:numeric-support` | numeric support activation package | gate, freeze, rehearsal |
| `evidence:gate-support-fusion` | `owner:belief` | `gate:support-fusion` | support fusion activation package | gate, freeze, rehearsal |
| `evidence:gate-autonomous-contradiction-arbitration` | `owner:belief` | `gate:autonomous-contradiction-arbitration` | autonomous contradiction activation package | gate, freeze, rehearsal |
| `evidence:gate-autonomous-identity` | `owner:identity` | `gate:autonomous-identity` | autonomous identity activation package | gate, freeze, rehearsal |
| `evidence:gate-cross-instance-identity` | `owner:identity` | `gate:cross-instance-identity` | cross-instance identity activation package | gate, freeze, rehearsal |
| `evidence:gate-transmission-reciprocity` | `owner:privacy-policy` | `gate:transmission-reciprocity` | transmission reciprocity activation package | gate, freeze, rehearsal |
| `evidence:gate-transmission-purpose-limitation` | `owner:privacy-policy` | `gate:transmission-purpose-limitation` | transmission purpose activation package | gate, freeze, rehearsal |
| `evidence:gate-remote-actionable-status` | `owner:connector` | `gate:remote-actionable-status` | remote actionable status activation package | gate, freeze, rehearsal |
| `evidence:gate-general-runtime-faithfulness-checking` | `owner:model-eval` | `gate:general-runtime-faithfulness-checking` | runtime faithfulness activation package | gate, freeze, rehearsal |
| `evidence:gate-worker-policy` | `owner:operations` | `gate:worker-policy` | worker policy activation package | gate, freeze, rehearsal |
| `evidence:gate-maintenance-retirement` | `owner:operations` | `gate:maintenance-retirement` | maintenance retirement activation package | gate, freeze, rehearsal |
| `evidence:gate-drift-response` | `owner:operations` | `gate:drift-response` | drift response activation package | gate, freeze, rehearsal |
| `evidence:gate-exception-processing` | `owner:operations` | `gate:exception-processing` | exception processing activation package | gate, freeze, rehearsal |
| `evidence:gate-exploration` | `owner:operations` | `gate:exploration` | exploration activation package | gate, freeze, rehearsal |
| `evidence:gate-proactive-initiation` | `owner:operations` | `gate:proactive-initiation` | proactive initiation activation package | gate, freeze, rehearsal |
| `evidence:gate-subagent-spawning` | `owner:operations` | `gate:subagent-spawning` | subagent spawning activation package | gate, freeze, rehearsal |
| `evidence:stage9-temporal-policy` | `owner:temporal-policy` | `stage:9` | initial temporal policy report | freeze, rehearsal |
| `evidence:stage10-event-policy` | `owner:schema` | `stage:10` | initial Event and relation policy report | freeze, rehearsal |
| `evidence:stage11-identity-policy` | `owner:identity` | `stage:11` | initial identity policy report | freeze, rehearsal |
| `evidence:stage12-support-policy` | `owner:belief` | `stage:12` | initial support policy report | freeze, rehearsal |
| `evidence:freeze-record` | `owner:operator` | `lifecycle:genesis-freeze` | versioned genesis-freeze-record | rehearsal, first genesis |
| `evidence:rehearsal-report` | `owner:operator` | `lifecycle:whole-system-rehearsal` | whole-system-rehearsal-report | first genesis |
| `evidence:all-genesis-blockers` | `owner:architecture` | `lifecycle:genesis-freeze` | closure and blocker report | first genesis |

The activation-gate evidence IDs are `evidence:gate-<capability-name>` for every gate row. Each package retains its source fixtures, preregistered thresholds, exact outputs, failures, measurements, and decision rationale. A gate package is independent even when it reuses `evidence:stage2-reference` or a substrate seam.

## Executable handoff register

Every handoff row states the question, implementation or evidence artefact, structural oracle or preregistered threshold, failure that returns the design to revision, executable tests, and blocking scope. The row also names exactly one canonical node and owner.

| handoff_id | canonical node ID | owner_id | evidence_ids | oracle_ids | blocking scope | question | artefact | threshold | failure | executable tests |
|---|---|---|---|---|---|---|---|---|---|---|
| `handoff:stage0` | `stage:0` | `owner:architecture` | `evidence:stage0-contract` | `oracle:stage0-contract` | all programme items | Which candidate identities, inputs, folds, boundaries, and capabilities must survive genesis? | candidate contract and selection record schema | complete IDs, alternatives, and no stage selection | missing identity, cycle, or hidden requirement | `test:stage0-contract` |
| `handoff:stage1` | `stage:1` | `owner:model-eval` | `evidence:1a-episode-ablation`, `evidence:1b-witness`, `evidence:1c-contract`, `evidence:1c-measurement`, `evidence:1d-budgets`, `evidence:1e-query-classification`, `evidence:1f-multimodal-recall` | `oracle:stage1-baseline`, `oracle:1a-episode-ablation`, `oracle:1b-witness`, `oracle:1c-measurement`, `oracle:1d-budgets`, `oracle:1e-query-classification`, `oracle:1f-multimodal-recall` | consuming stages and selected gates | Do the treatments have measured benefit, cost, and privacy bounds? | independent preregistered evidence packages | fixed sample, uncertainty, budget, tolerance, and pass rule, with usefulness and economics treated as acceptance criteria when applicable | indistinguishable, over-budget, post-hoc, or negative usefulness or economics result | `measurement:1a-episode-ablation`, `measurement:1b-witness`, `test:1c-contract`, `measurement:1c-measurement`, `measurement:1d-budgets`, `test:1e-query-classification`, `measurement:1f-multimodal-recall` |
| `handoff:stage2` | `stage:2` | `owner:architecture` | `evidence:stage2-reference` | `oracle:stage2-reference` | all later stages | Can the object model and folds execute without implicit data? | fixture compiler and reference materialiser | equal folds, probes, selectors, payload sets, and digests | implicit field, invalid chain, or digest mismatch | `test:stage2-reference` |
| `handoff:stage3` | `stage:3` | `owner:architecture` | `evidence:stage3-source-activity` | `oracle:stage3-source-activity` | later substrate | Can source and Activity records retain ordered content and recorded outcomes? | source and Activity substrate | production/reference equality and source-only retention | source loss or replay regeneration | `test:stage3-source-activity` |
| `handoff:stage4` | `stage:4` | `owner:schema` | `evidence:stage4-assertion-definition` | `oracle:stage4-assertion-definition` | later policy | Can assertions, transitions, and definitions replay without invented history? | assertion and definition substrate | canonical keys, valid chains, and historical definitions | source mutation or implicit seed | `test:stage4-assertion-definition` |
| `handoff:stage5` | `stage:5` | `owner:security` | `evidence:stage5-audience-influence`, `evidence:stage5-status-freshness` | `oracle:stage5-audience-influence`, `oracle:stage5-status-freshness` | read/write, privacy, identity, and gates | Can central audience resolution prevent hidden influence and stale action? | audience resolver and status freshness record | zero hidden residue and fail-closed stale state | distributed resolution or disclosure | `test:stage5-audience-influence`, `test:stage5-status-freshness` |
| `handoff:stage6` | `stage:6` | `owner:storage` | `evidence:stage6-erasure-recovery` | `oracle:stage6-erasure-recovery`, `oracle:stage6-restore-lock` | freeze, rehearsal, genesis | Can erasure and restore close over all managed surfaces? | storage, ledger, restore, and lock harness | zero residue, filtered restore, and blocked unverifiable authority | omitted surface, authority resurrection, or lock failure | `test:stage6-erasure-closure`, `test:stage6-restore-lock` |
| `handoff:stage7` | `stage:7` | `owner:architecture` | `evidence:stage7-vertical-slice`, `evidence:1c-measurement` | `oracle:stage7-vertical-slice`, `oracle:stage7-authority` | stage 8 and initial writes | Can one path retain source, publish candidate structure, inspect candidates explicitly, resolve audience, and account for delivery? | source-and-candidate-to-render vertical slice | fidelity, atomicity, source-only usefulness, candidate inspection, audience, operator schema burden, review rounds, latency, log growth, console/replay, and other applicable Stage 1 budget thresholds | critic bypass, partial publication, hidden residue, source-only uselessness, unavailable candidate inspection, schema burden, review or latency over budget, or log, console, or replay cost over budget | `test:stage7-vertical-slice`, `test:stage7-authority` |
| `handoff:stage8` | `stage:8` | `owner:operations` | `evidence:stage8-operational-substrate`, `evidence:stage8-disabled-worker` | `oracle:stage8-operational-substrate`, `oracle:stage8-disabled-worker` | freeze and job-dependent gates | Can jobs remain bounded and disabled without losing pending state? | event-sourced runner and disabled-worker harness | one valid outcome, zero disabled execution, additive activation | duplicate, stale, unbounded, or hidden execution | `test:stage8-jobs`, `test:stage8-disabled-worker` |
| `handoff:stage9` | `stage:9` | `owner:temporal-policy` | `evidence:stage9-temporal-policy` | `oracle:stage9-temporal-policy` | selected initial temporal policy | Do typed-time corrections preserve source and prevent descriptive firing? | temporal evaluator | zero dated-description firing and reference equality | guessed time or Trigger creation | `test:stage9-temporal-policy` |
| `handoff:stage10` | `stage:10` | `owner:schema` | `evidence:stage10-event-policy` | `oracle:stage10-event-policy` | selected initial Event policy | Are Event and relation policies reversible and audience-safe? | Event and schema evaluator | distinct Events, safe partial views, and replayed definitions | destructive merge or unsafe projection | `test:stage10-event-policy` |
| `handoff:stage11` | `stage:11` | `owner:identity` | `evidence:stage11-identity-policy` | `oracle:stage11-identity-policy` | selected initial identity policy | Can identity provide one handle without overlap or history loss? | identity hypothesis harness | disjoint, disclosure-cleared composites and exact severance | overlap or lost sibling history | `test:stage11-identity-policy` |
| `handoff:stage12` | `stage:12` | `owner:belief` | `evidence:stage12-support-policy` | `oracle:stage12-support-policy` | selected initial support policy | Can support, settlement, withdrawal, and contradiction remain conservative and audience-safe? | support and contradiction projections | selected single-source policy branch, correct default read and withdrawal fold, zero hidden delta, no dependence gain, and mechanical classifications | missing settlement authority, incorrect default recall or withdrawal, hidden influence, or linguistic overclaim | `test:stage12-support-policy` |
| `handoff:gate-generated-episodes` | `gate:generated-episodes` | `owner:model-eval` | `evidence:gate-generated-episodes` | `oracle:gate-generated-episodes`, `oracle:disabled-generated-episodes` | capability activation | Does generated episode production satisfy its evidence, cost, source, and privacy thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | invention, hidden source, cost, or privacy failure | `test:gate-generated-episodes`, `test:disabled-generated-episodes` |
| `handoff:gate-bulk-ingestion` | `gate:bulk-ingestion` | `owner:ingestion` | `evidence:gate-bulk-ingestion` | `oracle:gate-bulk-ingestion`, `oracle:disabled-bulk-ingestion` | capability activation | Does bulk ingestion satisfy its source, audience, work, and cancellation thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | unbounded work, source loss, audience loss, or cancellation failure | `test:gate-bulk-ingestion`, `test:disabled-bulk-ingestion` |
| `handoff:gate-procedural-memory` | `gate:procedural-memory` | `owner:ingestion` | `evidence:gate-procedural-memory` | `oracle:gate-procedural-memory`, `oracle:disabled-procedural-memory` | capability activation | Does procedural memory satisfy its lifecycle, retention, and authority thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | unbounded retention or authority escalation | `test:gate-procedural-memory`, `test:disabled-procedural-memory` |
| `handoff:gate-working-memory` | `gate:working-memory` | `owner:ingestion` | `evidence:gate-working-memory` | `oracle:gate-working-memory`, `oracle:disabled-working-memory` | capability activation | Does working memory satisfy its expiry, audience, and source thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | expiry, audience, or source loss | `test:gate-working-memory`, `test:disabled-working-memory` |
| `handoff:gate-historical-reinspection` | `gate:historical-reinspection` | `owner:model-eval` | `evidence:gate-historical-reinspection` | `oracle:gate-historical-reinspection`, `oracle:disabled-historical-reinspection` | capability activation | Does historical reinspection preserve source authority, audience boundaries, and erasure? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | hidden reinspection, laundering, or erasure failure | `test:gate-historical-reinspection`, `test:disabled-historical-reinspection` |
| `handoff:gate-ocr` | `gate:ocr` | `owner:model-eval` | `evidence:gate-ocr` | `oracle:gate-ocr`, `oracle:disabled-ocr` | capability activation | Does OCR satisfy source, correction, attribution, audience, and privacy thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | invented text, source loss, or privacy failure | `test:gate-ocr`, `test:disabled-ocr` |
| `handoff:gate-generated-captions` | `gate:generated-captions` | `owner:model-eval` | `evidence:gate-generated-captions` | `oracle:gate-generated-captions`, `oracle:disabled-generated-captions` | capability activation | Do generated captions satisfy source, attribution, correction, and privacy thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | invention, laundering, or erasure failure | `test:gate-generated-captions`, `test:disabled-generated-captions` |
| `handoff:gate-region-grounding` | `gate:region-grounding` | `owner:model-eval` | `evidence:gate-region-grounding` | `oracle:gate-region-grounding`, `oracle:disabled-region-grounding` | capability activation | Does region grounding satisfy geometry, source, audience, and privacy thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | geometry variance, invention, or privacy failure | `test:gate-region-grounding`, `test:disabled-region-grounding` |
| `handoff:gate-visual-retrieval` | `gate:visual-retrieval` | `owner:model-eval` | `evidence:gate-visual-retrieval` | `oracle:gate-visual-retrieval`, `oracle:disabled-visual-retrieval` | capability activation | Does visual retrieval satisfy authorised corpus, cost, audience, and erasure thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | hidden source, cost, or erasure failure | `test:gate-visual-retrieval`, `test:disabled-visual-retrieval` |
| `handoff:gate-scene-graph-writer` | `gate:scene-graph-writer` | `owner:model-eval` | `evidence:gate-scene-graph-writer` | `oracle:gate-scene-graph-writer`, `oracle:disabled-scene-graph-writer` | capability activation | Does scene-graph writing remain bounded, source-attributed, and proposal-only? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | source laundering, graph invention, or unsafe disclosure | `test:gate-scene-graph-writer`, `test:disabled-scene-graph-writer` |
| `handoff:gate-rich-recurrence` | `gate:rich-recurrence` | `owner:temporal-policy` | `evidence:gate-rich-recurrence` | `oracle:gate-rich-recurrence`, `oracle:disabled-rich-recurrence` | capability activation | Does rich recurrence satisfy typed policy and recurrence thresholds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | invalid or guessed recurrence | `test:gate-rich-recurrence`, `test:disabled-rich-recurrence` |
| `handoff:gate-business-calendar-adjustment` | `gate:business-calendar-adjustment` | `owner:temporal-policy` | `evidence:gate-business-calendar-adjustment` | `oracle:gate-business-calendar-adjustment`, `oracle:disabled-business-calendar-adjustment` | capability activation | Does business-calendar adjustment preserve named calendar and timezone semantics? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | silent calendar substitution or source mutation | `test:gate-business-calendar-adjustment`, `test:disabled-business-calendar-adjustment` |
| `handoff:gate-volatility-automation` | `gate:volatility-automation` | `owner:temporal-policy` | `evidence:gate-volatility-automation` | `oracle:gate-volatility-automation`, `oracle:disabled-volatility-automation` | capability activation | Does volatility automation remain bounded, typed, and authority-safe? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | unbounded work, false urgency, or authority escalation | `test:gate-volatility-automation`, `test:disabled-volatility-automation` |
| `handoff:gate-habitual-deontic-inference` | `gate:habitual-deontic-inference` | `owner:temporal-policy` | `evidence:gate-habitual-deontic-inference` | `oracle:gate-habitual-deontic-inference`, `oracle:disabled-habitual-deontic-inference` | capability activation | Does habitual and deontic inference remain qualified and non-normative? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | normative invention or source mutation | `test:gate-habitual-deontic-inference`, `test:disabled-habitual-deontic-inference` |
| `handoff:gate-qualitative-temporal-inference` | `gate:qualitative-temporal-inference` | `owner:temporal-policy` | `evidence:gate-qualitative-temporal-inference` | `oracle:gate-qualitative-temporal-inference`, `oracle:disabled-qualitative-temporal-inference` | capability activation | Does qualitative temporal inference preserve explicit uncertainty? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | guessed time or invalid transition | `test:gate-qualitative-temporal-inference`, `test:disabled-qualitative-temporal-inference` |
| `handoff:gate-autonomous-recurrence-interpretation` | `gate:autonomous-recurrence-interpretation` | `owner:temporal-policy` | `evidence:gate-autonomous-recurrence-interpretation` | `oracle:gate-autonomous-recurrence-interpretation`, `oracle:disabled-autonomous-recurrence-interpretation` | capability activation | Does autonomous recurrence interpretation remain bounded and prevent accidental Triggers? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | unbounded inference or accidental Trigger creation | `test:gate-autonomous-recurrence-interpretation`, `test:disabled-autonomous-recurrence-interpretation` |
| `handoff:gate-broad-event-role-vocabulary` | `gate:broad-event-role-vocabulary` | `owner:schema` | `evidence:gate-broad-event-role-vocabulary` | `oracle:gate-broad-event-role-vocabulary`, `oracle:disabled-broad-event-role-vocabulary` | capability activation | Does the broad Event role vocabulary satisfy version and disclosure controls? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | unsafe role, alias cycle, or disclosure failure | `test:gate-broad-event-role-vocabulary`, `test:disabled-broad-event-role-vocabulary` |
| `handoff:gate-autonomous-event-merging` | `gate:autonomous-event-merging` | `owner:schema` | `evidence:gate-autonomous-event-merging` | `oracle:gate-autonomous-event-merging`, `oracle:disabled-autonomous-event-merging` | capability activation | Does autonomous Event merging remain reversible and authority-bounded? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | destructive merge or stale commit | `test:gate-autonomous-event-merging`, `test:disabled-autonomous-event-merging` |
| `handoff:gate-numeric-support` | `gate:numeric-support` | `owner:belief` | `evidence:gate-numeric-support` | `oracle:gate-numeric-support`, `oracle:disabled-numeric-support` | capability activation | Does numeric support avoid probability overclaim and hidden influence? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | probability overclaim or hidden influence | `test:gate-numeric-support`, `test:disabled-numeric-support` |
| `handoff:gate-support-fusion` | `gate:support-fusion` | `owner:belief` | `evidence:gate-support-fusion` | `oracle:gate-support-fusion`, `oracle:disabled-support-fusion` | capability activation | Does support fusion preserve dependence suppression and audience safety? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | dependence gain or privacy leak | `test:gate-support-fusion`, `test:disabled-support-fusion` |
| `handoff:gate-autonomous-contradiction-arbitration` | `gate:autonomous-contradiction-arbitration` | `owner:belief` | `evidence:gate-autonomous-contradiction-arbitration` | `oracle:gate-autonomous-contradiction-arbitration`, `oracle:disabled-autonomous-contradiction-arbitration` | capability activation | Does autonomous contradiction arbitration remain proposal-only and authority-safe? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | linguistic overclaim or authority escalation | `test:gate-autonomous-contradiction-arbitration`, `test:disabled-autonomous-contradiction-arbitration` |
| `handoff:gate-autonomous-identity` | `gate:autonomous-identity` | `owner:identity` | `evidence:gate-autonomous-identity` | `oracle:gate-autonomous-identity`, `oracle:disabled-autonomous-identity` | capability activation | Does autonomous identity preserve disjointness, disclosure, and history? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | overlap, disclosure effect, or history loss | `test:gate-autonomous-identity`, `test:disabled-autonomous-identity` |
| `handoff:gate-cross-instance-identity` | `gate:cross-instance-identity` | `owner:identity` | `evidence:gate-cross-instance-identity` | `oracle:gate-cross-instance-identity`, `oracle:disabled-cross-instance-identity` | capability activation | Does cross-instance identity preserve consent, scope, and authority? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | cross-instance disclosure or authority escalation | `test:gate-cross-instance-identity`, `test:disabled-cross-instance-identity` |
| `handoff:gate-transmission-reciprocity` | `gate:transmission-reciprocity` | `owner:privacy-policy` | `evidence:gate-transmission-reciprocity` | `oracle:gate-transmission-reciprocity`, `oracle:disabled-transmission-reciprocity` | capability activation | Does transmission reciprocity preserve purpose, obligation, and disclosure controls? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | hidden obligation or disclosure failure | `test:gate-transmission-reciprocity`, `test:disabled-transmission-reciprocity` |
| `handoff:gate-transmission-purpose-limitation` | `gate:transmission-purpose-limitation` | `owner:privacy-policy` | `evidence:gate-transmission-purpose-limitation` | `oracle:gate-transmission-purpose-limitation`, `oracle:disabled-transmission-purpose-limitation` | capability activation | Does purpose-limited transmission preserve bounded obligations and audience safety? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | purpose laundering or unbounded obligation | `test:gate-transmission-purpose-limitation`, `test:disabled-transmission-purpose-limitation` |
| `handoff:gate-remote-actionable-status` | `gate:remote-actionable-status` | `owner:connector` | `evidence:gate-remote-actionable-status` | `oracle:gate-remote-actionable-status`, `oracle:disabled-remote-actionable-status` | capability activation | Does remote actionable status reject stale or unauthorised actions? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | stale action, key break, or authority escalation | `test:gate-remote-actionable-status`, `test:disabled-remote-actionable-status` |
| `handoff:gate-general-runtime-faithfulness-checking` | `gate:general-runtime-faithfulness-checking` | `owner:model-eval` | `evidence:gate-general-runtime-faithfulness-checking` | `oracle:gate-general-runtime-faithfulness-checking`, `oracle:disabled-general-runtime-faithfulness-checking` | capability activation | Does runtime faithfulness checking remain observational and non-authoritative? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | checker authority or hidden influence | `test:gate-general-runtime-faithfulness-checking`, `test:disabled-general-runtime-faithfulness-checking` |
| `handoff:gate-worker-policy` | `gate:worker-policy` | `owner:operations` | `evidence:gate-worker-policy` | `oracle:gate-worker-policy`, `oracle:disabled-worker-policy` | capability activation | Does worker policy preserve bounded retry, duplicate protection, and authority limits? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | duplicate output, unbounded retry, or authority escalation | `test:gate-worker-policy`, `test:disabled-worker-policy` |
| `handoff:gate-maintenance-retirement` | `gate:maintenance-retirement` | `owner:operations` | `evidence:gate-maintenance-retirement` | `oracle:gate-maintenance-retirement`, `oracle:disabled-maintenance-retirement` | capability activation | Does maintenance retirement preserve pending state and behavioural equivalence? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | lost pending state or non-equivalence | `test:gate-maintenance-retirement`, `test:disabled-maintenance-retirement` |
| `handoff:gate-drift-response` | `gate:drift-response` | `owner:operations` | `evidence:gate-drift-response` | `oracle:gate-drift-response`, `oracle:disabled-drift-response` | capability activation | Does drift response remain bounded and auditable? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | hidden policy change or uncontrolled response | `test:gate-drift-response`, `test:disabled-drift-response` |
| `handoff:gate-exception-processing` | `gate:exception-processing` | `owner:operations` | `evidence:gate-exception-processing` | `oracle:gate-exception-processing`, `oracle:disabled-exception-processing` | capability activation | Does exception processing preserve authority and its audit trail? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | authority bypass or lost audit trail | `test:gate-exception-processing`, `test:disabled-exception-processing` |
| `handoff:gate-exploration` | `gate:exploration` | `owner:operations` | `evidence:gate-exploration` | `oracle:gate-exploration`, `oracle:disabled-exploration` | capability activation | Does exploration remain bounded, private, and proposal-only? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | unsolicited disclosure, unbounded work, or authority escalation | `test:gate-exploration`, `test:disabled-exploration` |
| `handoff:gate-proactive-initiation` | `gate:proactive-initiation` | `owner:operations` | `evidence:gate-proactive-initiation` | `oracle:gate-proactive-initiation`, `oracle:disabled-proactive-initiation` | capability activation | Does proactive initiation remain purpose-bound and race-safe? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | unsolicited action, stale commit, or purpose failure | `test:gate-proactive-initiation`, `test:disabled-proactive-initiation` |
| `handoff:gate-subagent-spawning` | `gate:subagent-spawning` | `owner:operations` | `evidence:gate-subagent-spawning` | `oracle:gate-subagent-spawning`, `oracle:disabled-subagent-spawning` | capability activation | Does subagent spawning preserve isolation, authority, and cost bounds? | activation-gate row, active harness, disabled oracle, and additive seam | active and disabled results satisfy the declared gate fields | authority escape, unbounded cost, or duplicate publication | `test:gate-subagent-spawning`, `test:disabled-subagent-spawning` |
| `handoff:oracle-seed-definition-set` | `stage:4` | `owner:schema` | `evidence:stage4-assertion-definition` | `oracle:seed-definition-set` | genesis | Does the minimal governed seed contain every permanent mechanic and no social or environmental preload? | seed-definition fixture and historical replay package | every selected seed has a stable ID, version, source, and executable use | a required mechanic lacks a seed or a preload exceeds the governed minimum | `test:oracle-seed-definition-set` |
| `handoff:genesis-freeze` | `lifecycle:genesis-freeze` | `owner:operator` | `evidence:freeze-record`, `evidence:all-genesis-blockers` | `oracle:genesis-freeze` | first rehearsal | Is the selected capability closure ready to become permanent? | genesis-freeze-record | every required entry, input, evidence, oracle, invariant, disabled oracle, and seam resolves | missing closure, cycle, or stage-level selection | `test:genesis-freeze` |
| `handoff:whole-system-rehearsal` | `lifecycle:whole-system-rehearsal` | `owner:operator` | `evidence:rehearsal-report` | `oracle:whole-system-rehearsal`, `oracle:rehearsal-delete-replay`, `oracle:rehearsal-restore-ledger` | first real genesis | Does a fresh instance satisfy the frozen contract under ordinary and faulted operation? | disposable instances and rehearsal report | all selected workflows and disabled checks match | any divergence, restore fault, or budget failure | `test:rehearsal-complete-path`, `test:rehearsal-delete-replay`, `test:rehearsal-restore-ledger` |
| `handoff:first-real-genesis` | `lifecycle:first-real-genesis` | `owner:operator` | `evidence:freeze-record`, `evidence:rehearsal-report` | `oracle:first-real-genesis`, `oracle:genesis-readiness` | permanence boundary | Do deployed inputs exactly match the approved candidate? | signed first-real-genesis-manifest | exact versions, digests, readiness, and configuration | any mismatch or stale evidence | `test:genesis-manifest-verify`, `test:genesis-readiness` |

Every activation gate has a corresponding `handoff:gate-<capability-name>` row with the gate owner, `evidence:gate-<capability-name>`, `oracle:gate-<capability-name>`, `test:gate-<capability-name>`, and the gate's independent blocking scope. The handoff row records the gate's exact required inputs, disabled oracle, additive seam, failure isolation, and retained evidence from the activation-gate register. The worker, maintenance, drift, exception, exploration, proactive-initiation, and subagent rows are included in this rule.

## Executable register

The executable register has one row per `test:<name>` or `measurement:<name>`. Each row names the exact command or harness artefact and one owning handoff ID.

| executable_id | command or named harness artefact | owning_handoff_id |
|---|---|---|
| `test:stage0-contract` | `perm-candidate-inventory`, `selection-schema-audit`, and `capability-census-audit` | `handoff:stage0` |
| `measurement:1a-episode-ablation` | `measure-episode-ablation` | `handoff:stage1` |
| `measurement:1b-witness` | `measure-witness-assurance` | `handoff:stage1` |
| `test:1c-contract` | `measure-extraction-contract` | `handoff:stage1` |
| `measurement:1c-measurement` | `measure-extraction-convergence` | `handoff:stage1` |
| `measurement:1d-budgets` | `measure-storage-budget` | `handoff:stage1` |
| `test:1e-query-classification` | `measure-query-classification` | `handoff:stage1` |
| `measurement:1f-multimodal-recall` | `measure-multimodal-recall` | `handoff:stage1` |
| `test:stage2-reference` | `model-fixture-compile`, `model-reference-fold`, `model-replay-digest`, `model-audience-probes`, and `model-transition-properties` | `handoff:stage2` |
| `test:stage3-source-activity` | `source-activity-substrate` | `handoff:stage3` |
| `test:stage4-assertion-definition` | `assertion-definition-substrate` | `handoff:stage4` |
| `test:stage5-audience-influence` | `audience-influence-substrate` | `handoff:stage5` |
| `test:stage5-status-freshness` | `status-freshness-replay` | `handoff:stage5` |
| `test:stage6-erasure-closure` | `genesis-managed-erasure-closure`, `genesis-zero-residue`, `erase-one-reference-retain-another`, and `pending-live-blob-deletion` | `handoff:stage6` |
| `test:stage6-restore-lock` | `restore-old-backup-current-ledger`, `restore-missing-current-ledger`, `privacy-restore-writer-lock`, and `external-copy-bounded` | `handoff:stage6` |
| `test:stage7-vertical-slice` | `write-fidelity`, `write-constraint-tax`, `write-source-fallback`, and `write-fault-injection` | `handoff:stage7` |
| `test:stage7-authority` | `stage7-authority-gate` | `handoff:stage7` |
| `test:stage8-jobs` | `job-crash-race`, `job-retry-poison`, `job-stale-head`, and `job-authority` | `handoff:stage8` |
| `test:stage8-disabled-worker` | `disabled-worker-zero-scheduling`, `disabled-worker-pending-state`, and `disabled-worker-additive-activation` | `handoff:stage8` |
| `test:stage9-temporal-policy` | `time-dated-no-fire`, `time-correction`, `time-vague-timezone`, and `time-recurrence-properties` | `handoff:stage9` |
| `test:stage10-event-policy` | `event-distinct`, `event-merge-sever`, `event-partial-projection`, and `schema-historical-replay` | `handoff:stage10` |
| `test:stage11-identity-policy` | `identity-overlap`, `identity-disclosure`, `identity-merge-sever`, and `identity-one-handle` | `handoff:stage11` |
| `test:stage12-support-policy` | `support-hidden-zero`, `support-dependence`, `support-last-withdrawal`, and `support-contest` | `handoff:stage12` |
| `test:genesis-freeze` | `freeze-inventory-complete`, `freeze-selection-closure`, `freeze-oracle-map`, `freeze-disabled-seams`, and `freeze-dependency-dag` | `handoff:genesis-freeze` |
| `test:rehearsal-complete-path` | `rehearsal-complete-path` | `handoff:whole-system-rehearsal` |
| `test:rehearsal-delete-replay` | `rehearsal-delete-replay` | `handoff:whole-system-rehearsal` |
| `test:rehearsal-restore-ledger` | `rehearsal-restore-ledger` | `handoff:whole-system-rehearsal` |
| `test:genesis-manifest-verify` | `genesis-manifest-verify` | `handoff:first-real-genesis` |
| `test:genesis-readiness` | `genesis-readiness` | `handoff:first-real-genesis` |
| `test:gate-generated-episodes` | `gate-generated-episodes` | `handoff:gate-generated-episodes` |
| `test:disabled-generated-episodes` | `disabled-generated-episodes` | `handoff:gate-generated-episodes` |
| `test:gate-bulk-ingestion` | `gate-bulk-ingestion` | `handoff:gate-bulk-ingestion` |
| `test:disabled-bulk-ingestion` | `disabled-bulk-ingestion` | `handoff:gate-bulk-ingestion` |
| `test:gate-procedural-memory` | `gate-procedural-memory` | `handoff:gate-procedural-memory` |
| `test:disabled-procedural-memory` | `disabled-procedural-memory` | `handoff:gate-procedural-memory` |
| `test:gate-working-memory` | `gate-working-memory` | `handoff:gate-working-memory` |
| `test:disabled-working-memory` | `disabled-working-memory` | `handoff:gate-working-memory` |
| `test:gate-historical-reinspection` | `gate-historical-reinspection` | `handoff:gate-historical-reinspection` |
| `test:disabled-historical-reinspection` | `disabled-historical-reinspection` | `handoff:gate-historical-reinspection` |
| `test:gate-ocr` | `gate-ocr` | `handoff:gate-ocr` |
| `test:disabled-ocr` | `disabled-ocr` | `handoff:gate-ocr` |
| `test:gate-generated-captions` | `gate-generated-captions` | `handoff:gate-generated-captions` |
| `test:disabled-generated-captions` | `disabled-generated-captions` | `handoff:gate-generated-captions` |
| `test:gate-region-grounding` | `gate-region-grounding` | `handoff:gate-region-grounding` |
| `test:disabled-region-grounding` | `disabled-region-grounding` | `handoff:gate-region-grounding` |
| `test:gate-visual-retrieval` | `gate-visual-retrieval` | `handoff:gate-visual-retrieval` |
| `test:disabled-visual-retrieval` | `disabled-visual-retrieval` | `handoff:gate-visual-retrieval` |
| `test:gate-scene-graph-writer` | `gate-scene-graph-writer` | `handoff:gate-scene-graph-writer` |
| `test:disabled-scene-graph-writer` | `disabled-scene-graph-writer` | `handoff:gate-scene-graph-writer` |
| `test:gate-rich-recurrence` | `gate-rich-recurrence` | `handoff:gate-rich-recurrence` |
| `test:disabled-rich-recurrence` | `disabled-rich-recurrence` | `handoff:gate-rich-recurrence` |
| `test:gate-business-calendar-adjustment` | `gate-business-calendar-adjustment` | `handoff:gate-business-calendar-adjustment` |
| `test:disabled-business-calendar-adjustment` | `disabled-business-calendar-adjustment` | `handoff:gate-business-calendar-adjustment` |
| `test:gate-volatility-automation` | `gate-volatility-automation` | `handoff:gate-volatility-automation` |
| `test:disabled-volatility-automation` | `disabled-volatility-automation` | `handoff:gate-volatility-automation` |
| `test:gate-habitual-deontic-inference` | `gate-habitual-deontic-inference` | `handoff:gate-habitual-deontic-inference` |
| `test:disabled-habitual-deontic-inference` | `disabled-habitual-deontic-inference` | `handoff:gate-habitual-deontic-inference` |
| `test:gate-qualitative-temporal-inference` | `gate-qualitative-temporal-inference` | `handoff:gate-qualitative-temporal-inference` |
| `test:disabled-qualitative-temporal-inference` | `disabled-qualitative-temporal-inference` | `handoff:gate-qualitative-temporal-inference` |
| `test:gate-autonomous-recurrence-interpretation` | `gate-autonomous-recurrence-interpretation` | `handoff:gate-autonomous-recurrence-interpretation` |
| `test:disabled-autonomous-recurrence-interpretation` | `disabled-autonomous-recurrence-interpretation` | `handoff:gate-autonomous-recurrence-interpretation` |
| `test:gate-broad-event-role-vocabulary` | `gate-broad-event-role-vocabulary` | `handoff:gate-broad-event-role-vocabulary` |
| `test:disabled-broad-event-role-vocabulary` | `disabled-broad-event-role-vocabulary` | `handoff:gate-broad-event-role-vocabulary` |
| `test:gate-autonomous-event-merging` | `gate-autonomous-event-merging` | `handoff:gate-autonomous-event-merging` |
| `test:disabled-autonomous-event-merging` | `disabled-autonomous-event-merging` | `handoff:gate-autonomous-event-merging` |
| `test:gate-numeric-support` | `gate-numeric-support` | `handoff:gate-numeric-support` |
| `test:disabled-numeric-support` | `disabled-numeric-support` | `handoff:gate-numeric-support` |
| `test:gate-support-fusion` | `gate-support-fusion` | `handoff:gate-support-fusion` |
| `test:disabled-support-fusion` | `disabled-support-fusion` | `handoff:gate-support-fusion` |
| `test:gate-autonomous-contradiction-arbitration` | `gate-autonomous-contradiction-arbitration` | `handoff:gate-autonomous-contradiction-arbitration` |
| `test:disabled-autonomous-contradiction-arbitration` | `disabled-autonomous-contradiction-arbitration` | `handoff:gate-autonomous-contradiction-arbitration` |
| `test:gate-autonomous-identity` | `gate-autonomous-identity` | `handoff:gate-autonomous-identity` |
| `test:disabled-autonomous-identity` | `disabled-autonomous-identity` | `handoff:gate-autonomous-identity` |
| `test:gate-cross-instance-identity` | `gate-cross-instance-identity` | `handoff:gate-cross-instance-identity` |
| `test:disabled-cross-instance-identity` | `disabled-cross-instance-identity` | `handoff:gate-cross-instance-identity` |
| `test:gate-transmission-reciprocity` | `gate-transmission-reciprocity` | `handoff:gate-transmission-reciprocity` |
| `test:disabled-transmission-reciprocity` | `disabled-transmission-reciprocity` | `handoff:gate-transmission-reciprocity` |
| `test:gate-transmission-purpose-limitation` | `gate-transmission-purpose-limitation` | `handoff:gate-transmission-purpose-limitation` |
| `test:disabled-transmission-purpose-limitation` | `disabled-transmission-purpose-limitation` | `handoff:gate-transmission-purpose-limitation` |
| `test:gate-remote-actionable-status` | `gate-remote-actionable-status` | `handoff:gate-remote-actionable-status` |
| `test:disabled-remote-actionable-status` | `disabled-remote-actionable-status` | `handoff:gate-remote-actionable-status` |
| `test:gate-general-runtime-faithfulness-checking` | `gate-general-runtime-faithfulness-checking` | `handoff:gate-general-runtime-faithfulness-checking` |
| `test:disabled-general-runtime-faithfulness-checking` | `disabled-general-runtime-faithfulness-checking` | `handoff:gate-general-runtime-faithfulness-checking` |
| `test:gate-worker-policy` | `gate-worker-policy` | `handoff:gate-worker-policy` |
| `test:disabled-worker-policy` | `disabled-worker-policy` | `handoff:gate-worker-policy` |
| `test:gate-maintenance-retirement` | `gate-maintenance-retirement` | `handoff:gate-maintenance-retirement` |
| `test:disabled-maintenance-retirement` | `disabled-maintenance-retirement` | `handoff:gate-maintenance-retirement` |
| `test:gate-drift-response` | `gate-drift-response` | `handoff:gate-drift-response` |
| `test:disabled-drift-response` | `disabled-drift-response` | `handoff:gate-drift-response` |
| `test:gate-exception-processing` | `gate-exception-processing` | `handoff:gate-exception-processing` |
| `test:disabled-exception-processing` | `disabled-exception-processing` | `handoff:gate-exception-processing` |
| `test:gate-exploration` | `gate-exploration` | `handoff:gate-exploration` |
| `test:disabled-exploration` | `disabled-exploration` | `handoff:gate-exploration` |
| `test:gate-proactive-initiation` | `gate-proactive-initiation` | `handoff:gate-proactive-initiation` |
| `test:disabled-proactive-initiation` | `disabled-proactive-initiation` | `handoff:gate-proactive-initiation` |
| `test:gate-subagent-spawning` | `gate-subagent-spawning` | `handoff:gate-subagent-spawning` |
| `test:disabled-subagent-spawning` | `disabled-subagent-spawning` | `handoff:gate-subagent-spawning` |
| `test:oracle-seed-definition-set` | `seed-definition-set` | `handoff:oracle-seed-definition-set` |

Each activation-gate executable measures the gate-specific retrieval or operational value, cost, latency, grounding, correction, erasure, and privacy thresholds. Each disabled-behaviour executable checks zero active effect, preserved required state, and the additive activation seam. A test or measurement belongs to one oracle only, except where a named composite oracle explicitly lists member oracle IDs and has its own composite test.

## Oracle register

The oracle register has one row per oracle. An oracle has one owner, one capability or stage, one blocking scope, exactly one handoff ID, and exactly one executable test or measurement ID. A composite oracle lists member oracle IDs and has one composite executable ID and one handoff ID. No test or measurement satisfies two unrelated oracle IDs.

| oracle_id | owner_id | capability_or_stage_id | blocking scope | handoff_id | executable_id |
|---|---|---|---|---|---|
| `oracle:stage0-contract` | `owner:architecture` | `stage:0` | all stages and freeze | `handoff:stage0` | `test:stage0-contract` |
| `oracle:stage1-baseline` | `owner:model-eval` | `stage:1` | consuming nodes | `handoff:stage1` | `test:1c-contract` |
| `oracle:1a-episode-ablation` | `owner:model-eval` | `stage:1` | `gate:generated-episodes` | `handoff:stage1` | `measurement:1a-episode-ablation` |
| `oracle:1b-witness` | `owner:security` | `stage:1` | stage 5, stage 7, and freeze | `handoff:stage1` | `measurement:1b-witness` |
| `oracle:1c-measurement` | `owner:model-eval` | `stage:1` | stage 7 authoritative writes | `handoff:stage1` | `measurement:1c-measurement` |
| `oracle:1d-budgets` | `owner:console` | `stage:1` | stage 6 and freeze | `handoff:stage1` | `measurement:1d-budgets` |
| `oracle:1e-query-classification` | `owner:architecture` | `stage:1` | stage 7 and freeze | `handoff:stage1` | `test:1e-query-classification` |
| `oracle:1f-multimodal-recall` | `owner:model-eval` | `stage:1` | optional multimodal gates | `handoff:stage1` | `measurement:1f-multimodal-recall` |
| `oracle:stage2-reference` | `owner:architecture` | `stage:2` | all later nodes | `handoff:stage2` | `test:stage2-reference` |
| `oracle:stage3-source-activity` | `owner:architecture` | `stage:3` | later substrate | `handoff:stage3` | `test:stage3-source-activity` |
| `oracle:stage4-assertion-definition` | `owner:schema` | `stage:4` | later policy | `handoff:stage4` | `test:stage4-assertion-definition` |
| `oracle:stage5-audience-influence` | `owner:security` | `stage:5` | reads and writes | `handoff:stage5` | `test:stage5-audience-influence` |
| `oracle:stage5-status-freshness` | `owner:connector` | `stage:5` | genesis and `gate:remote-actionable-status` | `handoff:stage5` | `test:stage5-status-freshness` |
| `oracle:stage6-erasure-recovery` | `owner:storage` | `stage:6` | freeze and genesis | `handoff:stage6` | `test:stage6-erasure-closure` |
| `oracle:stage6-restore-lock` | `owner:storage` | `stage:6` | freeze and genesis | `handoff:stage6` | `test:stage6-restore-lock` |
| `oracle:stage7-authority` | `owner:architecture` | `stage:7` | authoritative writes | `handoff:stage7` | `test:stage7-authority` |
| `oracle:stage7-vertical-slice` | `owner:architecture` | `stage:7` | stage 8 and initial-policy evidence | `handoff:stage7` | `test:stage7-vertical-slice` |
| `oracle:stage8-operational-substrate` | `owner:operations` | `stage:8` | freeze and job gates | `handoff:stage8` | `test:stage8-jobs` |
| `oracle:stage8-disabled-worker` | `owner:operations` | `stage:8` | freeze and optional worker gates | `handoff:stage8` | `test:stage8-disabled-worker` |
| `oracle:stage9-temporal-policy` | `owner:temporal-policy` | `stage:9` | selected temporal policy | `handoff:stage9` | `test:stage9-temporal-policy` |
| `oracle:stage10-event-policy` | `owner:schema` | `stage:10` | selected Event policy | `handoff:stage10` | `test:stage10-event-policy` |
| `oracle:stage11-identity-policy` | `owner:identity` | `stage:11` | selected identity policy | `handoff:stage11` | `test:stage11-identity-policy` |
| `oracle:stage12-support-policy` | `owner:belief` | `stage:12` | selected support policy | `handoff:stage12` | `test:stage12-support-policy` |
| `oracle:genesis-freeze` | `owner:operator` | `lifecycle:genesis-freeze` | rehearsal | `handoff:genesis-freeze` | `test:genesis-freeze` |
| `oracle:whole-system-rehearsal` | `owner:operator` | `lifecycle:whole-system-rehearsal` | first genesis | `handoff:whole-system-rehearsal` | `test:rehearsal-complete-path` |
| `oracle:rehearsal-delete-replay` | `owner:operator` | `lifecycle:whole-system-rehearsal` | first genesis | `handoff:whole-system-rehearsal` | `test:rehearsal-delete-replay` |
| `oracle:rehearsal-restore-ledger` | `owner:operator` | `lifecycle:whole-system-rehearsal` | first genesis | `handoff:whole-system-rehearsal` | `test:rehearsal-restore-ledger` |
| `oracle:first-real-genesis` | `owner:operator` | `lifecycle:first-real-genesis` | permanence boundary | `handoff:first-real-genesis` | `test:genesis-manifest-verify` |
| `oracle:genesis-readiness` | `owner:operator` | `lifecycle:first-real-genesis` | permanence boundary | `handoff:first-real-genesis` | `test:genesis-readiness` |
| `oracle:gate-generated-episodes` | `owner:model-eval` | `capability:generated-episodes` | capability activation | `handoff:gate-generated-episodes` | `test:gate-generated-episodes` |
| `oracle:disabled-generated-episodes` | `owner:model-eval` | `capability:generated-episodes` | freeze and rehearsal disablement | `handoff:gate-generated-episodes` | `test:disabled-generated-episodes` |
| `oracle:gate-bulk-ingestion` | `owner:ingestion` | `capability:bulk-ingestion` | capability activation | `handoff:gate-bulk-ingestion` | `test:gate-bulk-ingestion` |
| `oracle:disabled-bulk-ingestion` | `owner:ingestion` | `capability:bulk-ingestion` | freeze and rehearsal disablement | `handoff:gate-bulk-ingestion` | `test:disabled-bulk-ingestion` |
| `oracle:gate-procedural-memory` | `owner:ingestion` | `capability:procedural-memory` | capability activation | `handoff:gate-procedural-memory` | `test:gate-procedural-memory` |
| `oracle:disabled-procedural-memory` | `owner:ingestion` | `capability:procedural-memory` | freeze and rehearsal disablement | `handoff:gate-procedural-memory` | `test:disabled-procedural-memory` |
| `oracle:gate-working-memory` | `owner:ingestion` | `capability:working-memory` | capability activation | `handoff:gate-working-memory` | `test:gate-working-memory` |
| `oracle:disabled-working-memory` | `owner:ingestion` | `capability:working-memory` | freeze and rehearsal disablement | `handoff:gate-working-memory` | `test:disabled-working-memory` |
| `oracle:gate-historical-reinspection` | `owner:model-eval` | `capability:historical-reinspection` | capability activation | `handoff:gate-historical-reinspection` | `test:gate-historical-reinspection` |
| `oracle:disabled-historical-reinspection` | `owner:model-eval` | `capability:historical-reinspection` | freeze and rehearsal disablement | `handoff:gate-historical-reinspection` | `test:disabled-historical-reinspection` |
| `oracle:gate-ocr` | `owner:model-eval` | `capability:ocr` | capability activation | `handoff:gate-ocr` | `test:gate-ocr` |
| `oracle:disabled-ocr` | `owner:model-eval` | `capability:ocr` | freeze and rehearsal disablement | `handoff:gate-ocr` | `test:disabled-ocr` |
| `oracle:gate-generated-captions` | `owner:model-eval` | `capability:generated-captions` | capability activation | `handoff:gate-generated-captions` | `test:gate-generated-captions` |
| `oracle:disabled-generated-captions` | `owner:model-eval` | `capability:generated-captions` | freeze and rehearsal disablement | `handoff:gate-generated-captions` | `test:disabled-generated-captions` |
| `oracle:gate-region-grounding` | `owner:model-eval` | `capability:region-grounding` | capability activation | `handoff:gate-region-grounding` | `test:gate-region-grounding` |
| `oracle:disabled-region-grounding` | `owner:model-eval` | `capability:region-grounding` | freeze and rehearsal disablement | `handoff:gate-region-grounding` | `test:disabled-region-grounding` |
| `oracle:gate-visual-retrieval` | `owner:model-eval` | `capability:visual-retrieval` | capability activation | `handoff:gate-visual-retrieval` | `test:gate-visual-retrieval` |
| `oracle:disabled-visual-retrieval` | `owner:model-eval` | `capability:visual-retrieval` | freeze and rehearsal disablement | `handoff:gate-visual-retrieval` | `test:disabled-visual-retrieval` |
| `oracle:gate-scene-graph-writer` | `owner:model-eval` | `capability:scene-graph-writer` | capability activation | `handoff:gate-scene-graph-writer` | `test:gate-scene-graph-writer` |
| `oracle:disabled-scene-graph-writer` | `owner:model-eval` | `capability:scene-graph-writer` | freeze and rehearsal disablement | `handoff:gate-scene-graph-writer` | `test:disabled-scene-graph-writer` |
| `oracle:gate-rich-recurrence` | `owner:temporal-policy` | `capability:rich-recurrence` | capability activation | `handoff:gate-rich-recurrence` | `test:gate-rich-recurrence` |
| `oracle:disabled-rich-recurrence` | `owner:temporal-policy` | `capability:rich-recurrence` | freeze and rehearsal disablement | `handoff:gate-rich-recurrence` | `test:disabled-rich-recurrence` |
| `oracle:gate-business-calendar-adjustment` | `owner:temporal-policy` | `capability:business-calendar-adjustment` | capability activation | `handoff:gate-business-calendar-adjustment` | `test:gate-business-calendar-adjustment` |
| `oracle:disabled-business-calendar-adjustment` | `owner:temporal-policy` | `capability:business-calendar-adjustment` | freeze and rehearsal disablement | `handoff:gate-business-calendar-adjustment` | `test:disabled-business-calendar-adjustment` |
| `oracle:gate-volatility-automation` | `owner:temporal-policy` | `capability:volatility-automation` | capability activation | `handoff:gate-volatility-automation` | `test:gate-volatility-automation` |
| `oracle:disabled-volatility-automation` | `owner:temporal-policy` | `capability:volatility-automation` | freeze and rehearsal disablement | `handoff:gate-volatility-automation` | `test:disabled-volatility-automation` |
| `oracle:gate-habitual-deontic-inference` | `owner:temporal-policy` | `capability:habitual-deontic-inference` | capability activation | `handoff:gate-habitual-deontic-inference` | `test:gate-habitual-deontic-inference` |
| `oracle:disabled-habitual-deontic-inference` | `owner:temporal-policy` | `capability:habitual-deontic-inference` | freeze and rehearsal disablement | `handoff:gate-habitual-deontic-inference` | `test:disabled-habitual-deontic-inference` |
| `oracle:gate-qualitative-temporal-inference` | `owner:temporal-policy` | `capability:qualitative-temporal-inference` | capability activation | `handoff:gate-qualitative-temporal-inference` | `test:gate-qualitative-temporal-inference` |
| `oracle:disabled-qualitative-temporal-inference` | `owner:temporal-policy` | `capability:qualitative-temporal-inference` | freeze and rehearsal disablement | `handoff:gate-qualitative-temporal-inference` | `test:disabled-qualitative-temporal-inference` |
| `oracle:gate-autonomous-recurrence-interpretation` | `owner:temporal-policy` | `capability:autonomous-recurrence-interpretation` | capability activation | `handoff:gate-autonomous-recurrence-interpretation` | `test:gate-autonomous-recurrence-interpretation` |
| `oracle:disabled-autonomous-recurrence-interpretation` | `owner:temporal-policy` | `capability:autonomous-recurrence-interpretation` | freeze and rehearsal disablement | `handoff:gate-autonomous-recurrence-interpretation` | `test:disabled-autonomous-recurrence-interpretation` |
| `oracle:gate-broad-event-role-vocabulary` | `owner:schema` | `capability:broad-event-role-vocabulary` | capability activation | `handoff:gate-broad-event-role-vocabulary` | `test:gate-broad-event-role-vocabulary` |
| `oracle:disabled-broad-event-role-vocabulary` | `owner:schema` | `capability:broad-event-role-vocabulary` | freeze and rehearsal disablement | `handoff:gate-broad-event-role-vocabulary` | `test:disabled-broad-event-role-vocabulary` |
| `oracle:gate-autonomous-event-merging` | `owner:schema` | `capability:autonomous-event-merging` | capability activation | `handoff:gate-autonomous-event-merging` | `test:gate-autonomous-event-merging` |
| `oracle:disabled-autonomous-event-merging` | `owner:schema` | `capability:autonomous-event-merging` | freeze and rehearsal disablement | `handoff:gate-autonomous-event-merging` | `test:disabled-autonomous-event-merging` |
| `oracle:gate-numeric-support` | `owner:belief` | `capability:numeric-support` | capability activation | `handoff:gate-numeric-support` | `test:gate-numeric-support` |
| `oracle:disabled-numeric-support` | `owner:belief` | `capability:numeric-support` | freeze and rehearsal disablement | `handoff:gate-numeric-support` | `test:disabled-numeric-support` |
| `oracle:gate-support-fusion` | `owner:belief` | `capability:support-fusion` | capability activation | `handoff:gate-support-fusion` | `test:gate-support-fusion` |
| `oracle:disabled-support-fusion` | `owner:belief` | `capability:support-fusion` | freeze and rehearsal disablement | `handoff:gate-support-fusion` | `test:disabled-support-fusion` |
| `oracle:gate-autonomous-contradiction-arbitration` | `owner:belief` | `capability:autonomous-contradiction-arbitration` | capability activation | `handoff:gate-autonomous-contradiction-arbitration` | `test:gate-autonomous-contradiction-arbitration` |
| `oracle:disabled-autonomous-contradiction-arbitration` | `owner:belief` | `capability:autonomous-contradiction-arbitration` | freeze and rehearsal disablement | `handoff:gate-autonomous-contradiction-arbitration` | `test:disabled-autonomous-contradiction-arbitration` |
| `oracle:gate-autonomous-identity` | `owner:identity` | `capability:autonomous-identity` | capability activation | `handoff:gate-autonomous-identity` | `test:gate-autonomous-identity` |
| `oracle:disabled-autonomous-identity` | `owner:identity` | `capability:autonomous-identity` | freeze and rehearsal disablement | `handoff:gate-autonomous-identity` | `test:disabled-autonomous-identity` |
| `oracle:gate-cross-instance-identity` | `owner:identity` | `capability:cross-instance-identity` | capability activation | `handoff:gate-cross-instance-identity` | `test:gate-cross-instance-identity` |
| `oracle:disabled-cross-instance-identity` | `owner:identity` | `capability:cross-instance-identity` | freeze and rehearsal disablement | `handoff:gate-cross-instance-identity` | `test:disabled-cross-instance-identity` |
| `oracle:gate-transmission-reciprocity` | `owner:privacy-policy` | `capability:transmission-reciprocity` | capability activation | `handoff:gate-transmission-reciprocity` | `test:gate-transmission-reciprocity` |
| `oracle:disabled-transmission-reciprocity` | `owner:privacy-policy` | `capability:transmission-reciprocity` | freeze and rehearsal disablement | `handoff:gate-transmission-reciprocity` | `test:disabled-transmission-reciprocity` |
| `oracle:gate-transmission-purpose-limitation` | `owner:privacy-policy` | `capability:transmission-purpose-limitation` | capability activation | `handoff:gate-transmission-purpose-limitation` | `test:gate-transmission-purpose-limitation` |
| `oracle:disabled-transmission-purpose-limitation` | `owner:privacy-policy` | `capability:transmission-purpose-limitation` | freeze and rehearsal disablement | `handoff:gate-transmission-purpose-limitation` | `test:disabled-transmission-purpose-limitation` |
| `oracle:gate-remote-actionable-status` | `owner:connector` | `capability:remote-actionable-status` | capability activation | `handoff:gate-remote-actionable-status` | `test:gate-remote-actionable-status` |
| `oracle:disabled-remote-actionable-status` | `owner:connector` | `capability:remote-actionable-status` | freeze and rehearsal disablement | `handoff:gate-remote-actionable-status` | `test:disabled-remote-actionable-status` |
| `oracle:gate-general-runtime-faithfulness-checking` | `owner:model-eval` | `capability:general-runtime-faithfulness-checking` | capability activation | `handoff:gate-general-runtime-faithfulness-checking` | `test:gate-general-runtime-faithfulness-checking` |
| `oracle:disabled-general-runtime-faithfulness-checking` | `owner:model-eval` | `capability:general-runtime-faithfulness-checking` | freeze and rehearsal disablement | `handoff:gate-general-runtime-faithfulness-checking` | `test:disabled-general-runtime-faithfulness-checking` |
| `oracle:gate-worker-policy` | `owner:operations` | `capability:worker-policy` | capability activation | `handoff:gate-worker-policy` | `test:gate-worker-policy` |
| `oracle:disabled-worker-policy` | `owner:operations` | `capability:worker-policy` | freeze and rehearsal disablement | `handoff:gate-worker-policy` | `test:disabled-worker-policy` |
| `oracle:gate-maintenance-retirement` | `owner:operations` | `capability:maintenance-retirement` | capability activation | `handoff:gate-maintenance-retirement` | `test:gate-maintenance-retirement` |
| `oracle:disabled-maintenance-retirement` | `owner:operations` | `capability:maintenance-retirement` | freeze and rehearsal disablement | `handoff:gate-maintenance-retirement` | `test:disabled-maintenance-retirement` |
| `oracle:gate-drift-response` | `owner:operations` | `capability:drift-response` | capability activation | `handoff:gate-drift-response` | `test:gate-drift-response` |
| `oracle:disabled-drift-response` | `owner:operations` | `capability:drift-response` | freeze and rehearsal disablement | `handoff:gate-drift-response` | `test:disabled-drift-response` |
| `oracle:gate-exception-processing` | `owner:operations` | `capability:exception-processing` | capability activation | `handoff:gate-exception-processing` | `test:gate-exception-processing` |
| `oracle:disabled-exception-processing` | `owner:operations` | `capability:exception-processing` | freeze and rehearsal disablement | `handoff:gate-exception-processing` | `test:disabled-exception-processing` |
| `oracle:gate-exploration` | `owner:operations` | `capability:exploration` | capability activation | `handoff:gate-exploration` | `test:gate-exploration` |
| `oracle:disabled-exploration` | `owner:operations` | `capability:exploration` | freeze and rehearsal disablement | `handoff:gate-exploration` | `test:disabled-exploration` |
| `oracle:gate-proactive-initiation` | `owner:operations` | `capability:proactive-initiation` | capability activation | `handoff:gate-proactive-initiation` | `test:gate-proactive-initiation` |
| `oracle:disabled-proactive-initiation` | `owner:operations` | `capability:proactive-initiation` | freeze and rehearsal disablement | `handoff:gate-proactive-initiation` | `test:disabled-proactive-initiation` |
| `oracle:gate-subagent-spawning` | `owner:operations` | `capability:subagent-spawning` | capability activation | `handoff:gate-subagent-spawning` | `test:gate-subagent-spawning` |
| `oracle:disabled-subagent-spawning` | `owner:operations` | `capability:subagent-spawning` | freeze and rehearsal disablement | `handoff:gate-subagent-spawning` | `test:disabled-subagent-spawning` |
| `oracle:seed-definition-set` | `owner:schema` | `stage:4` | genesis | `handoff:oracle-seed-definition-set` | `test:oracle-seed-definition-set` |

Each gate has an independent activation oracle and a separate disabled-behaviour oracle. Disabled-behaviour oracles are executable members of the corresponding freeze or rehearsal composite.

## Invariant register

The invariant register has these fields: `invariant_id`, `owner_id`, `capability_or_stage_id`, `normative_owner_heading`, `canonical_contract_statement`, `required_clause_ids`, `preservation_obligations`, `required_evidence_ids`, and `oracle_ids`.

| invariant_id | owner_id | capability_or_stage_id | normative_owner_heading | canonical contract statement | required clause IDs | preservation obligations | required evidence IDs | oracle IDs |
|---|---|---|---|---|---|---|---|---|
| `invariant:append-only-replay` | `owner:architecture` | `stage:2` | [Overview](overview.md#permanence-contract) | Replay follows the append-only event history and does not rewrite a committed meaning. | `clause:append-only`, `clause:replay-fold` | later records correct or supersede earlier records without deleting history | `evidence:stage2-reference`, `evidence:stage7-vertical-slice` | `oracle:stage2-reference`, `oracle:stage7-authority` |
| `invariant:no-invented-history` | `owner:schema` | `capability:assertion-definition-substrate` | [Assertions](statements.md) | Replay never invents a value that the recorded source and transitions omit. | `clause:no-invented-value`, `clause:historical-definition` | correction, modality, definition, and upcast paths preserve omission | `evidence:stage4-assertion-definition` | `oracle:stage4-assertion-definition` |
| `invariant:source-only-fallback` | `owner:architecture` | `capability:read-write-vertical-slice` | [Verified write](verified-write.md) | Source content remains available when extraction, grounding, critics, or optional interpretation fail. | `clause:source-retention`, `clause:critic-failure` | no structured proposal replaces the source without atomic publication | `evidence:stage3-source-activity`, `evidence:stage7-vertical-slice` | `oracle:stage3-source-activity`, `oracle:stage7-authority` |
| `invariant:central-audience-no-residue` | `owner:security` | `capability:audience-influence-substrate` | [Privacy and provenance](privacy-and-provenance.md) | Central audience resolution prevents hidden inputs from changing visible output and leaves zero prohibited residue. | `clause:central-resolution`, `clause:zero-residue`, `clause:teller-fallback` | denial does not disclose hidden membership, support, or existence | `evidence:stage5-audience-influence` | `oracle:stage5-audience-influence` |
| `invariant:terminal-erasure` | `owner:storage` | `capability:artefact-erasure-recovery-substrate` | [Privacy and provenance](privacy-and-provenance.md) | Terminal erasure closes over every managed-live surface and prevents erased data from regaining authority. | `clause:five-live-surfaces`, `clause:pending-deletion`, `clause:dependent-invalidation` | shared references retain only the still-authorised occurrence | `evidence:stage6-erasure-recovery` | `oracle:stage6-erasure-recovery` |
| `invariant:restore-current-ledger` | `owner:storage` | `capability:artefact-erasure-recovery-substrate` | [Privacy and provenance](privacy-and-provenance.md) | Restore filters old material against a verified current authoritative ledger while the writer lock excludes publication and serving remains blocked. | `clause:restore-ledger`, `clause:restore-lock`, `clause:restore-blocked` | unverifiable authority never opens serving | `evidence:stage6-erasure-recovery` | `oracle:stage6-erasure-recovery`, `oracle:stage6-restore-lock` |
| `invariant:stable-source-identity` | `owner:architecture` | `capability:source-activity-substrate` | [Artefacts and perceptions](artefacts-and-perceptions.md) | Source and Artefact identities remain stable across replay, projection rebuild, and optional interpretation. | `clause:stable-source-id`, `clause:source-locator` | derived records never replace their source identity | `evidence:stage3-source-activity`, `evidence:stage6-erasure-recovery` | `oracle:stage3-source-activity`, `oracle:stage6-erasure-recovery` |
| `invariant:identity-reversibility` | `owner:identity` | `capability:initial-identity-policy` | [Identity](identity.md) | Merge and severance preserve exact source histories and invalidate dependent projections. | `clause:disjoint-composite`, `clause:severance-replay`, `clause:dependant-invalidation` | a tentative composite cannot affect a response without disclosure clearance | `evidence:stage11-identity-policy` | `oracle:stage11-identity-policy` |
| `invariant:typed-temporal-safety` | `owner:temporal-policy` | `capability:initial-temporal-policy` | [Time](time.md) | Descriptive dates do not arm Triggers, and temporal correction preserves source records. | `clause:occurrence-task-trigger`, `clause:dated-no-fire`, `clause:temporal-correction` | unknown time remains explicit rather than guessed | `evidence:stage9-temporal-policy` | `oracle:stage9-temporal-policy` |
| `invariant:definition-replay` | `owner:schema` | `capability:initial-event-role-relation-policy` | [Events and roles](events-and-roles.md) | A historical record replays under the definition version recorded for that record. | `clause:definition-id`, `clause:definition-version`, `clause:schema-governance` | later definitions do not reinterpret earlier records | `evidence:stage4-assertion-definition`, `evidence:stage10-event-policy` | `oracle:stage4-assertion-definition`, `oracle:stage10-event-policy` |
| `invariant:conservative-support` | `owner:belief` | `capability:initial-support-policy` | [Belief](belief.md) | Support remains ordinal, audience-safe, and independent of hidden or dependent evidence. | `clause:ordinal-support`, `clause:dependence-suppression`, `clause:mechanical-contradiction` | unranked Attestations remain visible under the fail-closed fallback | `evidence:stage12-support-policy` | `oracle:stage12-support-policy` |
| `invariant:independent-extension-disablement` | `owner:operations` | `capability:disabled-worker-contract` | [Off-turn work](off-turn.md) | An unselected capability has no active effect, preserves required state, and activates later through an additive versioned seam. | `clause:zero-disabled-execution`, `clause:pending-state`, `clause:additive-activation` | activation cannot reinterpret genesis history | `evidence:stage8-disabled-worker`, `evidence:freeze-record` | `oracle:stage8-disabled-worker`, `oracle:genesis-freeze` |

The required clause IDs remain represented under their normative owner headings. The preservation obligations remain present even when a capability is disabled. Any new invariant receives a stable ID before it enters the freeze record.

## Unresolved and adversarial identifiers

The detailed status and evidence remain in [confidence](confidence.md). The roadmap uses stable IDs for cross-register references.

| identifier family | declared IDs |
|---|---|
| unresolved design items | `unresolved:selector-digest-permanence`, `unresolved:successor-seed-definitions`, `unresolved:connector-witness-assurance`, `unresolved:inter-agent-freshness-record`, `unresolved:remote-actionable-activation`, `unresolved:authoritative-ledger-architecture`, `unresolved:recovery-authority-continuity`, `unresolved:erasure-authority-storage-restore`, `unresolved:job-semantics`, `unresolved:qualitative-temporal-subalgebra`, `unresolved:temporal-uncertainty-recurrence`, `unresolved:event-subrole-governance`, `unresolved:event-coreference-policy`, `unresolved:definition-replay-authority`, `unresolved:identity-thresholds`, `unresolved:support-arithmetic-fusion`, `unresolved:initial-policy-matrix`, `unresolved:generated-episode-encoding-retrieval`, `unresolved:target-model-constraint-tax`, `unresolved:extraction-convergence`, `unresolved:bulk-ingestion-cost-fidelity`, `unresolved:console-fold-log-budget`, `unresolved:eager-lazy-structuring`, `unresolved:referential-frame-principal`, `unresolved:relay-chain-dependence`, `unresolved:structural-query-sufficiency`, `unresolved:exploration-yield`, `unresolved:habitual-modality-policy`, `unresolved:proactive-initiation-salience`, `unresolved:baseline-erasure-closure`, `unresolved:influence-taint`, `unresolved:historical-artefact-reinspection`, `unresolved:multimodal-interpretation`, `unresolved:runtime-faithfulness`, and `unresolved:depth-two-attitudes` |
| adversarial obligations | `obligation:append-only`, `obligation:no-invented-history`, `obligation:source-only-fallback`, `obligation:central-audience-no-residue`, `obligation:five-managed-live-surfaces`, `obligation:restore-writer-lock`, `obligation:pending-deletion-denial`, `obligation:disabled-worker-additive-seam`, `obligation:stable-source-identity`, and `obligation:identity-severance` |

Stable identity and transition-fold references resolve to the existing affected stage, capability, evidence, oracle, and invariant IDs in the registers; they do not introduce generic unresolved IDs. Each unresolved or adversarial row in the owning confidence register maps to exactly one primary numbered stage, capability, or activation gate. Secondary prerequisites are used only when the dependency is real. A genesis-blocking row names exact owner, capability or stage, evidence, and oracle IDs.

## Genesis freeze review

### Prerequisite node IDs

`stage:0`, `stage:1`, `stage:2`, `stage:3`, `stage:4`, `stage:5`, `stage:6`, `stage:7`, `stage:8`, `stage:9`, `stage:10`, `stage:11`, `stage:12`.

### Evidence prerequisite IDs

`evidence:stage0-contract`, `evidence:all-genesis-blockers`, and every evidence ID in the computed selection closure.

### Semantic contracts

The freeze record fixes stable IDs, canonical encodings, event payload meanings, transition folds, selector definitions, storage boundaries, versioning and upcast rules, initial policy versions, required genesis inputs, and the exact versioned genesis-selection-record. It does not select a numbered stage.

The freeze prerequisite is the union of every `required_substrate` capability entry, every `initial_policy` capability entry, their transitive prerequisite capability closure, the transitive prerequisite closure of each selected capability and gate, and every genesis-blocking oracle. An `activation_gate` or `declined` entry requires its disabled-behaviour oracle and additive-seam reference. The activation gate itself does not need to run unless the capability is selected as `initial_policy`.

The freeze rejects missing prerequisites, cycles, duplicate capability IDs, unresolved evidence or oracle IDs, an oracle without exactly one executable handoff test or measurement, an invariant without owner, evidence, and oracle IDs, and a stage-level selection substituted for capability-level closure. The freeze rejects a missing disabled behaviour proof even when an optional capability is declined.

### Owning chapters

[Overview](overview.md#permanence-contract), every normative object owner, [privacy and provenance](privacy-and-provenance.md), [confidence](confidence.md), and [evolution](#programme-taxonomy).

### Evidence produced

`evidence:freeze-record` is the `genesis-freeze-record`. It maps every frozen item to its reference oracle, production-shaped comparison, unresolved-register closure, invariant, versioning rule, named automated test, and required input. `evidence:all-genesis-blockers` records the computed capability closure and every genesis-blocking oracle. The record also lists declined capabilities, disabled-behaviour results, additive seams, and post-genesis activation gates.

### Required decisions

The architecture, security, storage, operations, model and eval, schema, identity, belief, connector, and operator owners approve the complete freeze record. Approval cannot substitute for a missing executable oracle, unresolved genesis input, disabled-behaviour oracle, additive seam, or invariant mapping.

### Falsification and stop conditions

The relevant stage or activation gate returns to revision if any frozen field lacks a source, oracle, versioning rule, restore or erasure consequence, cost bound, or replay proof. A revision may break all pre-genesis state and invalidates the proposed freeze record.

### Experimental-state disposition

Prior experimental stores are disposable after evidence inputs, expected results, measurements, failures, and rationales are preserved. The selected canonical fixture corpus and reference outputs are retained as the frozen implementation oracle.

### Blocking scope

The freeze blocks the whole-system rehearsal and first real genesis. It does not require an unselected activation gate to pass.

### Deferred

Only explicitly named post-genesis extensions and declined capabilities remain deferred.

## Whole-system rehearsal

### Prerequisite node IDs

`lifecycle:genesis-freeze`.

### Evidence prerequisite IDs

`evidence:freeze-record`, `evidence:all-genesis-blockers`.

### Semantic contracts

The rehearsal creates fresh disposable instances from the frozen candidate. It exercises connector input, source recording, verified writes, audience-resolved reads, correction, retraction, identity and Event severance, temporal safety, attachments, scheduling, erasure, crash recovery, projection deletion, replay, and cost budgets.

The rehearsal matrix is generated from the versioned selection record and its transitive capability dependency closure. Selected initial capabilities run as active behaviour. Required substrate capabilities always run. Unselected activation-gate and declined capabilities are checked for disabled behaviour, preserved pending state where applicable, zero scheduling or execution, and the additive seam. The rehearsal does not exercise unselected capability behaviour as active behaviour.

Every restore acquires the single-writer lock, verifies the current authoritative ledger, filters and rebuilds while serving is blocked, opens serving, and then releases the lock. `restore-old-backup-current-ledger` represents the reachable filtered outcome. `restore-missing-current-ledger` represents unverifiable current authority and remains blocked.

### Owning chapters

All normative chapters, with [privacy and provenance](privacy-and-provenance.md) owning restore and erasure semantics and [evolution](#executable-handoff-register) owning the rehearsal gate.

### Evidence produced

`evidence:rehearsal-report` contains fresh-instance transcripts, reference comparisons, replay digests, crash schedules, restore schedules, erasure outcomes, projection-rebuild results, disabled-capability checks, and measured budgets. It includes `rehearsal-complete-path`, `rehearsal-delete-replay`, and `rehearsal-restore-ledger`.

### Required decisions

The same freeze owners classify every discrepancy as an implementation fault, a specification fault, or a rejected expectation. A specification fault returns to the owning stage or activation gate and reopens the freeze. A disabled-capability failure returns to the capability's gate or its substrate owner.

### Falsification and stop conditions

The design returns to revision if any rehearsal diverges from a structural oracle, reads restore authority before acquiring the writer lock, permits ledger publication while the lock is held, opens serving before filtering and rebuilding complete, releases the lock before serving opens, opens serving without a verifiable current authoritative ledger, loses source or stable identity, violates an audience boundary, exceeds a frozen budget, executes an unselected worker, or cannot recover after projection deletion or crash.

### Experimental-state disposition

Every rehearsal instance, log, payload store, blob store, backup, projection, and snapshot is disposable. Fixture inputs, exact expected results, transcripts, measurements, failures, and decision rationales are retained as evidence.

### Blocking scope

The rehearsal blocks first real genesis. A failed disabled-capability check blocks genesis because the freeze cannot prove independent disablement. A failed active optional capability blocks genesis only when the selection record selected it as `initial_policy`.

### Deferred

No required genesis capability may be deferred past a failed rehearsal. Named post-genesis extensions remain disabled.

## First real genesis

### Prerequisite node IDs

`lifecycle:whole-system-rehearsal`.

### Evidence prerequisite IDs

`evidence:freeze-record`, `evidence:rehearsal-report`.

### Semantic contracts

The first real successor instance is created from the approved freeze record and rehearsal result. The post-genesis permanence contract begins at this boundary. Persisted meanings and stable identities cannot change incompatibly. Replay cannot invent omitted values. Later capabilities use additive versioned mechanisms.

### Owning chapters

[Overview](overview.md#permanence-contract) owns permanence. Each canonical chapter owns its frozen object and policy semantics. [Privacy and provenance](privacy-and-provenance.md) owns the restore, erasure, audience, and external-copy contracts.

### Evidence produced

The `first-real-genesis-manifest` records every frozen version, required input, policy version, fixture-corpus digest, reference-oracle digest, selection-record digest, rehearsal report, ledger readiness proof, and deployment configuration needed to reproduce the genesis decision.

### Required decisions

The operator authorises creation only from the approved freeze record and rehearsal result. The manifest records the authorisation, exact genesis inputs, versioned capability selection, and readiness checks.

### Falsification and stop conditions

The instance is not created when any manifest input differs from the frozen candidate, a required service or ledger freshness proof is unavailable, a selected capability lacks its evidence closure, a disabled capability lacks its proof, or rehearsal evidence is stale relative to implementation. After creation, faults use additive correction and cannot return the real instance to experimental compatibility rules.

### Experimental-state disposition

No real genesis state is disposable. Pre-genesis stores remain destroyed. Retained evidence remains an audit input and has no authority over the live instance.

### Blocking scope

First real genesis is the permanence boundary. It is blocked by every required substrate capability, every selected initial-policy capability, their transitive closure, every genesis-blocking oracle, the freeze record, and the rehearsal report.

### Deferred

Only named post-genesis extensions and declined capabilities remain gated. Their activation cannot reinterpret genesis history.

## Handoff requirements

A pre-genesis implementation increment states the affected final contracts and owning chapters, the structured prerequisite node IDs, the evidence prerequisite IDs, the executable evidence artefacts and production/reference comparisons, the generated logs, fixtures, projections, snapshots, blobs, and measurements that are disposable, the fixture inputs and expected results retained as evidence, the falsification conditions and owner to which failure returns, links to unresolved and adversarial IDs, consequences for the genesis-selection-record and capability closure, named automated structural oracles, scenario tests, and preregistered thresholds.

An increment need not preserve an earlier experimental format. Composite benchmark scores are not acceptance targets. Structural behaviour uses deterministic oracles. Model judges remain limited to linguistic behaviour. [`confidence.md`](confidence.md) records the evidence and caveat behind every gate.

| handoff summary | declared unresolved IDs |
|---|---|
| unresolved design items carried into implementation | `unresolved:selector-digest-permanence`, `unresolved:successor-seed-definitions`, `unresolved:connector-witness-assurance`, `unresolved:inter-agent-freshness-record`, `unresolved:remote-actionable-activation`, `unresolved:authoritative-ledger-architecture`, `unresolved:recovery-authority-continuity`, `unresolved:erasure-authority-storage-restore`, `unresolved:job-semantics`, `unresolved:qualitative-temporal-subalgebra`, `unresolved:temporal-uncertainty-recurrence`, `unresolved:event-subrole-governance`, `unresolved:event-coreference-policy`, `unresolved:definition-replay-authority`, `unresolved:identity-thresholds`, `unresolved:support-arithmetic-fusion`, `unresolved:initial-policy-matrix`, `unresolved:generated-episode-encoding-retrieval`, `unresolved:target-model-constraint-tax`, `unresolved:extraction-convergence`, `unresolved:bulk-ingestion-cost-fidelity`, `unresolved:console-fold-log-budget`, `unresolved:eager-lazy-structuring`, `unresolved:referential-frame-principal`, `unresolved:relay-chain-dependence`, `unresolved:structural-query-sufficiency`, `unresolved:exploration-yield`, `unresolved:habitual-modality-policy`, `unresolved:proactive-initiation-salience`, `unresolved:baseline-erasure-closure`, `unresolved:influence-taint`, `unresolved:historical-artefact-reinspection`, `unresolved:multimodal-interpretation`, `unresolved:runtime-faithfulness`, and `unresolved:depth-two-attitudes` |

The implementation handoff audit checks every stage and gate body against its dependency-register row in both directions. It checks every canonical reference against its owning register. It rejects duplicate IDs, orphaned rows, unresolved references, undeclared many-to-one oracle mappings, capability-status occurrences outside the census, and a gate that lacks an independent disabled-behaviour oracle or additive seam.

Experimental contracts do not drain into `docs/` stage by stage. Normative material moves to `docs/` when the genesis freeze makes it part of the as-built successor, or when a post-genesis extension is activated. The future tree remains the design owner until those boundaries.
