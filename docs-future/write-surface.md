# The write surface

The conversational write surface has two operations: `record` and `claim`. Both create durable proposal transactions governed by [the verified-write state machine](verified-write.md#proposal-state-machine). The surface does not define Occasion, Activity, Proposition, Assertion, Attestation, Event, ArtefactReference, Perception, or Derivation identity. [The assertion model](statements.md) and [artefacts and perceptions](artefacts-and-perceptions.md) own those definitions.

## `record`

`record` stores an external social input as an Occasion. The Occasion can contain an utterance, zero or more ArtefactReferences, or both. An artefact-only share is valid. `record` does not require a synthetic utterance for it.

```lua
local proposal = quill:record({
  utterance = "wren built quill after rowan shared the architecture",
  artefacts = attachments,
  frame = "system",
  transmission = "public",
})
```

The call returns a durable proposal handle immediately. The current system records model calls durably and batches writes inside a block, which supplies the execution seam ([current write path](../docs/write-path.md), [model-call storage](../docs/events-and-storage.md#event-sourcing)). The durable proposal handle, later-block review, state transitions, and atomic publication are design synthesis required by permanence; the `stage:7` first audience-resolved read/write vertical-slice crash, retry, and source-only fixtures validate them.

The initial extraction runs once per block over the buffered Occasions. A pre-acceptance infrastructure failure can enter `extraction_retry_wait` and retry the extraction within the bounded policy. A later block reads the proposal after it reaches `awaiting_review`:

```lua
local review = proposal:review()
review:amend(review.assertions[3], { role = "source" })
review:drop(review.assertions[6], "the utterance does not assert this")
review:accept()
```

The review contains proposed Proposition, Assertion, Attestation, Event, Perception, and Derivation handles, critic diagnostics, and the current proposal state. It does not report these objects as committed until the publication record commits. Dropped and replaced proposal versions remain in the audit trace.

The write surface never makes rejected proposals available through ordinary structured queries. The source Occasion remains available through the source lane from `source_buffered` onward. If pre-acceptance extraction fails, every proposal is dropped, the extraction retry bound is exhausted, or review cannot complete, the state machine appends `source_only`. A post-acceptance publication failure enters `publication_retry_wait` and retries publication of the unchanged accepted set; it never falls back to `source_only` and never reruns durable model work. Source-first retention is forced by current prose storage and by the observed limits of neural verification, but this exact fallback state and its publication boundary are design synthesis ([current data model](../docs/data-model.md#contententry), [writer failure](../docs/ontology-failures/2026-07-23.md#the-neural-writer-is-unverified), [welding research](research/2026-07-24/lanes/welding.md)). Source-only storage is a valid durable outcome rather than a partial commit.

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

The admitted source kinds are `agent_observation`, `operator_assertion`, `tool_observation`, and `derivation`. Each kind has its own authority and grounding requirements. `claim` never fabricates an utterance, teller, text span, or Occasion. A tool observation names the tool Activity and result. A derivation supplies the typed input and execution environment required by [verified writes](verified-write.md#derivation-records).

`claim` avoids extraction, but it does not bypass critics, review, atomic publication, audience checks, or compare-at-commit. Proposal, retry, and publication records preserve the [canonical InfluenceEnvelope](privacy-and-provenance.md#influence-envelopes), including non-evidentiary marks. A fresh source-only context uses a new Activity; it cannot clear marks on an existing context or proposal. A correction after publication appends the applicable Assertion or Attestation transition. It does not mutate the published record.

## Required caller decisions

The caller supplies judgements that source extraction cannot establish safely:

- the frame, including an explicit principal redirect when applicable;
- the default transmission principle for an Occasion;
- the teller when the speaker relays another person's words;
- the source kind for `claim`;
- explicit `unknown` or `not_applicable` values where the schema permits them.

A compound Occasion can produce proposals with different transmission principles. The call-level value is only a default. Review cannot accept the publication set until every Attestation and derived output has an audience result. [Privacy and provenance](privacy-and-provenance.md) owns the compilation and influence rules.

The source kind and teller are independent. An agent restatement of a participant's words remains grounded in the original Attestation and does not create independent support. A direct agent observation uses an Activity and has no human teller.

## Block and retry behaviour

Several `record` calls in one block can share one extraction Activity. The durable proposal handles remain empty until that Activity and the hard critics complete. Review therefore occurs in a later block. This preserves batching without hiding the proposal lifecycle.

Each failed attempt appends its failure class and attempt number. Before acceptance, a retriable extraction infrastructure failure enters `extraction_retry_wait`. After acceptance, a retriable publication infrastructure failure enters `publication_retry_wait`; it retries the unchanged accepted publish set without rerunning durable model work. A critic infrastructure retry reuses the recorded extraction output and remains in `critic_review`. A deterministic critic rejection returns diagnostics for amendment or dropping; it does not rerun unchanged extraction. The bounded retry policy is versioned. Exhaustion before acceptance produces `source_only`. Publication exhaustion resolves a committed marker to `published` or aborts a confirmed uncommitted attempt with `publication_attempts_exhausted`, following [the recovery protocol](verified-write.md#proposal-state-machine). It does not produce `source_only`.

A caller can supersede a pending proposal with a replacement. Supersession names both proposal IDs. The old proposal can never publish. An abort records the actor and reason. Neither operation removes the durable source.

Crash recovery resumes from the folded proposal state. It reuses recorded model outputs and stable temporary IDs. It does not repeat a nondeterministic call whose Activity result is already durable. Recording nondeterministic activity is corroborated by the current event log and durable-execution research; the exact temporary-ID, compare-at-commit, and crash fold are local synthesis ([current model-call contract](../docs/events-and-storage.md#event-sourcing), [durable activity research](research/2026-07-24/verification/part-b.md)). Atomic publication and compare-at-commit follow [the canonical protocol](verified-write.md#proposal-state-machine) and must pass the `stage:7` first audience-resolved read/write vertical-slice fault-injection oracle.

## Teachable errors

A hard-critic error identifies the proposal, critic version, violated definition, source locator, and expected correction. It can name a domain or range mismatch, deprecated relation, malformed validity value, unsupported selector, insufficient authority, unresolved audience, ambiguous Event co-reference, or stale compare head.

The error does not teach ontology-language that the agent may activate. The agent can propose a missing relation or role definition, but activation is a governed schema operation described by [relations](relations.md). Persistent rejection enters the operator exception queue.

## Excluded operations

The conversational surface does not provide:

- a critic bypass or force flag;
- arbitrary Event identity construction or raw role-edge mutation;
- caller-supplied support or credence;
- caller-selected identity merges;
- bulk document or media ingestion;
- self-slot or charter mutation;
- automatic Assertion creation from an arriving artefact.

Bulk ingestion uses the job protocol in [memory typology](memory-typology.md) and [off-turn work](off-turn.md). Artefact arrival creates an ArtefactReference. Image-derived memory requires a recorded Perception and source lineage. Governed schema activation, identity resolution, and charter changes use their canonical owner surfaces.

## Cost boundary

The model call is on the proposal path and never on the state fold. Replay consumes the recorded Activity. A read can use a transient model reranker only when its output is discarded and no stored state depends on the call. [The query surface](query-surface.md) defines read accounting for content that reaches model context.

The eager review loop remains an empirical policy. It can move to end-of-turn or a bounded background retry if measurement shows unacceptable latency or constraint tax. Such a policy change does not change persisted Occasion, Activity, proposal, or publication meanings. Routine indefinite re-extraction of committed sources remains excluded because it creates nondeterministic drift and cost proportional to stored history.
