# The two traces

The two traces are the durable source record and an optional generated mnemonic narrative. Structured Assertions are linked to the source record but are not themselves “the other trace”: they are semantic interpretations with their own lifecycle.

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

## The optional generated trace

A generated episode is a synthetic mnemonic scene or narrative produced from selected source records by an Activity and recorded as a separate Derivation output. It may help distinguish, sequence, or aggregate occasions. It is not raw experience, an Assertion, an Attestation, or evidence.

The supporting study reported gains of 40 points on temporal reasoning, 30 on multi-session aggregation, and 25 on update tracking, with no single-session gain ([dual-trace results](research/2026-08-03/dual-trace.md#the-experiment)). The evidence is narrow: one unreplicated benchmark, an automated judge, about twenty questions per category, no privacy dimension, and no ablation separating encoding-time generation from retrieval-time reconstruction ([limitations](research/2026-08-03/dual-trace.md#limitations-theirs-and-ours)). Reported cost neutrality depended on a context-heavy harness and does not transfer to a durable event log.

Generation also invites concrete invention by design. The study's non-evidence disclaimer and protocol are prompt-sensitive, and long narrative occupies the embedding regime with observed geometry variance. These caveats make generated episodes an open experiment pending Stage 0a, not required genesis state.

## The episodic wall

If generated episodes are enabled, mechanical rules enforce the boundary:

- a generated episode is attributed to its producing Activity and Derivation, never to a participant;
- it cannot be an input to a semantic Assertion Derivation or accrue Attestations;
- it is labelled as reconstruction on every read surface;
- its lineage names all source Occasions, Activities, Perceptions, and policy versions used;
- its transmission restriction is no wider than the intersection of its inputs;
- generation over content whose principle cannot safely govern an indivisible narrative is rejected;
- correction appends a replacement or disables the generated result without editing source records.

A narrative body is indivisible prose. If omitting restricted material would change its account, the whole body is suppressed. Unlike an Event projection, it cannot safely reveal selected “edges”. Central audience resolution must run before the narrative is rendered.

Generated prose cannot claim completeness. It is a selective reconstruction and may omit salient details, combine anchors poorly, or invent scene geometry. A source link lets a reader audit it; the link does not turn it into evidence.

## Retrieval and deduplication

Retrieval may co-return semantic Assertions, source Occasions, prior Perceptions, and an authorised generated episode through explicit links. A generated episode is not a fallback whose absence lowers confidence in an otherwise supported Assertion. Source records remain available whether generation ran or not.

Proposition equality and Assertion lifecycle are defined in [statements](statements.md). They must not be used to collapse Occasions. Re-mentioning a proposition preserves the new Occasion and may add a distinct Attestation with its own teller, expression strength, source locator, audience, and retraction authority.

Event co-reference is stricter still. Similar descriptions remain separate Events unless a reversible resolution hypothesis is accepted under [events and roles](events-and-roles.md). Generated narrative never supplies the evidence needed to merge them.

## Enablement gate

Generated episodes remain disabled unless an in-repository experiment shows encoding-side value over source-window retrieval at matched source coverage. The gate must measure temporal, aggregation, update, privacy, invention, cost, log volume, and audience non-interference. A passing aggregate score is insufficient: fixtures must show that no generated detail becomes an Assertion, Attestation, Event co-reference input, or hidden-content signal.
