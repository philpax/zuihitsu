# Coverage

This register maps the current system's observed failures and open GitHub issues to the successor mechanisms that answer them, grades each answer, and records what the design makes worse.

Six of the eleven surveyed failures are closed structurally: the successor representation cannot encode them. Five are answered in design and rest on evidence not yet gathered. Structural closure does not mean that extraction, policy, or model behaviour is correct. The failures are those recorded in [`../docs/ontology-failures/2026-07-23.md`](../docs/ontology-failures/2026-07-23.md). [Confidence](confidence.md#evidence-map) grades the evidence behind each mechanism, and [evolution](evolution.md) names the experiments that gather what is missing.

## The eleven failures

| Failure | Classification | Mechanism | Evidence required | Residual risk |
|---|---|---|---|---|
| [Facts are sentences](../docs/ontology-failures/2026-07-23.md#facts-are-sentences) | Closed structurally | [Propositions, Assertions, and Attestations](statements.md) separate content, situated validity, and support. One Occasion grounds many Assertions, and source prose stays on the Occasion. | `extraction-convergence` by coordinate, `write-surface-comparison`, and compound-utterance scenarios | Extraction can assign wrong structure. Source-only content is intentionally less queryable. A false grounding rejection or an unconfirmed relation turns teller support into a restricted derivation. |
| [One event, one subject, many copies](../docs/ontology-failures/2026-07-23.md#one-event-one-subject-many-copies) | Closed structurally | A stable [Event](events-and-roles.md) owns independently attested role and attribute Assertions. An explicit re-mention attests the existing Event; an ambiguous one mints a new Event. No Event merge exists. | Duplication and partial-projection scenarios | Ambiguous re-mentions leave duplicate Events, whose cost is unmeasured until `event-coreference` reopens. Role tails are inconsistent even among experts. |
| [Relations are bare edges](../docs/ontology-failures/2026-07-23.md#relations-are-bare-edges) | Closed structurally | A relation Assertion carries validity, provenance, audience, frame, polarity, and modality. Its versioned definition carries domain, range, and constraints. | Encoding and historical-definition scenarios | A wrong but well-typed relation can pass. |
| [Schedule and description conflate](../docs/ontology-failures/2026-07-23.md#schedule-and-description-conflate-in-the-temporal-model) | Closed structurally | Occurrence is an Event attribute Assertion. Only an active agent-authored [Task](time.md#occurrence-and-task) fires, and `wake_turn` starts an ordinary turn. | Dated-description and Task scenarios; the milestone 4 temporal policy | No surveyed peer reports this failure. Uncertainty, timezone ownership, and recurrence policy need validation. |
| [Relation schemas are immutable and vocabulary drifts](../docs/ontology-failures/2026-07-23.md#relation-schemas-are-immutable-and-vocabulary-drifts) | Immutability closed structurally; drift answered in design, not yet validated | Versioned definitions use deprecate-and-alias without rewriting history. [Agent coinage under critics](relations.md#coining-under-critics) contains drift. | Alias-cycle and historical-replay scenarios; `relation-coinage-replay` | Coinage critics can miss a near-duplicate or teach falsely, and depend on embedding geometry. |
| [Identity complexity leaks into behaviour](../docs/ontology-failures/2026-07-23.md#identity-complexity-leaks-into-behaviour) | Closed structurally in the representation; the behavioural fix remains inference | Reads present one resolved handle under the environment at their frontier, and writes land on the primary. Recall-cleared content renders only into operator diagnostics. | One-handle, clearance, and severance scenarios | A mistaken accepted composite still affects recall. The 0.30 relay failure supports the need; that one handle removes the behaviour is an inference. The recall rule depends on the rendering choke point. |
| [Identity is binary and entangled with storage](../docs/ontology-failures/2026-07-23.md#identity-is-binary-and-entangled-with-storage) | Answered in design, not yet validated | Stubs and agent-minted person Entities survive. [Resolution hypotheses](identity.md#resolution-hypotheses) with `recall` or `disclosure` clearance are withdrawable without consuming source identities, and severance lists composite writes for re-homing. Autonomous scoring is deferred. | Overlap, clearance, severance, and re-homing scenarios; the evidence `autonomous-identity` names | Composite thresholds, one-handle behaviour, recitation attacks, and re-derivation cost are unvalidated; re-homing is operator labour. |
| [Belief has no credence model](../docs/ontology-failures/2026-07-23.md#belief-has-no-credence-model) | Answered in design, not yet validated | Attestations keep source kind, expression strength, dependence lineage, and audience. [Support](belief.md#support-projection) is a versioned ordinal, and [settlement](belief.md#settlement-and-withdrawal) a per-audience read-time projection, so a single-teller fact is recallable the next turn. Fusion is deferred. | Shared-room, relay, hidden-support, and withdrawal scenarios; `dependence-lineage` | Arithmetic and truth-directed interpretation are unsettled. No claim yet has two independent human tellers. |
| [Hygiene thresholds are embedder geometry](../docs/ontology-failures/2026-07-23.md#hygiene-thresholds-are-embedder-geometry) | Answered in design, not yet validated | Canonical Proposition equality removes embedding thresholds from deduplication. Retrieval stays a separate projection fused by rank position. | `extraction-convergence` and `query-classification`; the supersession-proposal fallback | Retrieval and the near-duplicate critic still use geometry and need recalibration per embedder. Human-recall transfer to agent salience remains an analogy. |
| [Load-bearing behaviour is prompt-sensitive](../docs/ontology-failures/2026-07-23.md#load-bearing-behaviour-is-prompt-sensitive) | Answered in design, not yet validated | A durable [proposal transaction](verified-write.md#proposal-states) and hard critics make omission, rejection, and acceptance explicit. Candidates A and B move load-bearing checks out of wording. | `write-surface-comparison` and `log-and-console-budgets` | Forced choice moves variance into junk fill. A failed gate requires simplification or withheld authority. |
| [The neural writer is unverified](../docs/ontology-failures/2026-07-23.md#the-neural-writer-is-unverified) | Answered in design, not yet validated | Typed [hard critics](verified-write.md#hard-critics) separate structure, authority, audience, and grounding. The teller must have produced the cited span, grounding checks values and references, and an unconfirmed relation fails closed to a derivation. Publication is atomic or `source_only`. | `write-surface-comparison` with seeded laundering, `principle-assignment`, gold fixtures, and drift monitoring | Critics establish well-formedness and limited groundedness only. A well-typed falsehood can pass, and relation confirmation is only as good as its soft critic. |

The six structurally closed failures count the schema failure by its immutability half, since the critics that contain drift are unmeasured, and identity leakage by its representation, since its behavioural fix remains an inference ([confidence](confidence.md#evidence-map)).

## Regressions

Each regression is named so that it is measured rather than discovered.

Context manifests are a single silent failure point. Influence, restriction, the episodic wall, access accounting, erasure review, and the recall rule all depend on the choke point, and a renderer that bypasses it under-taints undetected. Completeness must be structural: one path from stored content to a model context, with a scenario per renderer kind.

The testimony floor costs recall. A fact told in a direct message stays in that message's audience unless the teller grants wider sharing or the operator widens it ([testimony principle floor](privacy-and-provenance.md#testimony-principle-floor)). `principle-assignment` measures the cost, and option b reopens if it is too high.

Grants and relation confirmation lean on soft critics. Both fail closed, so a missed confirmation loses recall rather than leaking, but a false confirmation still widens or launders. `principle-assignment` and `write-surface-comparison` measure both directions.

The delivered-audience stop is a declassification point. Restriction does not accumulate across a conversation, which is sound only while the [pre-delivery check](privacy-and-provenance.md#pre-delivery-check) is sound: a reply that leaked is afterwards treated as cleared. The check also costs answers, since a draft that fails twice ends the turn with a non-disclosing deferral. `taint-breadth` measures drift towards teller-only.

Coined relation names and minted handles keep their minting context's restriction until cleared or published, so vocabulary coined in a direct message is not yet reusable in a group.

The subject guard costs recall. Under every candidate a person, and any group containing them, loses facts others recorded about them without scoped witness evidence. `guard-denial-rate` measures the three candidates.

Read-time settlement makes single-teller claims default-readable. This fixes amnesia, since no claim in the corpus has two human tellers, but a single teller's error is recalled, as `single_source`, until corrected.

Duplicate Events accumulate without an Event merge: a happening described twice reads as two Events until `event-coreference` reopens.

Erasure sends co-rendered dependants, including delivered replies, to operator review rather than invalidating them. `erasure-closure-size` measures that labour.

Agent coinage reintroduces geometry: the near-duplicate critic compares embedded descriptions, and its threshold needs recalibration on every embedder change.

The console and log cost grows. See #66 below.

Generated episodes are deferred, but reopening them brings three regressions their reopen condition must measure. Narrative generation is a prompt-borne load-bearing surface: the prior art's pilot found its protocol directive overridden by the model's trained default until moved into the system prompt. Narrative is a geometry-sensitive index: the failure survey measured its widest similarity variance in the long-text regime a narrative occupies, 0.80 against 0.94 for the same content under different prefixes. Narrative licenses invention, because it asks a model to commit to concrete detail it was not told, and the current instance has already produced the unelaborated form of that failure. This is why the [episodic wall](two-traces.md#the-episodic-wall) is a critic over manifests and exists from genesis.

## Mitigations the design erases

A mitigation in the current ontology is evidence of a workaround tax the redesign should erase. Three become structure:

- The write-time cross-subject advisory, which steers the agent around the one-subject representation, becomes the [Event](events-and-roles.md). Nothing is left to steer around.
- The third-party-routine rule, which teaches the model not to stamp a recurrence on someone else's job, becomes the absence of a [Task](time.md#occurrence-and-task): a description cannot fire.
- The current-day guard's text check, which asks whether the source utterance could have supplied today's date, becomes the [grounding critic](verified-write.md#hard-critics). The current system calls that check a heuristic and accepts false suppressions in both directions; a span matched against the source is a property of the text.

The first two move load-bearing behaviour into structure. The third replaces a heuristic with a decidable check, a weaker kind of erasure.

## Issues

GitHub state was checked on 2026-08-29 and 2026-08-31. An answer here does not close a live issue against the current deployment.

| Issue | Effect | Where |
|---|---|---|
| #1, #72, #75, #96, #99, #118, #119, #120, #18, #129 | Not addressed: current implementation, tooling, or research | None |
| #7 memory landscape | Answered obliquely: the surveys discharge the data-model half | [survey](research/2026-07-24/lanes/survey-issue7.md) |
| #15 self-observations | Answered obliquely: agent observations are Assertions; the charter is configuration | [memory typology](memory-typology.md#the-self-and-directives-are-configuration) |
| #20 autonomous activity, open | Answered obliquely: `wake_turn` at genesis; `proactive-initiation` deferred | [time](time.md#occurrence-and-task) |
| #42 relation schemas cannot be edited, open | Addressed: versioned definitions, deprecate-and-alias, coinage under critics | [relations](relations.md) |
| #44 long-document ingestion, open | Deferred as `bulk-ingestion`; conversational reading with span citation is at genesis | [artefacts](artefacts-and-perceptions.md#reading-a-document) |
| #58 procedural memories, open | Addressed; automatic extraction deferred | [memory typology](memory-typology.md#procedural) |
| #59 persistent scratchpad, open | Addressed: working notes with per-note taint | [memory typology](memory-typology.md#working) |
| #66 console replica budget | Made worse: genesis-blocking budget | `log-and-console-budgets` ([evolution](evolution.md#milestone-1-evidence)) |
| #74 search past conversations, open | Addressed: a source-text lane under Occasion restriction | [query surface](query-surface.md#search-lanes) |
| #90 eval corpus redundancy | Addressed as evidence discipline: preregistered measurements against a baseline | [evolution](evolution.md#preregistration) |
| #93 challenge-response for merges | Answered obliquely: challenge-response may grant `disclosure` | [identity](identity.md#recall-and-disclosure-clearance) |
| #94 autonomous identity unification, open | Deferred as `autonomous-identity`, first planned after the successor lands | [identity](identity.md#evidence-and-authority) |
| #97 platform-name mint race | Right fix changes: the agent mints no platform stub; the general name race is untouched | [identity](identity.md#permanent-stubs) |
| #100 in-block neural calls, open | Addressed: recorded Activities without commit authority | [verified write](verified-write.md) |
| #103 typed dates and durations, open | Addressed: typed temporal values | [time](time.md#typed-temporal-values) |
| #104 merged identity fails to relay sibling history, open | Addressed for a new instance; the behavioural fix is inference | [identity](identity.md#the-operational-wall) |
| #105 API reference cost | Answered obliquely: a write-surface cost constraint | `write-surface-comparison` |
| #106 volatile facts never age, open | Addressed: volatility policy without source mutation; automation deferred | [time](time.md#staleness-and-volatility) |
| #109 Discord reply context | Right fix changes: a connector-supplied `in_reply_to` link between Occasions | [object model](statements.md#occasion) |
| #110 Discord attachments, closed | Current-system baseline; see below | [artefacts](artefacts-and-perceptions.md) |
| #112 episodic session recaps, open | Deferred as `generated-episodes` | [two traces](two-traces.md#evidence-and-deferral) |
| #113 undated events stamped with the assertion day | Fixed in the current system; unestablished validity stays open | [time](time.md#validity-observation-and-recording) |
| #114 fabricated content attributed to a teller, open | Addressed for a new instance: teller binding, testimony grounding, and fail-closed relation confirmation | [verified write](verified-write.md#hard-critics) |
| #115 date correction needs a full-text supersede, open | Addressed for a new instance: supersession leaves source intact | [time](time.md#correction-and-world-change) |
| #116 structured agent name, open | Addressed: self configuration | [memory typology](memory-typology.md#the-self-and-directives-are-configuration) |
| #121 ontology string literals, open | Addressed: registered definitions and enums | [relations](relations.md#all-ontology-definitions-are-versioned) |
| #123 audience and participation conflation, open | Inherited, not solved: genesis-blocking; see below | [witness evidence](privacy-and-provenance.md#witness-evidence) |
| #124 brief-visible fact refusal, open | Addressed for a new instance by central audience resolution; blocked by #123 in the current system | [query surface](query-surface.md#reads-are-pure-functions) |
| #125 occurrence dated to another referent | Fixed in the current system; the design uses the referential frame | [object model](statements.md#frame) |
| #126 brief names a participant by arrival stub | Fixed in the current system; one resolved handle | [identity](identity.md#the-operational-wall) |
| #127 per-read-path redaction, open | As #124 | [query surface](query-surface.md) |
| #132 blob-reading API, open | Addressed: audience-checked `inspect` with selectors and recorded Perceptions | [query surface](query-surface.md#source-retrieval-and-reinspection) |

These answers hold for a new instance, not the running one. Six rows are live bugs a built successor does not fix retroactively: #104, #106, #114, #115, #124, and #127. Each stays open until fixed there or the deployment is replaced, which argues against over-investing in the old model, not for treating them as handled.

Three current-system fixes landed on 2026-08-06 ([current-system fixes](research/2026-08-06/current-system-fixes.md)): span justification cut dated occurrences by roughly 40% on one field (#113), withdrawal without substitution disarms a misdated occurrence (#125), and the brief renders a participant under the class primary (#126). The first becomes the grounding critic, generalised to every value, and the second a rule for [off-turn jobs](off-turn.md#authority-classes).

#66 is genesis-blocking. The console holds the whole log in browser memory and re-folds from zero on every scrub. Model calls are already 96% of payload bytes, and the design adds extraction calls, manifests, and proposal records. Everything in the fold, such as per-audience state and settlement and alias resolution, is paid per scrub. A preregistered budget for bytes per turn, fold time, and replay cost gates milestone 3.

#123 is inherited and blocks genesis. The connector must distinguish availability from witnessed presence, preserve `availability ⊇ presence`, record witness scope, supply the roster and later-joiner semantics `channel(C)` needs, and default disclosure to teller-only when it cannot demonstrate more. #124 and #127 cannot pass in the current system while #123 remains: a richer condition evaluated against the wrong set is a more expressive leak. `witness-assurance-audit` covers it.

#110 sets the inbound baseline. The Discord relay transfers files and announces transfer and size-cap failures ([relay](../platform-connectors/discord/src/bot/attachments/relay.rs), [connector documentation](../platform-connectors/discord/README.md#behaviour)). The server records filename, media type, hash, length, and classification on `ConversationTurn` ([`AttachmentKind`](../crates/core/src/attachment.rs)), renders supported images, and inlines bounded text ([attachment rendering](../src/agent/turn/attachments/mod.rs)). `ModelCalled` keeps image hash and MIME type but no Perception, selector, reference authorisation, or lineage. The blob store is backed up rather than reconstructed, and its route treats a hash as a bearer capability, both accepted for a server not publicly exposed ([storage contract](../docs/events-and-storage.md#blob-store)). The successor records Artefacts, references, Perceptions, and selectors from genesis and authorises by reference ([artefacts and perceptions](artefacts-and-perceptions.md)); #132 is answered by `inspect`.
