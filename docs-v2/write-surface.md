# The write surface

The write surface is the agent-facing API that produces [verified-write proposals](verified-write.md#proposal-states). It defines no object identity; [the object model](statements.md) and [artefacts and perceptions](artefacts-and-perceptions.md) own those.

Two candidate surfaces exist for conversational writes, and [the evidence milestone](program/evolution.md) decides between them by a head-to-head comparison on the real corpus. This chapter does not choose. Both produce the same proposal, critic, and publication records, so the choice changes which Activities occur and what the agent is taught, not the persisted object model. `claim`, the caller decisions, teachable errors, excluded operations, and the cost boundary are common to both.

Under both, the Occasion is durable before any structure exists. Source-first retention is forced by current prose storage and the observed limits of neural verification ([current data model](../docs/data-model.md#contententry), [writer failure](../docs/ontology-failures/2026-07-23.md#the-neural-writer-is-unverified), [welding research](research/2026-07-24/lanes/welding.md)). The current system records model calls durably and batches writes inside a block, which supplies the execution seam ([current write path](../docs/write-path.md), [model-call storage](../docs/events-and-storage.md#event-sourcing)).

## Candidate A: record, extract, review

The connector records every external input as an Occasion before the agent sees it. `record` takes one or more existing Occasion IDs and opens a structuring proposal over them. It never takes free text, so the agent cannot author the source it then structures. The Occasion can hold text parts, ArtefactReference parts, or both. An artefact-only share is valid and needs no synthetic utterance.

```lua
local proposal = quill:record({
  occasions = { occasion_id },
  frame = "system",
})
```

The call returns a proposal handle immediately. One extraction Activity per block proposes typed structure for the block's Occasions, the hard critics run over it, and a later block reviews the result:

```lua
local review = proposal:review()
review:amend(review.assertions[3], { role = "source" })
review:drop(review.assertions[6], "the utterance does not assert this")
review:accept()
```

The review holds the proposed handles, the critic diagnostics, and the proposal state, and reports nothing as committed until publication commits. Dropped and amended versions stay in the audit trace. Candidate A costs an extra model call per write block, a second block before structure is visible, and the review rounds.

## Candidate B: typed claims

The agent writes typed claims directly against one or more recorded Occasions. Each claim names the Proposition fields, the validity, and the source locator: a text span, or a selector over a cited ArtefactReference. A testimony claim's teller is bound to the cited span ([teller binding](#teller-binding)). The hard critics are deterministic, so they run at the call and return teachable errors at once. A block's claims form one proposal that publishes atomically at block commit. An Occasion with no accepted claim ends `source_only`. Candidate B needs no extraction call and no second block. It costs the constraint tax of schema-shaped writing inside the turn and risks omitting structure an extractor would have proposed.

## What the comparison measures

The evidence milestone runs both candidates over the same corpus and measures fidelity, omission, junk fill, constraint tax on the rest of the turn, blocks and review rounds per write, latency, retries, log growth, and the usefulness of the source-only path. The criteria are preregistered acceptance gates, not reports. Forced-choice elicitation removes omission variance but moves it into field content as junk fill, so both are measured. Eager structuring at the turn against deferred structuring in a background job is also open. Either schedule uses the same records: a deferred proposal is opened by a `structuring` job ([off-turn work](off-turn.md#authority-classes)), under the same teller binding and audience rules.

## `claim`

`claim` writes explicit structure from a non-Occasion source. The caller supplies a source kind and the source Activity fields:

```lua
local proposal = quill:claim("runs_on", "model/opus-4.8", {
  source = "agent_observation",
  frame = "system",
  modality = "actual",
  polarity = "positive",
  valid_from = "2026-07-16",
})
```

The source kinds are `agent_observation`, `operator_assertion`, `tool_observation`, and `derivation`, each with its own authority and grounding. `claim` never fabricates an utterance, teller, span, or Occasion. A tool observation names the tool Activity and result. A derivation supplies the inputs required by [verified writes](verified-write.md#derivation-inputs). `claim` skips extraction but not the critics, atomic publication, audience checks, or the predecessor check. The producing Activity's manifests travel with the proposal, so a fresh Activity cannot clear what an earlier context contained. A later correction appends a transition and never mutates the published record.

## Required caller decisions

The caller supplies the judgements that extraction cannot establish safely:

- the frame, including an explicit `principal` redirect through `presents` when applicable;
- the source kind for `claim`;
- an explicit `unknown` or `not_applicable` where the schema permits one.

Transmission is not a free caller decision. A `testimony` Attestation's principle starts at the floor that [privacy and provenance](privacy-and-provenance.md#transmission-principles) defines: its source Occasion's restriction. Per item, the caller may only narrow that floor, or cite a teller-grant span to widen it, which the attestation critic checks ([verified writes](verified-write.md#hard-critics)). Any other widening is an operator action. A compound Occasion therefore yields items with different principles only by narrowing. Every other output takes its ancestry restriction, which the caller cannot set. The source kind and the teller are independent, and a direct agent observation has no human teller.

## Teller binding

The teller is not a caller decision. A `testimony` teller is the participant who produced the cited span on that Occasion, and the attestation critic rejects any other teller.

Reported speech is always `quoted`. A relay ("Quinn told me that X", said by Rowan) is a `quoted` Assertion with Rowan as teller, plus `relayed_from` lineage naming Quinn as the original source person. Quinn gets no teller exception and no erasure authority over the relay, because Rowan performed the telling. Reads render it only as reported speech ("Rowan says Quinn said X"), never as a flat claim. Mode is part of the [reuse match](statements.md#assertion-reuse), so a relay never attaches support to an `asserted` Assertion with the same key. The lineage feeds [dependence](belief.md#dependence). A claim a book makes follows the same rule ([reading a document](artefacts-and-perceptions.md#reading-a-document)).

## Retries and abandonment

Failed attempts are recorded as [verified writes](verified-write.md#proposal-states) describes, under a versioned, bounded retry policy whose exhaustion ends in `source_only`. A caller can abandon a pending proposal, optionally naming a replacement, and it never publishes. Neither outcome removes the source.

## Teachable errors

A hard-critic error names the proposal item, the critic version, the violated definition, the source locator, and the expected correction. It can report a domain or range mismatch, a testimony value or entity reference absent from the cited span, a teller who did not produce the cited span, a deprecated relation with its successor, a malformed validity value, an unsupported selector, insufficient authority, a principle wider than its floor with no cited grant, an unresolved audience, or a stale predecessor.

The agent coins relations under the critics in [relations](relations.md#coining-under-critics), which render the relations in use at the point of writing and teach reuse before coinage. Entity kinds, roles, Event types, frames, modalities, and transmission principles stay operator-governed, and no error invites the agent to create one. Persistent rejection enters the operator exception queue.

## Excluded operations

The conversational surface does not provide:

- a critic bypass or force flag;
- arbitrary Event identity construction or raw role-edge mutation;
- caller-supplied support or credence;
- caller-selected identity merges;
- bulk document or media ingestion;
- mutation of the self or the directives;
- automatic Assertion creation from an arriving artefact.

Bulk ingestion uses jobs ([memory typology](memory-typology.md), [off-turn work](off-turn.md)). An arriving artefact creates only an ArtefactReference; image-derived memory needs a recorded Perception. Identity, operator-governed definitions, and the self and directives have their own owner surfaces.

## Cost boundary

The model call is on the proposal path, never on the fold; replay consumes the recorded Activity. A read may use a transient reranker only when its output is discarded ([the query surface](query-surface.md)). The structuring schedule is an empirical policy that can move to end-of-turn or a bounded background retry without changing the meaning of any persisted record. Routine re-extraction of committed sources stays excluded, because it creates nondeterministic drift and cost proportional to stored history.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `record-requires-occasion` | The agent calls `record` with free text instead of Occasion IDs. | The call is refused with a teachable error. No Occasion is created. |
| `artefact-only-record` | A participant shares an image with no text, and the agent calls `record` on the Occasion. | The Occasion holds one ArtefactReference part and no text part. No synthetic utterance is created. |
| `compound-transmission` | One Occasion in channel C carries a remark and a confidence about a third party, and the caller narrows only the confidence to `in_confidence`. | The remark's Attestation carries the floor `channel(C)`. The confidence's Attestation carries `in_confidence`. Neither is wider than `channel(C)`. |
| `caller-widening-rejected` | A DM's testimony is proposed as `public` with no cited grant. | The attestation critic rejects the item with a teachable error naming the floor. Nothing publishes for that item. |
| `relay-is-quoted` | Rowan says "Quinn told me that X", and an `asserted` Assertion with the same key exists. | A new `quoted` Assertion is minted with Rowan as teller and `relayed_from` naming Quinn. The `asserted` Assertion gains no Attestation. Quinn has no erasure authority over the relay. |
| `teller-not-span-author` | A proposed Attestation names Quinn as teller but cites a span Rowan produced. | The attestation critic rejects it with a teachable error. |
| `claim-no-fabricated-occasion` | The agent records a tool observation with `claim`. | The source is the tool Activity. No Occasion, utterance, teller, or span is created. |
| `review-later-block` | Candidate A: a block records two Occasions. | One extraction Activity serves both. Their proposals are reviewable only in a later block. |
| `typed-claim-inline-error` | Candidate B: the agent writes a claim whose object violates the relation's range. | The call returns a teachable error within the block. No model call occurs. |
| `excluded-force-flag` | A caller passes a critic bypass or supplies a credence value. | The call is refused with a teachable error, and no proposal is opened. |
| `surfaces-same-records` | Candidates A and B structure the same Occasion to the same content. | Both publish identical Proposition keys, Assertions, and Attestations. Only the Activities differ. |
