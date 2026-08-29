# docs-future

This tree specifies a proposed successor architecture. It does not describe zuihitsu as currently built. Current behavior is documented in [`../docs/`](../docs/).

The chapters use present tense as normative design language. Implementation status is recorded separately in [`coverage.md`](coverage.md), [`confidence.md`](confidence.md), and [`evolution.md`](evolution.md).

## Scope and permanence

The running pre-successor instance is not migrated. The successor starts at genesis. After genesis, persisted meanings and stable identities cannot be changed incompatibly. Later capabilities must be additive and versioned. Replay cannot invent omitted historical values, and no stage can require an already-born successor agent to reset. [`overview.md`](overview.md#permanence-contract) defines the full contract.

Status labels have fixed meanings:

| Status | Meaning |
|---|---|
| `required at genesis` | Permanent substrate or raw input required before the successor runs. |
| `initial policy` | Behavior enabled in the first usable successor. |
| `gated extension` | Later behavior whose required raw inputs exist from genesis. |
| `open experiment` | Uncommitted behavior that requires an evidence gate. |

## Canonical glossary

| Term | Definition and owner |
|---|---|
| Occasion | Durable external social or input event with an optional utterance and zero or more ArtefactReferences. [Assertions](statements.md#occasion-and-activity) owns the definition. |
| Activity | Durable agent, operator, tool, or model action. [Assertions](statements.md#occasion-and-activity) owns the definition. |
| Artefact | Minted identity for one immutable byte sequence, with versioned verified digest assertions for lookup and deduplication. [Artefacts and perceptions](artefacts-and-perceptions.md) owns the definition. |
| ArtefactReference | Occasion-specific act of sharing an Artefact. [Artefacts and perceptions](artefacts-and-perceptions.md) owns the definition. |
| Perception | Versioned fallible model or tool observation of an Artefact or selector. It is not testimony. [Artefacts and perceptions](artefacts-and-perceptions.md) owns the definition. |
| Event | Stable happening identity whose roles and attributes are Assertions. [Events and roles](events-and-roles.md) owns the definition. |
| Task | Authorised agent action intent with an append-only lifecycle. [Time](time.md#occurrence-task-and-trigger) owns the temporal contract. |
| Trigger | A separately minted condition/action binding that can fire only for a live Task. [Time](time.md#occurrence-task-and-trigger) owns the temporal contract. |
| Proposition | Canonical subject, relation, object, frame, polarity, and modality. [Assertions](statements.md#proposition) owns the definition. |
| Assertion | Proposition situated in validity and immutable asserted/quoted mode, with a separate append-only lifecycle. [Assertions](statements.md#assertion) owns the definition. |
| Attestation | One teller's support for an Assertion on one Occasion. [Assertions](statements.md#attestation) owns the definition. |
| Derivation | Immutable result produced by a versioned Activity from typed inputs and explicit resolution, ontology, policy, and implementation versions. [Assertions](statements.md#derivation) owns the definition. |

Earlier dated research uses `Statement` for combinations of Proposition, Assertion, and Attestation. Normative chapters do not use it as a catch-all term. Research snapshots remain historical evidence and are not rewritten.

## Reading order

1. [`overview.md`](overview.md) defines the architecture and permanence contract.
2. [`statements.md`](statements.md) defines the assertion layer, lifecycle, contradiction subset, and source locators.
3. [`artefacts-and-perceptions.md`](artefacts-and-perceptions.md) defines multimodal evidence and records current attachment behavior.
4. [`privacy-and-provenance.md`](privacy-and-provenance.md) defines audience resolution, influence, and erasure.
5. [`evolution.md`](evolution.md) defines the staged research and build order.

The remaining normative chapters apply those definitions:

| Chapter | Subject |
|---|---|
| [`events-and-roles.md`](events-and-roles.md) | Event identity, role Assertions, and reversible co-reference |
| [`relations.md`](relations.md) | Registered definitions and schema evolution |
| [`identity.md`](identity.md) | Platform stubs, merge hypotheses, and resolution environments |
| [`belief.md`](belief.md) | Audience-safe support, dependence, and reliability evidence |
| [`time.md`](time.md) | Assertion validity, Event occurrence, Tasks, Triggers, and recurrence |
| [`two-traces.md`](two-traces.md) | Source material and generated episodic narrative |
| [`memory-typology.md`](memory-typology.md) | Semantic, episodic, procedural, and working lifecycles |
| [`verified-write.md`](verified-write.md) | Append-only proposal and critic transaction |
| [`write-surface.md`](write-surface.md) | Agent-facing write operations |
| [`query-surface.md`](query-surface.md) | Audience-resolved reads and access accounting |
| [`off-turn.md`](off-turn.md) | Event-sourced background jobs and bounded authority |

Supporting registers:

- [`coverage.md`](coverage.md) maps current failures and issues to mechanisms, evidence, gates, and residual risk.
- [`confidence.md`](confidence.md) records genesis blockers, stage gates, deferred work, adversarial obligations, and the evidence map.
- [`lineage.md`](lineage.md) is an ancestry index into the evidence map.
- [`research/`](research/) contains dated evidence snapshots. The [corpus modelling study](research/2026-08-03/modelling-study.md) tests the earlier model against recorded data.

The failure survey remains in current-system documentation because it records observed failures: [`../docs/ontology-failures/2026-07-23.md`](../docs/ontology-failures/2026-07-23.md).