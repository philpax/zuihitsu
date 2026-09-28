# The memory typology

Memory has four lifecycle classes: semantic, episodic, procedural, and working. The class controls authority, retrieval, decay, and evidential use. It is not a search label. The four-way taxonomy is well supported by cognitive architectures and agent memory systems. The storage and policy below are this design's synthesis ([time and memory research](research/2026-07-24/lanes/time-memory.md)).

The [object model](statements.md) defines the objects: Entity, Occasion, Activity, context manifest, Proposition, Assertion, Attestation, Artefact, ArtefactReference, and Perception. This chapter assigns them to lifecycles without redefining them.

## Semantic

Semantic memory is the curated Assertions about the world, with the source records needed to interpret them. It is durable, audience-resolved, and eligible for structural query. Support is computed from visible Attestations. Model and tool observations enter through `observation` Attestations and Perceptions, never as fictitious human testimony.

Not every input becomes structure. An Occasion can yield no Assertion, and an Artefact can stay reachable only through its reference. Formal, figurative, or insufficiently grounded content can remain source-only.

Semantic does not mean true. An Assertion can be quoted, candidate, contested, superseded, or retracted under the lifecycle in the [object model](statements.md#lifecycle-mechanics).

## Episodic

Episodic memory preserves experience. Its durable side is the source trace: Occasions and Activities with their text, participants, ordering, ArtefactReferences, and tool and model records. Source Occasions are never demoted. They remain retrievable under source and audience policy. Episodic ranking may decay with recency, but old source history is not rewritten.

Access recency and frequency are counted per audience, and decay is anchored to the read's explicit as-of time, under the [query surface](query-surface.md#access-accounting) rule. It applies to every lifecycle class that ranks by access.

A generated episode is a derived Artefact of kind `episodic_reconstruction`, and generation is a deferred capability. An episode is never testimony, never an input to a semantic write, and never supported by Attestations. [The two traces](two-traces.md) owns the boundary, including the evidence for and against generation and the [episodic wall](two-traces.md#the-episodic-wall).

## Procedural

Procedural memory is executable agent-authored code plus a natural-language description used for retrieval. Writing or revising a procedure is an Activity. Each invocation is another recorded Activity with its code version, inputs, tool effects, and outcome.

Procedures are retrieved by purpose and run in the ordinary sandbox with no additional authority. Ranking decays by invocation recency and frequency rather than calendar age, because an unused routine is not thereby false or stale.

A procedure is not an Assertion. Claims about what it does, whether it succeeded, or when it is safe are ordinary Assertions supported by tool observations or operator evidence.

Automatic procedure extraction is deferred. It reopens when scenarios show bounded cost, review, and authority ([evolution](program/evolution.md#deferred-capabilities)).

## Working

Working memory is a persistent but transient agent scratchpad. A note is not an Assertion, has no teller, and carries no independent support. Promotion does not relabel a note: it creates a proposed Assertion or other durable result through a recorded Activity and the normal critics. Explicit promotion by the agent is in the genesis design. Scheduled review of notes, with its `working_review_due` queue mark, is deferred as working-note promotion review ([evolution](program/evolution.md#deferred-capabilities)).

A note's taint is the restriction projection over the ancestry of the Activity that wrote it ([privacy and provenance](privacy-and-provenance.md#influence-from-context-manifests)). A promoted result inherits it, and the [episodic wall](two-traces.md#the-episodic-wall) applies through the same manifests. Taint is per note and follows the note's own ancestry. A session-global accumulation would eventually block every promotion.

This is conservative because a model cannot report which rendered input actually influenced a note. Over-taint can block a useful promotion. Under-taint can disclose restricted content. Measured on the live log, memories touched per block have a median of 1 and a maximum of 11, but that figure is only a lower bound on taint-set size. A manifest also lists the brief's components (2,240 to 8,106 characters per brief) and the rendered conversation history ([log measurements](research/2026-08-06/log-measurements.md)). Brief components were composed for the conversation's audience, so their practical effect is to bound a note's promotion to no wider than the audience of the conversation it was written in. Whether that over-taints useful promotions is open until the evidence milestone measures it ([confidence register](program/confidence.md#evidence-map)).

Notes are stored in the event log, with compaction permitted. The taint is derived from manifests that replay must reproduce, together with the promote-or-discard outcome, so a side table outside the log cannot hold them.

## Conversational artefacts are not bulk ingestion

An image or document shared in conversation creates an ArtefactReference on an Occasion. Inspection is a recorded Activity. Neither arrival nor inspection creates an Assertion by itself. When the agent deliberately records a claim from an image or a document, the claim cites the Perception or the text span it came from, and the audience restrictions of the underlying reference carry through. Reading a book over several turns uses this conversational path with page and text-span selectors ([reading a document](artefacts-and-perceptions.md#reading-a-document)).

Bulk ingestion is a different operation and is deferred. It is a bounded, source-first job over one Artefact, normally a long document or media object. The source and its reference are durable before any selection or extraction. The job records its segmentation, selection decisions, extraction Activities, and per-unit success or source-only fallback. It never simulates conversational Occasions or fabricates utterances. A job has one governing source audience unless explicit source partitions carry separately authorised principles. Mixed-audience material takes the stricter principle, and per-claim model guesses cannot widen it. Retry and supersession follow [off-turn work](off-turn.md).

Research motivates separate lifecycles and source-first selective structuring, but document selection, extraction economics, and per-document audience behaviour are unresolved ([time and memory research](research/2026-07-24/lanes/time-memory.md#mapping-zuihitsus-open-issues-onto-the-typology)). Bulk ingestion reopens with evidence of bounded model calls and log growth, precision against selected spans, source-only degradation, audience non-interference, and replay after retry.

## The self and directives are configuration

The agent's charter and identity are versioned operator-owned configuration, not memory. They are always supplied through a dedicated slot and cannot be searched, retracted, consolidated, or promoted by memory machinery. The agent may propose a change, and activation is an operator action.

Directives are separately versioned configuration, scoped globally, per context, or per conversation. A connector may author directives only for its own context and cannot edit the self slot. A directive is not an Assertion: it has no truth value, teller, validity interval, or support. The modelling study found 22 directives stored as ordinary content, one repeated verbatim ten times, and keeping configuration outside the typology addresses that category error ([modelling study](research/2026-08-03/modelling-study.md#directives-are-not-assertions)).

Claims about the agent remain semantic Assertions. "The agent observed X" can be sourced by an Activity. "The operator instructs the agent to do X" is configuration when it configures this system and a deontic Assertion when it describes an obligation in the world. The write path chooses explicitly and never infers configuration from imperative prose.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `document-read-not-ingestion` | The agent reads a shared book over several turns. | One Occasion and reference, extraction and inspection Activities, and claims citing text spans. No Occasion is created per chunk. |
| `source-occasion-not-demoted` | An old Occasion yielded no Assertion and no episode exists. | The Occasion remains retrievable under its audience policy. |
| `procedure-not-assertion` | The agent writes and invokes a procedure. | Two Activities and no Assertion. A later claim about the procedure's success is an ordinary Assertion with an `observation` Attestation citing the invocation Activity. |
| `note-promotion-inherits-taint` | A note written from a context that rendered Quinn's `in_confidence` testimony is promoted. | The proposed Assertion's effective principle admits no audience beyond Quinn. |
| `note-taint-per-note` | Two notes are written in one session from different contexts. | Each note's restriction is computed from its own ancestry. The note written without the restricted read carries no restriction from it. |
| `access-recency-per-audience` | An item is rendered repeatedly in Quinn's one-to-one conversation and never in a group of Quinn and Rowan. | A read for Quinn alone ranks it by those accesses. A read for the group ignores them. |
| `directive-not-assertion` | A connector sets "Be laconic, one paragraph at most." for its context. | The text is recorded as a context-scoped directive. No Assertion, teller, or support is created. |
| `self-slot-unsearchable` | A memory search matches words in the charter. | The charter does not appear in results and cannot be retracted or consolidated by memory operations. |
