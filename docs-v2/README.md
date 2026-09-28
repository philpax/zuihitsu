# docs-v2

This tree specifies a proposed successor architecture. It does not describe zuihitsu as currently built. Current behaviour is documented in [`../docs/`](../docs/).

The chapters use present tense as normative design language. Each rule is stated once, in the chapter that owns it, and other chapters link to it. Evidence grades and open questions live in [`confidence.md`](program/confidence.md), the mapping from observed failures to mechanisms lives in [`coverage.md`](program/coverage.md), and the milestones towards genesis live in [`evolution.md`](program/evolution.md).

## Scope and permanence

The running instance remains outside the successor boundary. Its agents are frozen until the successor comes online, are never migrated, and are not a compatibility target. The successor starts at a first real genesis.

Before genesis, every successor log, encoding, projection, and fixture is disposable. After genesis, persisted meaning and stable identity never change incompatibly. New capability is additive or arrives through a recorded upcast, an upcast never supplies a value its input lacked, and no change may require resetting an agent born on the successor. A capability is either in the genesis design or deferred. [`overview.md`](overview.md#permanence-contract) defines the full contract and the keep-at-genesis test.

## Canonical glossary

| Term | Definition and owner |
|---|---|
| Occasion | One inbound message or delivery, or one outbound agent utterance, with an ordered sequence of text and ArtefactReference parts, participants with witness evidence, a restriction, an optional `in_reply_to` link, and observed and recorded time. [Object model](statements.md#occasion). |
| Activity | Any recorded agent, operator, tool, or model action, such as a model call or Lua block. [Object model](statements.md#activity). |
| Context manifest | The ordered IDs of every object rendered into one model call's context. Influence, taint, restriction, and access accounting are projections over it. [Object model](statements.md#context-manifest). |
| Entity | Minted identity of a registered, fixed kind for a person, place, or other thing. A connector stub is a person Entity with connector scope. A handle such as `person/rowan` is a mutable label that resolves to the Entity's ULID. [Object model](statements.md#entity). |
| Proposition | Computed key over subject, relation, object, frame, polarity, and modality, holding definition IDs and ULIDs, never versions or handles. [Object model](statements.md#proposition). |
| Assertion | A Proposition situated in typed validity with an immutable `asserted` or `quoted` mode, the definition versions it was accepted under, and a lifecycle that folds per audience. Settlement is a projection, not a state. [Object model](statements.md#assertion). |
| Attestation | One source's support for one Assertion, with the validity its source asserted. Its source is `testimony`, `observation`, or `derivation`; only testimony carries a teller. [Object model](statements.md#attestation). |
| Perception | Fallible output of a model or tool Activity over an ArtefactReference and selector. It is never testimony. [Object model](statements.md#perception) and [artefacts and perceptions](artefacts-and-perceptions.md). |
| Artefact | Minted identity for one immutable byte sequence. Its digest lives in the erasable payload. [Artefacts and perceptions](artefacts-and-perceptions.md). |
| ArtefactReference | One act of sharing an Artefact on one Occasion. [Artefacts and perceptions](artefacts-and-perceptions.md). |
| Selector | Content-keyed address of all or part of one Artefact under a registered definition version. It has no minted ID. [Artefacts and perceptions](artefacts-and-perceptions.md#selectors). |
| Event | Minted identity for a happening whose type, roles, and attributes are Assertions. [Events and roles](events-and-roles.md). |
| Resolution hypothesis | A reversible identity `same_as` proposal over Entities of one kind, including connector stubs. Acceptance mints a separate composite with `recall` or `disclosure` clearance and a primary member. Events have no co-reference mechanism. [Object model](statements.md#resolution-hypothesis) and [identity](identity.md). |
| ResolutionEnvironment | The accepted hypotheses, composites, and policy versions at a recorded frontier, recomputed from the log. It is derived, never stored. [Identity](identity.md#resolution-environments). |
| Task | Agent-authored action intent with one or more trigger conditions that fire only while it is active. [Object model](statements.md#task) and [time](time.md). |
| Frontier | The opaque log position a read or write observed. [Overview](overview.md#distributed-operation). |

The dated snapshots under [`research/`](research/) use `Statement` for combinations of Proposition, Assertion, and Attestation, and they name objects this glossary no longer has, such as Derivation, SourceAuthority, Trigger, and InfluenceEnvelope. They are evidence and are not rewritten.

## Reading order

1. [`overview.md`](overview.md) defines the architecture, the permanence contract, and the distributed-operation constraints.
2. [`statements.md`](statements.md), the object model, defines every object, its legal transitions, context manifests, the contradiction subset, and source locators.
3. [`artefacts-and-perceptions.md`](artefacts-and-perceptions.md) defines Artefacts, references, selectors, and Perceptions.
4. [`privacy-and-provenance.md`](privacy-and-provenance.md) defines audience resolution, influence, and erasure.
5. [`evolution.md`](program/evolution.md) defines the milestones to the first real genesis and the deferred capabilities.

The remaining normative chapters apply those definitions:

| Chapter | Subject |
|---|---|
| [`events-and-roles.md`](events-and-roles.md) | Event identity, role Assertions, and disclosure-safe projection |
| [`relations.md`](relations.md) | Versioned definitions, agent coinage, and schema evolution |
| [`identity.md`](identity.md) | Platform stubs, resolution hypotheses, clearance, and resolution environments |
| [`belief.md`](belief.md) | Audience-safe support and dependence |
| [`time.md`](time.md) | Validity, Event occurrence, Tasks, trigger conditions, and recurrence |
| [`two-traces.md`](two-traces.md) | Source material and generated episodic narrative |
| [`memory-typology.md`](memory-typology.md) | Semantic, episodic, procedural, and working lifecycles |
| [`verified-write.md`](verified-write.md) | Proposal states and critics |
| [`write-surface.md`](write-surface.md) | Agent-facing write operations |
| [`query-surface.md`](query-surface.md) | Audience-resolved reads and access accounting |
| [`off-turn.md`](off-turn.md) | Background jobs and stateless workers |

Supporting registers:

- [`coverage.md`](program/coverage.md) maps current failures and issues to mechanisms, evidence, and residual risk.
- [`confidence.md`](program/confidence.md) holds the evidence map, open questions, and claims deliberately not made.
- [`lineage.md`](program/lineage.md) is an ancestry index into the evidence map.
- [`research/`](research/) contains dated evidence snapshots. The [corpus modelling study](research/2026-08-03/modelling-study.md) tests the earlier model against recorded data.

The failure survey remains in current-system documentation because it records observed failures: [`../docs/ontology-failures/2026-07-23.md`](../docs/ontology-failures/2026-07-23.md).

## Scenario IDs

Each normative chapter ends with a table of scenarios under a heading named "Scenarios". A scenario has a stable kebab-case ID, such as `compound-utterance`, `identity-severance`, or `hidden-support`, a one-line setup, and a one-line expected result that names what is returned, rejected, or recorded. An ID is never reused for a different scenario.

A future Rust test that exercises a scenario carries its ID. A CI check reads only tables under a heading named "Scenarios". It fails when an ID documented in such a table has no test, or when a test names an ID that no such table documents. The check itself is future work. Fixtures derived from the operator's real data are anonymised to invented placeholders before they are committed.
