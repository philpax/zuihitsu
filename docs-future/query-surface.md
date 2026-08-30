# The query surface

Reads operate on audience-resolved projections. The caller never receives hidden records for local filtering. [The assertion model](statements.md) owns Assertion, Attestation, Derivation, and transition identity. [Privacy and provenance](privacy-and-provenance.md) owns audience resolution. [Identity](identity.md) owns the unified ResolutionEnvironment.

## Read state machine

The current system has multiple read paths and has leaked withheld metadata when one path applied an incomplete policy ([current leak](research/2026-08-06/current-system-fixes.md#redaction-decided-per-read-path), [current visibility contract](../docs/visibility.md), [brief composition](../docs/conversations-and-briefs.md)). Central audience resolution follows from that observation. The exact stable read ID, lifecycle, delivery accounting, and deduplication key below are design synthesis. Stage 2 non-interference, replay, retry, and cancellation fixtures must validate them before they become authoritative.

Each read has a stable read ID. Its append-only states separate candidate generation from disclosure and access accounting.

| State | Recorded content | Visibility and accounting | Permitted successor |
|---|---|---|---|
| `requested` | query shape, caller, complete audience membership, purpose, policy versions, unified ResolutionEnvironment, witness/presence assurance snapshot, source head, authoritative erasure-ledger position, managed-live tombstone state, and readable retention authority | no content has been disclosed | `resolving`, `denied`, `cancelled` |
| `resolving` | projection and search versions used to generate candidates plus the canonical authorization-input digest | candidates remain inside the audience resolver; candidate generation is not an access | `resolved`, `denied`, `cancelled` |
| `resolved` | visible result IDs, result kinds, ranks, redacted operator trace reference, and the authorization-input digest that licensed them | no access is counted until content is rendered | `rendering`, `superseded`, `cancelled` |
| `rendering` | the exact result fields selected for model or user context | access is pending | `rendered`, `failed` |
| `rendered` | a digest of the exact content envelope delivered, destination kind, and completion status | access recency and frequency update once for each delivered object | terminal |
| `failed` | failure phase and whether any content was delivered | only content confirmed as delivered is counted | terminal |
| `denied` | the failed policy class without hidden object IDs or counts | no access is counted | terminal |
| `superseded` | the replacement read ID | no access is counted for the superseded read unless it already rendered content | terminal |
| `cancelled` | cancellation actor and phase | only previously delivered content is counted | terminal |

A retry after failure uses a new attempt under the same read ID. Reuse requires byte equality of a canonical authorisation-input digest over: the caller and complete ordered audience membership; authentication and identity-assurance records; the complete ResolutionEnvironment and its hypothesis/status versions; every witness, presence, delivery, acknowledgement, and consent record consulted; transmission, subject-guard, support, Event-projection, and purpose policy versions; source and projection heads; the independently authoritative erasure-ledger position and freshness proof; and the managed-live tombstone, readable-reference, pending-deletion, and retention state for every candidate source. The retry recomputes this digest immediately before rendering. Any changed input, unprovable ledger freshness, pending or blocked erasure, withdrawn reference authority, or missing record discards the resolved envelope and returns to `resolving` or `denied`; source and policy heads alone are insufficient. If a fresh retry renders the same envelope again, the log records another delivery, but the access projection can deduplicate repeated delivery to the same model-call context by `(read_id, object_id, destination_id)`. A superseded or hidden candidate does not affect access recency.

Retry fixtures vary one input at a time: audience membership, caller authentication, tentative identity membership, witness presence, consent revocation, policy version, source head, authoritative ledger position, ledger freshness, tombstone state, reference authority, pending physical deletion, and retention state. Every variation forces re-resolution. A missing freshness proof denies. A control with an identical digest may reuse the envelope and produces the same rendered digest.

The access unit is content actually rendered into model or user context. Candidate generation, hidden matches, internal rank fusion, and operator-only diagnostics are not accesses. Current brief composition demonstrates that content can enter context without an explicit semantic read event, so this unit is a decided policy rather than an observed current invariant ([brief composition](../docs/conversations-and-briefs.md), [log measurements](research/2026-08-06/log-measurements.md)). Stage 0e classifies real turns, and Stage 2 replay fixtures must prove that retries, supersession, and hidden candidates update no recency except for confirmed delivery.

## Audience-resolved Assertions

A structured read returns Assertions aggregated from Attestations visible to the complete audience. It returns the folded Assertion status under named transition, support, ontology, policy, and ResolutionEnvironment version. It does not reveal that hidden Attestations exist. Hidden support cannot change an agent-visible ordinal, rank, explanation, or action unless the result inherits the hidden transmission restriction.

An Event read applies the Event type's disclosure-safe projection. The resolver can omit independently omissible role Assertions. It returns an explicit incomplete shell or suppresses the Event when omission would manufacture a stronger or false proposition. [Events and roles](events-and-roles.md) owns these projection rules.

Identity resolution produces an operational handle under a named resolution environment. Conversational reads do not expose sibling stubs, candidate merge counts, or hidden merge evidence. Authorised operators can request a separate resolution trace.

## Structural queries

Structural questions traverse typed records:

- an agent-role query answers who participated in an Event;
- an Assertion validity query answers when a Proposition held;
- a shared-Event query answers what happened between resolved entities;
- a transition query answers what changed across correction, supersession, promotion, or retraction;
- a lineage query answers how a result was produced.

These queries do not require a model. Their response records the source head sequence and all projection versions needed to reproduce the result.

A lineage response distinguishes complete lineage from an audit trace. Complete lineage lists every typed input, including positive Assertions, scoped negative query results, aggregates, tool observations, Perceptions, ontology and policy versions, the identity-resolution environment, assumptions, implementation or criterion version, and source head sequence. An audit trace names the recorded boundary when an older or external activity lacks complete inputs. The surface does not describe either form as a proof of truth.

## Search result kinds

Search combines structural proximity, source text, semantic indexes, artefact metadata, and gated multimodal indexes. Every result labels the matched lane:

- `human_utterance` for an Occasion text span;
- `structural_assertion` for Proposition or Assertion fields;
- `perception` for OCR, captions, or other model or tool observations;
- `visual_embedding` for a gated cross-modal index;
- `artefact_metadata` for mechanically known metadata.

The label identifies why the result matched. It does not change the result's provenance. Generated OCR and captions remain Perceptions rather than human utterances. [Artefacts and perceptions](artefacts-and-perceptions.md) owns these distinctions.

Signals produce versioned rankings. Rank fusion combines rank positions rather than incomparable raw scores. Rank-order fusion is corroborated as a production retrieval shape, but no surveyed gain is adopted as a target ([production-system survey](research/2026-07-24/lanes/survey-issue7.md), [dual-trace retrieval evidence](research/2026-08-03/dual-trace.md)). The chosen lane set, weights, and reranker boundary are design policy and remain Stage 0e/Stage 2 gated. A transient model reranker may inspect only the already audience-resolved head. Its output is discarded after the read and cannot become stored evidence. Similarity and reranking remain ranking inputs rather than authority to merge, settle, or disclose.

## Source retrieval and reinspection

A result can return a source reference and an existing visible Perception without reading original bytes again. Original Artefact bytes require an explicit `inspect` operation. `inspect` performs audience and retention checks, records the Activity, names the selector and transformation pipeline, and records any resulting Perception or derived Artefact. It never runs as an implicit consequence of search.

Historical reinspection, OCR, region grounding, and visual embeddings are separately gated capabilities. The permanent result kind and Activity shapes can exist while these policies remain disabled.

## Operator traces

An authorised operator can inspect candidate generation, audience decisions, identity resolution, ranking contributions, and suppression reasons. The conversational agent receives only the resolved result and a non-sensitive explanation. The agent-visible response does not disclose hidden cardinality, hidden object IDs, suppressed ranks, or whether a denied candidate existed.

Operator trace access is itself a rendered read and uses the operator's audience and purpose. A trace cannot use operator authority to widen a later conversational result.

## Errors

A query error identifies the request field, registered definition, policy class, or ambiguity that prevented resolution. It can suggest a narrower time range, an explicit frame, a known handle, or an authorised inspection operation. A denial does not distinguish no match from a hidden match when that distinction would disclose protected state.

The surface omits raw similarity scores, numeric support, merge internals, hidden Attestations, and unfiltered derivation inputs. The substrate owns those decisions because exposing them would move privacy and identity policy into prompt behaviour.
