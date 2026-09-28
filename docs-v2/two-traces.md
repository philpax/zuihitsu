# The two traces

The two traces are the durable source record and a generated mnemonic narrative. Generated episodes are a deferred capability. Structured Assertions link to the source record but are not the other trace: they are semantic interpretations with their own lifecycle.

External inputs and delivered agent utterances are [Occasions](statements.md#occasion), and agent, operator, tool, and model actions are [Activities](statements.md#activity). This chapter owns the boundary between source material and generated narrative, which is the episodic wall.

## The source trace

The source trace preserves what arrived or happened without pretending that extraction is lossless. For an Occasion it holds participants, witness evidence, part order, observed and recorded time, the text when present, and ArtefactReferences. For an Activity it holds the actor, inputs, context manifest, implementation or model version, tool observations, and outputs.

Assertions cite typed source locators into this trace. One compound utterance can ground several Assertions, and an utterance can also ground none. The [modelling study](research/2026-08-03/modelling-study.md#compound-entries-fragment-and-the-gloss-stops-being-one-to-one) found a single entry carrying eight claims and established that source prose cannot honestly be split into one invented gloss per claim. The gloss belongs to the utterance, and many Assertions point at it. The study also found figurative content whose only faithful representation is the source prose ([figurative content](research/2026-08-03/modelling-study.md#figurative-content)).

Retaining a source does not assert its content. Quotation, participant testimony, agent observation, and Perception stay distinct under the [object model](statements.md#attestation). A repeated claim may reuse a Proposition and add another Attestation or Assertion, and the second Occasion always stays independently addressable.

Media shared on an Occasion has three descriptive layers that never collapse: the Artefact shared through an ArtefactReference, a participant's caption as a text part marked `caption_of` that reference, and a machine Perception from a versioned Activity. A derived thumbnail, page rendering, or extracted text is a derived Artefact with explicit lineage. [Artefacts and perceptions](artefacts-and-perceptions.md) owns these rules and their scenarios.

## Generated episodes

A generated episode is a derived Artefact holding a synthetic mnemonic scene or narrative. A generation Activity produces it from ordered inputs: source Occasions and Activities, each with exact source locators. The Activity records the output's kind as `episodic_reconstruction`. The kind lives on the Activity's output record, outside the Artefact's erasable payload, so an erased episode is still known to have been one.

An episode can help the agent distinguish, sequence, or aggregate Occasions. It is not raw experience, an Assertion, an Attestation, or evidence. It is attributed to its generation Activity and never to a participant. It is labelled as a reconstruction on every read surface. Its audience restriction is the intersection over the generation Activity's context manifest, computed like any other manifest projection ([privacy and provenance](privacy-and-provenance.md#influence-from-context-manifests)). Generation over content whose transmission principle cannot govern an indivisible narrative is rejected. A correction appends a replacement or disables the episode and never edits source records.

A narrative is indivisible prose. If omitting restricted material would change its account, the whole body is suppressed for that audience. Unlike an Event projection, it cannot reveal selected edges. Audience resolution runs before the narrative is rendered.

Generated prose cannot claim completeness. It is a selective reconstruction that may omit salient details, combine anchors poorly, or invent scene geometry. Its source links let a reader audit it and do not turn it into evidence.

### Evidence and deferral

The supporting study reported gains of 40 points on temporal reasoning, 30 on multi-session aggregation, and 25 on update tracking, with no gain on single-session retrieval ([dual-trace results](research/2026-08-03/dual-trace.md#the-experiment)). The evidence is narrow: one unreplicated benchmark, an automated judge, about 20 questions per category, no privacy dimension, and no ablation separating encoding-time generation from retrieval-time reconstruction ([limitations](research/2026-08-03/dual-trace.md#limitations-theirs-and-ours)). The reported cost neutrality came from a context-heavy harness. In an event-sourced store an episode costs a record-time model call and permanent log volume.

Generation also instructs a model to invent concrete detail. The study's only guard against confusion is a prompt-borne disclaimer, and its own pilot found the protocol prompt-sensitive. Long narrative also sits in the embedding regime where the failure survey measured the widest geometry variance.

Generated episodes are therefore deferred. The capability reopens when an evaluation shows encoding-side value over source-window retrieval at matched source coverage, and measures temporal, aggregation, update, privacy, invention, cost, log volume, and audience non-interference. The evaluation also measures how many semantic writes the strict ancestry rule blocks through delivered replies, as the [episodic wall](#the-episodic-wall) describes. A passing aggregate score is not sufficient: the four wall scenarios below must pass, and no generated detail may become an Assertion, an Attestation, or a hidden-content signal ([evolution](program/evolution.md#deferred-capabilities)).

## The episodic wall

The wall is a hard critic in [verified writes](verified-write.md#hard-critics). It is stated over the [context manifest](statements.md#context-manifest):

A semantic write is rejected when any context manifest in its Activity ancestry contains an `episodic_reconstruction` Artefact.

The ancestry of a write is its own Activity's model-call manifests, followed transitively through the producing Activity of every object those manifests contain. An inbound Occasion is an external input with no producing Activity, so the walk stops at it. An outbound Occasion has a producing Activity, the turn that delivered it, and the walk continues through it. The mark therefore propagates through delivered Occasions. This differs from the restriction projection, which stops at a delivered Occasion and takes only its delivered audience ([privacy and provenance](privacy-and-provenance.md#influence-from-context-manifests)). Delivery settles who may know a reply's content. It does not make invented content evidence. Original evidence in the same context does not cancel the mark, and a model's statement that it ignored the episode cannot clear it.

The rule closes on itself. An Assertion is itself a semantic write, so no Assertion can have an episode in its ancestry, and rendering Assertions never carries the mark. Notes, intermediates, and agent replies can carry it, because their producing Activities may have rendered an episode.

A conversational reply may read an authorised episode as a labelled reconstruction. The reply is not a semantic write. A later context that renders the reply as conversation history has the episode in its ancestry, so semantic writes from that context are rejected. This is the intended conservative consequence: a reply composed from an episode can repeat invented detail, and the agent restating it would launder the detail into structure.

The consequence is also an open question for the generated-episodes reopen condition. Conversation history renders earlier replies into every later turn, so a strict ancestry check blocks semantic writes for the rest of any conversation in which an episode was read. The reopen evaluation measures that cost and decides whether a narrower rule is safe. No narrower rule is part of the genesis design.

The wall reads only the manifests and Activity input edges, which are genesis data. It has nothing to reject until episodes exist. Enabling generated episodes therefore changes no existing record. The wall depends entirely on manifest completeness: a renderer that bypasses the manifest choke point would let an episode influence a write without leaving a mark.

## Retrieval and deduplication

Retrieval may return semantic Assertions, source Occasions, prior Perceptions, and an authorised episode together through explicit links. An episode is not a fallback whose absence lowers confidence in an otherwise supported Assertion. Source records remain available whether generation ran or not.

[Assertion reuse](statements.md#assertion-reuse) never collapses Occasions. A re-mention keeps the new Occasion and adds its own Attestation, which preserves the redundant anchoring that the study observed supporting self-correction ([findings](research/2026-08-03/dual-trace.md#the-four-findings-that-matter-to-us)). Similar Event descriptions remain separate Events, because no Event merge exists at genesis ([events and roles](events-and-roles.md#co-reference)).

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `figurative-source-only` | "The failure was a blade that kept rising." | One Occasion. Zero Assertions is valid when decomposition would misrepresent the metaphor. |
| `tool-observation-no-teller` | A tool reports `17.2 °C` with no participant message. | One Activity with a tool observation. Any Assertion is sourced from that Activity. No utterance or human teller is fabricated. |
| `remention-preserves-occasion` | A participant repeats a claim already recorded, with no new validity. | The existing Assertion gains a second testimony Attestation citing the new Occasion. Both Occasions stay addressable. |
| `episodic-wall-direct` | A context renders a generated episode, and the model submits a semantic write. | The episodic-wall critic rejects the write with a teachable error. No Assertion or Attestation is published. |
| `episodic-wall-mixed` | A context renders an episode and authorised original evidence, and the model submits a semantic write. | The write is rejected. Original evidence does not cancel the mark, and a model-declared omission does not clear it. |
| `episodic-wall-note-mediated` | The model reads an episode and records a note. A later context renders the note and submits a semantic write. | The write is rejected. The note's producing Activity puts the episode in the ancestry. |
| `episodic-wall-source-only-control` | A fresh context renders only authorised original evidence, and an independently recorded Activity submits a semantic write. | The wall passes the write to the ordinary [verified-write](verified-write.md) and [write-surface](write-surface.md) checks. |
| `episode-reply-read` | A conversational reply reads an authorised episode. | The reply renders the episode labelled as a reconstruction. The reply is delivered as an outbound Occasion. Semantic writes from any later context that renders that Occasion as history are rejected, although its restriction contribution is only its delivered audience. |
| `episode-suppressed-whole` | An episode's inputs include material the current audience cannot see. | The whole narrative is suppressed. No partial body is rendered. |
| `generated-episode-lineage` | A model generates an episode from two Occasions. | A derived Artefact with `synthetic_generation` production, ordered source edges with exact locators, and the `episodic_reconstruction` classification; no selector, Assertion, Perception, or Attestation is inferred. |
