# The query surface

Reads operate on audience-resolved projections. The caller never receives hidden records for local filtering. [The object model](statements.md) owns object identity and the context manifest, [privacy and provenance](privacy-and-provenance.md) owns audience rules, and [identity](identity.md) owns resolution.

## Reads are pure functions

A read is a pure function of the query, the frontier, an explicit as-of time, and the complete audience. The issuing Activity records the frontier and the as-of time. The ResolutionEnvironment and the projection and policy versions are recomputed from the log at that frontier, and the as-of time anchors recency decay, so replay reproduces the same ranking without reading the wall clock. A read has no lifecycle. Audience resolution precedes ranking, so hidden records never enter a conversational read's candidates or change a visible result's rank, count, or explanation.

The current system has leaked withheld metadata when one of its several read paths applied an incomplete policy ([current leak](research/2026-08-06/current-system-fixes.md#redaction-decided-per-read-path), [current visibility contract](../docs/visibility.md), [brief composition](../docs/conversations-and-briefs.md)). One central resolver follows from that observation. Its exact inputs and projections are design synthesis for the reference model to validate.

## Access accounting

Access is content rendered into a model context: a model call's accessed objects are exactly its manifest entries, and recency and frequency are projections over manifests. Candidate generation, hidden matches, rank fusion, and operator diagnostics render nothing and are not accesses. The projection counts distinct objects per turn, so a retry does not inflate frequency. A transient reranker call records a manifest for influence, and the access projection excludes it because its output cannot reach a response or a write.

Access recency and frequency are per audience. A read for audience A counts only accesses in contexts whose audience contains A, since anything rendered there was already known to every member of A. Counting an access from Quinn's one-to-one conversation in a read for Quinn and Rowan would let the ranking reveal what Quinn discussed alone. Decay is computed at the read's as-of time ([memory typology](memory-typology.md)).

Current brief composition shows that content can enter context without an explicit semantic read, so this access unit is a decided policy rather than an observed current invariant ([brief composition](../docs/conversations-and-briefs.md), [log measurements](research/2026-08-06/log-measurements.md)). A missing manifest entry under-counts access as silently as it under-taints ([manifest completeness](privacy-and-provenance.md#influence-from-context-manifests)).

## Audience-resolved Assertions

A structured read returns Assertions whose support, validity, state, and [settlement](belief.md#settlement-and-withdrawal) are folded from the Attestations and transitions visible to the complete audience ([audience-safe state](privacy-and-provenance.md#audience-safe-state-and-zero-residue)). It does not reveal that hidden Attestations exist. The [subject guard](privacy-and-provenance.md#subject-guard) runs before ranking.

An Event read applies the disclosure-safe projection: omissible roles can be omitted, and an explicit incomplete shell or suppression replaces an omission that would manufacture a stronger or false proposition ([events and roles](events-and-roles.md#disclosure-safe-projection)).

Only disclosure-cleared identity composites reach response-affecting context ([identity](identity.md#recall-and-disclosure-clearance)). Conversational reads expose no sibling stubs, candidate merge counts, or hidden merge evidence.

## Structural queries

Structural questions traverse typed records:

- a role query answers who took part in an Event;
- a validity query answers when a Proposition held;
- a shared-Event query answers what happened between resolved entities;
- a transition query answers what changed across correction, supersession, promotion, or retraction;
- a lineage query answers how a result was produced.

These traversals need no model. Whether the set covers what the agent actually asks is untested; Milestone 1's query classification tests it. A lineage response is complete only when the typed inputs ([derivation inputs](verified-write.md#derivation-inputs)) and the producing manifests account for every influence. Otherwise it is an audit trace that names the unrecorded boundary. Neither is presented as a proof of truth.

## Search lanes

Search combines structural proximity, source text, semantic indexes, and artefact metadata. Each result is labelled with its lane:

- `human_utterance` for a span of an Occasion's text part, returned under that Occasion's [restriction](privacy-and-provenance.md#occasion-restriction);
- `structural_assertion` for Proposition or Assertion fields;
- `perception` for OCR, captions, or other model or tool observations;
- `episodic_reconstruction` for a generated episode held as a derived Artefact;
- `artefact_metadata` for mechanically known metadata.

A `visual_embedding` lane is deferred with visual retrieval ([artefacts and perceptions](artefacts-and-perceptions.md)).

The label says why the result matched and does not change its provenance: OCR and captions stay Perceptions. An authorised episode is readable in replies and then blocks semantic writes from that context ([the episodic wall](two-traces.md#the-episodic-wall)).

Rank fusion combines the lanes' versioned rank positions, not incomparable raw scores. It is corroborated as a production retrieval shape and adopted for its embedder independence; no surveyed gain is a target ([production-system survey](research/2026-07-24/lanes/survey-issue7.md), [dual-trace retrieval evidence](research/2026-08-03/dual-trace.md)). The lane set, weights, and reranker boundary are design policy. A transient reranker sees only the resolved head, and its output is never stored evidence. Similarity and reranking never authorise a merge, settlement, or disclosure.

### Embeddings

A vector is a recorded output of an embedding Activity, keyed by embedder version and stored in erasable payload, and is erased with it ([dependant closure](privacy-and-provenance.md#dependant-closure)). A read's query vector is recorded by the read's Activity. Replay reads recorded vectors and makes no embedder calls. An embedder change re-embeds as a recorded off-turn job, and a lane ranks only vectors of one embedder version.

## Relations in use

The relations rendered at the point of writing ([coining under critics](relations.md#coining-under-critics)) are computed per entity kind from Assertions visible to the context's audience. A relation used only in Assertions the audience cannot see is absent from the list, and a coined name follows the [coined-name restriction](privacy-and-provenance.md#occasion-restriction).

## Source retrieval and reinspection

A result can return a source reference and an existing visible Perception without reading bytes. Original bytes require an explicit `inspect`, never run implicitly, which checks the audience against live authorised references and records an Activity naming the selector and pipeline, with any resulting Perception or derived Artefact.

## Operator traces

An authorised operator can inspect candidate generation, audience decisions, identity resolution, ranking contributions, and suppression reasons. A trace is an operator Activity and lends no authority to a later conversational result. The agent receives only the resolved result, with no hidden cardinality, hidden IDs, suppressed ranks, or sign of a denied candidate.

## Errors

A query error names the field, definition, policy class, or ambiguity that prevented resolution, and can suggest a narrower range, an explicit frame, a known handle, or `inspect`. A denial never distinguishes an empty result from a hidden match. The surface omits raw similarity scores, numeric support, merge internals, hidden Attestations, and unfiltered derivation inputs, which would move privacy and identity policy into prompt behaviour.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `read-deterministic` | A query resolving a handle through an identity composite runs live and again under replay. | The Activity records only the frontier and as-of time; both results, including composite, ranks, and decay, are identical. |
| `hidden-candidate-no-rank-effect` | A confidence hidden from the audience is the closest match. | The results and order match a store without it. |
| `hidden-candidate-not-accessed` | A hidden and a visible record match; the visible one is rendered. | Only the rendered record is in the manifest and access recency. |
| `access-recency-audience-scoped` | An item is rendered in Quinn's one-to-one conversation; a later search runs for Quinn and Rowan. | The group read's recency ignores that access; a read for Quinn alone counts it. |
| `retry-no-double-access` | A model call is retried within one turn over the same content. | Both manifests are recorded; frequency counts each object once. |
| `reranker-not-access` | A reranker call sees a search's resolved head. | Its manifest is recorded; the head's access recency is unchanged. |
| `denial-indistinguishable` | One query matches only a hidden record; a control matches nothing. | Both responses are identical. |
| `perception-lane-label` | An OCR Perception matches a search. | The result is labelled `perception`, never `human_utterance`. |
| `source-lane-occasion-restriction` | A `human_utterance` match is in Rowan's direct message; the read is for a channel. | It is not returned. |
| `replay-no-embedder-calls` | A log with semantic searches is replayed. | Every vector comes from recorded payload; the replay makes no embedder calls. |
| `embedder-change-reembeds` | The operator configures a new embedder version. | A recorded job re-embeds; meanwhile each lane ranks one version only. |
| `relations-in-use-audience` | A relation appears only in a confidence; a channel write renders relations in use for that entity kind. | The relation is absent from the list. |
| `inspect-explicit-only` | A search returns an image reference. | No bytes are read and no Perception is created until `inspect` is called. |
| `structural-query-no-model` | The agent asks who took part in a recorded Event. | The role query answers from typed records with no model call. |
| `lineage-audit-boundary` | A lineage query reaches an Activity with incomplete inputs. | An audit trace naming the boundary is returned. |
| `operator-trace-no-widening` | An operator views a trace including a confidence; the agent then answers a participant. | The answer omits the confidence unless it clears the participant. |
