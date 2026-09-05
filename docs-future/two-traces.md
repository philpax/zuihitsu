# The two traces

The two traces are the durable source record and a generated mnemonic narrative supplied by `capability:generated-episodes`, whose status is `activation_gate`. Structured Assertions are linked to the source record but are not themselves “the other trace”: they are semantic interpretations with their own lifecycle.

An external social input is an [Occasion](statements.md). It owns one ordered interleaved sequence of text and [ArtefactReference](artefacts-and-perceptions.md) content parts; either kind may be absent. An agent, operator, tool, or model action with no external Occasion is an [Activity](statements.md). [Statements](statements.md) owns these identities and the Proposition/Assertion/Attestation/Derivation cardinalities. This chapter owns only the boundary between source material and generated episodic narrative.

## The source trace

The source trace preserves what arrived or happened without pretending that extraction is lossless. For an Occasion it includes participants, witness evidence, ordering, observed and recorded time, the original utterance when present, and ArtefactReferences. For an Activity it includes actor, inputs, implementation or model version, tool observations, and outputs.

Assertions cite typed source locators into this trace. One compound utterance can ground several Attestations and Assertions; one utterance can also ground none. The [modelling study](research/2026-08-03/modelling-study.md#compound-entries-fragment-and-the-gloss-stops-being-one-to-one) found a single entry carrying eight claims and established that source prose cannot honestly be split into one invented gloss per claim. It also found figurative content for which source prose is the only faithful representation ([figurative content](research/2026-08-03/modelling-study.md#figurative-content)).

Source retention does not assert source content. Quotation, participant Attestation, agent observation, Perception, and Derivation remain distinct under [statements](statements.md). A repeated claim may reuse a Proposition while producing another Attestation or Assertion as appropriate; the second Occasion always remains independently addressable.

### Compound and source-only fixtures

| Input | Required representation |
|---|---|
| “Rowan moved to York and now works at Northwind.” | One Occasion and utterance; at least two source locators; separate Propositions/Assertions/Attestations so validity and audience can differ. No synthetic per-claim utterances. |
| “The failure was a blade that kept rising.” | One Occasion and utterance; zero structured Assertions is valid if decomposition would misrepresent the metaphor. |
| A tool reports `17.2 °C` without a participant message | One Activity and tool observation; any Assertion is sourced or derived from that Activity. No fabricated utterance or human teller. |

## Artefacts, captions, and Perceptions

An artefact share has three possible descriptive layers that must not collapse:

1. The primary source artefact: immutable bytes identified as an Artefact and shared through an ArtefactReference on an Occasion.
2. A human caption or alt text: participant-authored source text on that Occasion, recorded as a text part marked `caption_of` the reference part. It may ground an Attestation, but it is not mechanically true merely because it accompanies the bytes.
3. A machine Perception: a fallible observation such as a caption, OCR result, object label, or region description produced by a versioned model/tool Activity. It is not participant testimony.

A generated thumbnail, crop, page rendering, OCR text file, or extracted frame is a derived Artefact with explicit lineage. The canonical identities, selectors, access checks, and erasure rules belong to [artefacts and perceptions](artefacts-and-perceptions.md).

### Multimodal fixtures

| Scenario | Required result |
|---|---|
| A participant sends an image with no text | An artefact-only Occasion is valid. The ArtefactReference records supplier and ordering. Inspection may create a Perception; arrival alone creates no Assertion. |
| A participant captions an image “Pepper at the harbour”; the model sees an indoor room | Preserve the human caption as source text and any Attestation it grounds; preserve the conflicting machine Perception under its model version. Do not attribute the Perception to the participant or mechanically call the two contradictory without a grounded proposition comparison. |
| The same bytes are shared twice | One Artefact, two ArtefactReferences, two Occasions. Their suppliers, captions, audiences, and retraction/erasure paths remain separate. |
| The agent later reinspects a region | Record an explicit audience-checked Activity consuming the ArtefactReference and selector, then a new Perception. Do not silently replace the earlier Perception. |
| One share is erased while another authorised reference survives | Retract or erase the affected reference and its dependent records; retain bytes only as authorised by the surviving reference. Never use content identity as proof of access. |

## The generated trace activation gate

A generated episode is a derived Artefact containing a synthetic mnemonic scene or narrative. A generation Activity produces it through a Derivation whose tagged typed output names the Artefact, `synthetic_generation`, and the durable `episodic_reconstruction/v1` classification. The Derivation consumes one or more ordered `source_occasion` or `source_activity` edges. Each edge names the exact source locators and audience decision. It does not invent a selector. Machine enforcement preserves the classification on every write and read surface. The episode may help distinguish, sequence, or aggregate occasions. It is not raw experience, an Assertion, an Attestation, evidence, or a fourth Derivation output type. [Artefacts and perceptions](artefacts-and-perceptions.md) owns derived Artefact byte identity and access, and [statements](statements.md#derivation) owns Derivation output cardinality, production tags, source edges, and classification.

The supporting study reported gains of 40 points on temporal reasoning, 30 on multi-session aggregation, and 25 on update tracking, with no single-session gain ([dual-trace results](research/2026-08-03/dual-trace.md#the-experiment)). The evidence is narrow: one unreplicated benchmark, an automated judge, about twenty questions per category, no privacy dimension, and no ablation separating encoding-time generation from retrieval-time reconstruction ([limitations](research/2026-08-03/dual-trace.md#limitations-theirs-and-ours)). Reported cost neutrality depended on a context-heavy harness and does not transfer to a durable event log.

Generation also invites concrete invention by design. The study's non-evidence disclaimer and protocol are prompt-sensitive, and long narrative occupies the embedding regime with observed geometry variance. These caveats make `capability:generated-episodes` an `activation_gate`, pending the `stage:1` generated-episode evidence package; it is not required genesis state.

## The episodic wall

If generated episodes are enabled, machine enforcement reads the durable `episodic_reconstruction/v1` classification from the Derivation's tagged derived-Artefact output and applies it to every rendered reference. The Artefact record continues to represent byte identity and mechanically observed metadata. [Privacy and provenance](privacy-and-provenance.md#influence-envelopes) owns monotone influence propagation and semantic-publication checks.

- a generated episode is attributed to its producing Activity and Derivation, never to a participant;
- the derived Artefact cannot be an input to a semantic Assertion Derivation or accrue Attestations;
- it is labelled as reconstruction on every read surface;
- its lineage names all source Occasions, Activities, Perceptions, and policy versions used;
- its transmission restriction is no wider than the intersection of its inputs;
- generation over content whose principle cannot safely govern an indivisible narrative is rejected;
- correction appends a replacement or disables the generated result without editing source records.

A narrative body is indivisible prose. If omitting restricted material would change its account, the whole body is suppressed. Unlike an Event projection, it cannot safely reveal selected “edges”. Central audience resolution must run before the narrative is rendered.

Generated prose cannot claim completeness. It is a selective reconstruction and may omit salient details, combine anchors poorly, or invent scene geometry. A source link lets a reader audit it; the link does not turn it into evidence.

## Episode-influenced model contexts

The boundary cases use the influence and publication rules in [privacy and provenance](privacy-and-provenance.md#influence-envelopes). [The canonical input vectors](statements.md#canonical-input-vectors) define the exact source edges, classifications, InfluenceEnvelope marks, and expected publication decisions. They are required specification inputs for the future fixture harness and do not claim implementation.

| Fixture | Context and attempted operation | Required result |
|---|---|---|
| Direct | The model context contains a generated episode, and the model submits a semantic write. | Reject the write. The `episodic_reconstruction/v1` influence is non-evidentiary. |
| Mixed | The model context contains a generated episode and authorised original evidence, and the model submits a semantic write. | Reject the write. Original evidence does not cancel episode influence, and a model-declared omission cannot clear it. |
| Note-mediated | The model reads a generated episode, records a note or intermediate, and later submits a semantic write. | Reject the write. Non-evidentiary influence propagates through notes and intermediates. |
| Source-only control | A fresh context contains only authorised original evidence, and an independently recorded Activity submits a semantic write. | Permit the write to proceed to the ordinary [verified-write](verified-write.md) and [write-surface](write-surface.md) checks. |

An ordinary conversational reply may read an authorised episode as a labelled reconstruction. This read does not permit a semantic write from the episode-influenced context.

## Retrieval and deduplication

Retrieval may co-return semantic Assertions, source Occasions, prior Perceptions, and an authorised generated episode through explicit links. A generated episode is not a fallback whose absence lowers confidence in an otherwise supported Assertion. Source records remain available whether generation ran or not.

Proposition equality and Assertion lifecycle are defined in [statements](statements.md). They must not be used to collapse Occasions. Re-mentioning a proposition preserves the new Occasion and may add a distinct Attestation with its own teller, expression strength, source locator, audience, and retraction authority.

Event co-reference is stricter still. Similar descriptions remain separate Events unless a reversible resolution hypothesis is accepted under [events and roles](events-and-roles.md). Generated narrative never supplies the evidence needed to merge them.

## Enablement gate

`capability:generated-episodes` remains disabled until its independent `activation_gate` passes. The gate must show encoding-side value over source-window retrieval at matched source coverage and measure temporal, aggregation, update, privacy, invention, cost, log volume, and audience non-interference. A passing aggregate score is insufficient: the direct, mixed, note-mediated, and source-only boundary fixtures must pass, and no generated detail may become an Assertion, Attestation, Event co-reference input, or hidden-content signal.
