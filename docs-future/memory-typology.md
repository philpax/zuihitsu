# The memory typology

Memory has four lifecycle classes: semantic, episodic, procedural, and working. The distinction controls authority, retrieval, decay, and evidential use; it is not a search label. The taxonomy is well motivated by cognitive architectures and agent systems, but the exact storage and policy below are this design's synthesis ([time and memory research](research/2026-07-24/lanes/time-memory.md)).

The canonical object identities are defined elsewhere. [Statements](statements.md) owns Occasion, Activity, Proposition, Assertion, Attestation, and Derivation. [Artefacts and perceptions](artefacts-and-perceptions.md) owns Artefact, ArtefactReference, and Perception. This chapter assigns those objects to memory lifecycles without redefining them.

## Semantic

Semantic memory consists of curated Assertions about the world and the source and derivation records needed to interpret them. It is durable, audience-resolved, and eligible for structural query. Support is computed from visible Attestations; model or tool observations enter through Perceptions and Derivations, not fictitious human testimony.

Not all durable input becomes semantic structure. An Occasion can yield no Assertion, and an Artefact can remain available only through its reference and access policy. Formal, figurative, or insufficiently grounded content may remain source-only.

Semantic memory does not mean “true”. Assertions can be quoted, candidate, contested, superseded, or retracted according to the append-only lifecycle in [statements](statements.md).

## Episodic

Episodic memory preserves experience and optional reconstructions of it. The durable source side is an Occasion or Activity with its original utterance, participants, ordering, ArtefactReferences, and tool/model records. A generated episode is a separate mnemonic narrative produced by a generation Activity and recorded as a Derivation.

Generated narrative is never participant testimony, never a premise for a semantic Derivation, and never allowed to accrue Attestations. It is labelled as reconstruction and carries lineage and the intersection of its inputs' transmission restrictions. It is an optional gated extension, not a required trace for every Occasion. [The two traces](two-traces.md) owns this boundary.

The evidence for generated episodes is limited to one unreplicated study using an automated judge and small per-category samples, with no privacy evaluation and no encoding-versus-retrieval ablation ([dual-trace study](research/2026-08-03/dual-trace.md#limitations-theirs-and-ours)). Its reported cost neutrality does not transfer to this event-sourced log. Stage 0a must show encoding-side value before generation is enabled.

Source Occasions are not demoted when no episode is generated. They remain retrievable under source and audience policy. Episodic ranking may decay with recency, but source history is not rewritten merely because it is old.

## Procedural

Procedural memory is executable agent-authored code plus a natural-language description used for retrieval. Producing or revising a procedure is an Activity. Invocation is another recorded Activity with code version, inputs, tool effects, and outcome.

Procedures are retrieved by purpose and run in the ordinary sandbox with no additional authority. Ranking decays by invocation recency and frequency rather than calendar age: an unused routine is not thereby false or stale. Automatic procedure extraction remains gated on bounded cost, review, and authority fixtures.

A procedure is not an Assertion. Claims about what it does, whether it succeeded, or when it is safe are ordinary Assertions supported by tool observations or operator evidence.

## Working

Working memory is a persistent but transient agent scratchpad. Notes are not Assertions, have no teller, and carry no independent epistemic support. Promotion does not relabel a note: it creates a proposed Assertion or other durable result through a recorded Activity and normal critics.

Every note carries an influence envelope for the content rendered into the model context that produced it, including visible Assertions, source Occasions, Perceptions, tool results, rejected proposals, and prior notes. A promoted result inherits the intersection of those restrictions. Taint is per note and monotone; session-global accumulation would eventually make all promotion impossible.

This is conservative because a model cannot report which visible input actually influenced a note. Over-taint can block useful promotion, but under-taint can disclose restricted content. The event-log versus compacted-storage choice remains a genesis decision because replay must reproduce the envelope and promote-or-discard outcome; the research lane explicitly treated storage as uncertain ([time and memory research](research/2026-07-24/lanes/time-memory.md#mapping-zuihitsus-open-issues-onto-the-typology)).

## Conversational artefacts are not bulk ingestion

An image or document shared during conversation first creates an ArtefactReference on an Occasion. Model inspection produces a Perception through a recorded Activity. Neither arrival nor inspection automatically creates a semantic Assertion. If the agent deliberately records an image-derived claim, the Derivation cites the Perception and underlying ArtefactReference and retains their audience restrictions.

This ordinary conversational path lets the model inspect a supplied image during the turn and records durable provenance for the inspection. Historical reinspection, OCR, generated captions, region grounding, and visual retrieval are separately gated capabilities. Their exact model belongs to [artefacts and perceptions](artefacts-and-perceptions.md).

Bulk ingestion is a different operation. It is a bounded, source-first job over an Artefact, normally a long document or media object. The source and its ArtefactReference become durable before selection or extraction. The job records deterministic segmentation or transforms, selection decisions, extraction Activities, Perceptions where applicable, and per-unit success or source-only fallback. It does not simulate hundreds of conversational Occasions or fabricate utterances.

A bulk job has one governing source audience unless explicit source partitions carry separately authorised policies. Derived Assertions and Perceptions inherit restrictions through normal provenance. Mixed-audience material defaults to the stricter policy; per-claim model guesses cannot widen it. Job leasing, retry, compare-at-commit, poison handling, and supersession belong to [off-turn work](off-turn.md).

Bulk ingestion remains stage-gated. Research motivates separate lifecycles and source-first selective structuring, but exact document selection, extraction economics, and per-document audience behaviour are unresolved ([time and memory research](research/2026-07-24/lanes/time-memory.md#mapping-zuihitsus-open-issues-onto-the-typology)). Required evidence includes bounded calls and log growth, precision against selected spans, source-only degradation, audience non-interference, and replay after retry.

## The self and directives are configuration

The agent's charter and identity are versioned, operator-owned configuration, not memory. They are always supplied through their dedicated slot and cannot be searched, retracted, consolidated, or promoted by memory machinery. The agent may propose a change, but activation is an operator action.

Directives are separately versioned configuration scoped globally, per context, or per conversation. Connector-authored directives are limited to their context and cannot edit the self slot. A directive is not an Assertion: it has no truth value, teller, validity interval, or support. The modelling study found 22 directives stored as ordinary content, including repeated connector material; keeping configuration outside the typology addresses that observed category error ([modelling study](research/2026-08-03/modelling-study.md#directives-are-not-assertions)).

Claims about the agent remain semantic Assertions. “The agent observed X” may be sourced by an Activity; “the operator instructs the agent to do X” is configuration or a deontic Assertion depending on whether it configures this system or describes an obligation in the world. The write path must choose explicitly rather than infer configuration from imperative prose.

## Scenario matrix

| Scenario | Durable objects | Not implied |
|---|---|---|
| A participant shares an image without text | Occasion plus ArtefactReference; optionally an inspection Activity and Perception | No utterance and no automatic Assertion |
| A tool returns a measurement off-turn | Activity and tool observation; optionally a derived Assertion | No human teller or synthetic Occasion |
| A long document is ingested | Artefact/Reference, job Activities, transforms or Perceptions, selected Assertions, and source-only spans | No conversational episode per chunk |
| An episode is generated for a rich session | Source Occasions plus an Activity, its Derivation, and a labelled reconstruction | No new evidence or participant Attestation |
| A scratch note is promoted | Original note and influence envelope plus a new proposal Activity | The note itself does not become an Assertion |
