# Overview

zuihitsu is a neurosymbolic personal-agent harness. Its append-only event log is the source of truth. Deterministic replay materialises projections. Language models operate through recorded Activities when their output affects durable state. One instance is one agent and one log.

The architecture in this tree is a proposed successor. It does not describe the current implementation. Current behaviour is documented in [`../docs/`](../docs/). The deployment it serves has one operator, one agent, a handful of participants reached through connectors, and a server that is not publicly exposed.

## Object boundaries

The successor records each inbound message and each outbound agent utterance as an [Occasion](statements.md#occasion), and agent, operator, tool, or model work as an [Activity](statements.md#activity). An Occasion owns one ordered, interleaved sequence of text and [ArtefactReference](statements.md#artefact-and-artefactreference) content parts; either kind may be absent. Every model-call Activity records a [context manifest](statements.md#context-manifest) of the objects rendered into its context. Influence, taint, restriction, and access accounting are projections over those manifests, not fields stored on objects. Every Occasion carries a restriction: its availability audience at receipt, or its delivered audience ([privacy and provenance](privacy-and-provenance.md#occasion-restriction)).

People, organisations, places, and topics are [Entities](statements.md#entity): minted ULIDs with a fixed registered kind. A handle such as `person/rowan` is a mutable label over the ULID, and Proposition keys never hold handles. A connector stub is a person Entity with connector scope, and it joins an agent-minted person Entity only through a reversible `same_as` hypothesis.

Semantic memory has three assertion layers. A Proposition is a canonical key over content. An Assertion situates that content in validity and an immutable asserted or quoted mode, and its validity and lifecycle fold per audience. Reported speech is always quoted. An Attestation records one source's support: testimony, a direct observation, or a derivation. Testimony must be grounded in the teller's own span, and its principle is never wider than its Occasion's restriction without a teller grant. Event roles and attributes are Assertions. Media observations are Perceptions rather than participant testimony. Generated narrative remains a separate non-evidentiary trace. An agent's action intent is a Task whose trigger conditions fire only while it is active.

| Concern | Normative owner |
|---|---|
| Object identities, Entities and handles, lifecycle mechanics, context manifests, contradiction | [Object model](statements.md) |
| Artefacts, selectors, Perceptions, reinspection | [Artefacts and perceptions](artefacts-and-perceptions.md) |
| Event identity, roles, and disclosure-safe projection | [Events and roles](events-and-roles.md) |
| Registered definitions and schema evolution | [Relations](relations.md) |
| Identity hypotheses, clearance, and resolution environments | [Identity](identity.md) |
| Support, settlement, dependence, and contest | [Belief](belief.md) |
| Validity, occurrence, Tasks, and trigger conditions | [Time](time.md) |
| Occasion restriction, witness evidence, transmission, the subject guard, influence, and erasure | [Privacy and provenance](privacy-and-provenance.md) |
| Proposal transaction and critics | [Verified write](verified-write.md) |
| Agent-facing reads and writes | [Query surface](query-surface.md) and [write surface](write-surface.md) |
| Memory lifecycles and generated episodes | [Memory typology](memory-typology.md) and [two traces](two-traces.md) |
| Background jobs | [Off-turn work](off-turn.md) |

## Permanence contract

The current instance is outside the successor boundary. Its agents are frozen until the successor comes online. They are never migrated or dual-read, and they are not a compatibility target. A successor instance starts at its first real genesis.

Before genesis, every successor log, encoding, event variant, fold, projection, and test fixture is disposable. Evidence that falsifies the design can replace any of them. Measurements, failures, and decision rationales remain evidence, but their formats carry no compatibility guarantee. Each increment leaves the repository buildable and its relevant tests passing; it need not leave a usable agent.

After the first real genesis, persisted meaning and stable identity never change incompatibly:

- an event payload version keeps its original meaning;
- a stable object or definition ID is never repurposed;
- new capability is additive: new event variants, registered definition versions, and new projections;
- a structural change that is not additive arrives through a recorded upcast;
- a policy change produces a versioned projection rather than silently changing an old conclusion;
- no change may require resetting an agent born on the successor.

The operator accepts additive changes and small recorded upcasts after genesis. An ontology-wide migration or an agent reset is not acceptable.

### The upcast rule

An upcast may restructure data. It never supplies a value that was absent from its input. A value the input did not carry becomes an explicit `unknown`. Replay therefore never invents historical content, and a reader can always tell a recorded value from a missing one.

### The keep-at-genesis test

A field or record kind is required at genesis only if it is part of an identity key, or if its value cannot be recovered later from the retained raw input. Everything else can be added later additively or by a recorded upcast, and the chapters list it as deferred instead of reserving it.

Frame, polarity, and modality pass the test because they are Proposition identity coordinates, and old structure cannot reconstruct them. Page and text-span selectors pass because grounding a read book at page level cannot be recovered without re-reading it. A capability is either in the genesis design or deferred. [Evolution](program/evolution.md) lists the deferred capabilities and the condition that reopens each.

The design records broad immutable source data and applies narrow versioned interpretation. Existing event-sourcing and durable-activity practice supports append-only replay. The exact no-incompatible-change contract is an operator constraint and a design decision ([current storage contract](../docs/events-and-storage.md), [verification](research/2026-07-24/verification/part-b.md), [migration cost](research/2026-07-24/lanes/survey-giants.md)).

## Distributed operation

Distributed operation is a non-goal that the design must not preclude. The successor runs as one writer, and it does not design a sync layer. Four choices keep the option open:

- every stable identity is a ULID, so two writers never mint the same ID;
- every lifecycle transition names its predecessor, so concurrent writes surface as a fork rather than a silent overwrite ([lifecycle mechanics](statements.md#lifecycle-mechanics));
- domain semantics never depend on the local log sequence number;
- a read or write records the log position it observed as an opaque frontier, which is a local sequence today and could be a vector clock later.

Occasions, Assertions, and Attestations are append-only sets, and two copies of them merge by union. Lifecycle transitions, handle assignment, Task trigger firing, identity acceptance, and erasure are the parts that would need coordination between writers.

## System commitments

The event log remains the source of truth. Every nondeterministic call that affects durable state is recorded. Replay performs no model, embedder, or tool calls. A live read can compute transient ranking, but access accounting covers only content rendered into a model context, as the context manifest records it, never hidden candidates.

Audience resolution occurs before evidence affects a conversational read, ranking, decision, derivation, or initiated action. Hidden evidence cannot alter an audience-visible result unless the result inherits the evidence restriction. Influence covers accepted, rejected, and transient computation, because every rendered context passes through the manifest.

The agent-facing surface remains small. The agent addresses Entities through handles, which are mutable labels over stable ULIDs, and uses typed query and write verbs. Identity resolution and policy machinery stay outside conversational ontology syntax. The agent coins relations under critics; entity kinds, roles, Event types, frames, modalities, and transmission principles stay operator-governed ([relations](relations.md)).

Scale-sensitive work is bounded and incremental. Long-document and media ingestion are jobs over Artefacts and Activities. Exploration is deferred and disabled until its own privacy and yield evidence exists ([off-turn work](off-turn.md#deferred-exploration)).

## Representational scope

Structured Assertions do not contain all retained content, and the design does not depend on extracting a Proposition from every input. Perceptions, OCR, captions, and generated episodes remain distinguishable from source material. [The object model](statements.md#representational-limits) states the limits.
