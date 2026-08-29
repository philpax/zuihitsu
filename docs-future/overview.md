# Overview

zuihitsu is a neurosymbolic personal-agent harness. Its append-only event log is the source of truth. Deterministic replay materialises projections. Language models operate through recorded Activities when their output affects durable state. One instance is one agent and one log.

The architecture in this tree is a proposed successor. It does not describe the current implementation. Current behavior is documented in [`../docs/`](../docs/).

## Permanent object boundaries

The successor records external input as an [Occasion](statements.md#occasion-and-activity) and agent, operator, tool, or model work as an Activity. An Occasion owns one ordered interleaved sequence of text and [ArtefactReference](artefacts-and-perceptions.md) content parts; either kind may be absent. An Activity can produce an Assertion, Perception, or Derivation without inventing an utterance. Agent action intent and firing are separate minted Task and Trigger records.

Semantic memory has three assertion layers. A Proposition contains canonical content. An Assertion situates that content in validity and immutable asserted/quoted mode, with a separate folded lifecycle. An Attestation records one teller's support on one Occasion. Event roles and attributes are Assertions. Media observations are Perceptions rather than participant testimony. Generated narrative remains a separate non-evidentiary trace.

| Concern | Normative owner |
|---|---|
| Object identities, assertion lifecycle, contradiction, derivation | [Assertions](statements.md) |
| Artefacts, typed content parts, Perceptions, reinspection | [Artefacts and perceptions](artefacts-and-perceptions.md) |
| Event identity, roles, and reversible co-reference | [Events and roles](events-and-roles.md) |
| Registered definitions and schema evolution | [Relations](relations.md) |
| Identity hypotheses and resolution environments | [Identity](identity.md) |
| Support and dependence | [Belief](belief.md) |
| Validity, occurrence, tasks, and triggers | [Time](time.md) |
| Witness evidence, transmission, influence, and erasure | [Privacy and provenance](privacy-and-provenance.md) |
| Proposal transaction and critics | [Verified write](verified-write.md) |
| Agent-facing reads and writes | [Query surface](query-surface.md) and [write surface](write-surface.md) |
| Memory lifecycles and generated episodes | [Memory typology](memory-typology.md) and [two traces](two-traces.md) |
| Background jobs | [Off-turn work](off-turn.md) |

## Permanence contract

The current instance is outside the successor boundary. It is not migrated or dual-read. A successor instance starts at genesis.

After successor genesis, persisted meaning and stable identity cannot change incompatibly:

- an event payload version retains its original meaning;
- a stable object or definition ID is never repurposed;
- new capability uses additive event variants, registered definition versions, new projections, or explicit superseding records;
- replay never invents a value absent from historical input;
- policy changes create versioned projections or Derivations rather than changing old conclusions silently;
- no stage requires resetting an agent born on the successor substrate.

The design records broad immutable source data and applies narrow versioned interpretation. Existing event-sourcing and durable-activity behavior supports append-only replay. The exact no-incompatible-change contract is an operator constraint and design decision ([current storage contract](../docs/events-and-storage.md), [verification](research/2026-07-24/verification/part-b.md), [migration cost](research/2026-07-24/lanes/survey-giants.md)).

## Status labels

Every capability uses one of four labels:

- `required at genesis`: permanent identity or raw data that must exist before the successor can run safely;
- `initial policy`: behavior enabled in the first usable successor;
- `gated extension`: behavior added later because genesis records its required inputs;
- `open experiment`: behavior outside the committed architecture until evidence passes its gate.

[Evolution](evolution.md) assigns these statuses to the build stages. A gated or experimental policy cannot require retrospective invention.

## System commitments

The event log remains the source of truth. Every nondeterministic call that affects durable state is recorded. Replay performs no model, embedder, or tool calls. A live read can compute transient ranking, but durable access accounting records content rendered into model context rather than hidden candidates.

Audience resolution occurs before evidence affects a conversational read, ranking, decision, Derivation, or initiated action. Hidden evidence cannot alter an audience-visible result unless the result inherits the evidence restriction. Influence tracking covers accepted, rejected, and transient computation.

The agent-facing surface remains small. The agent addresses stable handles and uses typed query and write verbs. Governed schema activation, identity resolution, and policy machinery remain outside conversational ontology syntax.

Scale-sensitive work is bounded and incremental. Long-document and media ingestion are jobs over Artefacts and Activities. Optional exploration remains disabled until it has separate privacy and yield evidence.

## Representational scope

Structured Assertions do not contain all retained content. Figurative or formal material can remain in an utterance or Artefact. An artefact-only Occasion is complete input. Generated Perceptions, OCR, captions, and episodes remain distinguishable from source material. Initial inference can treat non-actual modalities opaquely while retaining their permanent identity coordinate.

The design therefore does not depend on extracting a Proposition from every input. Source-only fallback is a valid terminal state of the write transaction.