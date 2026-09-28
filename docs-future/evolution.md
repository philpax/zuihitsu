# Evolution

This chapter defines the path from the current instance to the successor's first real genesis, and lists the capabilities genesis leaves deferred. A capability is either in the genesis design or deferred.

The current instance's agents are frozen and never migrated. Before genesis every successor log, encoding, fixture, and projection is disposable, so no milestone carries a compatibility, rollback, or experimental-state section, while measurements, failures, and rationales are kept as evidence. Each increment leaves the repository buildable with its tests passing, not necessarily a usable agent. The permanence contract begins at the first real genesis ([overview](overview.md#permanence-contract)). The dated snapshots under [`research/`](research/) use earlier stage names and are not rewritten.

## Milestones

| Milestone | Produces | Blocks |
|---|---|---|
| 1a. Labelling vocabulary | A provisional vocabulary and entity policy for milestone 1 | The extraction gold, `principle-assignment`, and `write-surface-comparison` |
| 1. Evidence | Preregistered measurements, an anonymised fixture corpus, and the decisions the chapters leave open | Authoritative publication in milestone 3, and the policy choices in milestone 4 |
| 2a. Thin reference model | An executable model of the privacy core, and a synthetic write surface | `write-surface-comparison`, and milestone 2 |
| 2. Reference model | An in-memory Rust model of the whole object model | Milestone 3 |
| 3. Vertical slice | The audience-resolved read and write path, erasure with the ledger, and stateless off-turn workers | Milestone 4 |
| 4. Policies, then genesis | Initial policies, a genesis freeze, a rehearsal, and the first real genesis | Nothing; genesis is the permanence boundary |

The order is fixed where one result feeds another. `extraction-convergence` and `principle-assignment` run after milestone 1a, whose vocabulary labels their gold. `relation-coinage-replay` runs after `extraction-convergence`, whose relations it consumes. `write-surface-comparison` runs after milestone 1a and after milestone 2a provides its synthetic write surface. Milestone 2a starts at once, and the other experiments can run in parallel with milestones 2a and 2. The anonymised fixture corpus joins the chapters' invented scenarios as reference-model tests when milestone 1 produces it.

## Milestone 1: evidence

Milestone 1 measures the claims the design cannot settle by argument. Every experiment runs on a pinned corpus, fixes its metrics before observing results, and reports a failed criterion as a negative result.

### Milestone 1a: labelling vocabulary

Milestone 1a fixes a provisional labelling vocabulary and entity policy: the relation set, entity kinds, frames, Event roles, modality and polarity conventions, transmission principles, source-kind conventions (`testimony`, `observation`, or `derivation`), when a mention mints an Entity, and the [reported-speech convention](statements.md#mode-and-reported-speech): reported speech is `quoted`, and a Proposition reference is only for an attitude about mental state. It labels the extraction and principle gold and configures both write surfaces. It is versioned in the preregistration, and a new version invalidates earlier gold. It is not a genesis decision: milestone 4 chooses the seed vocabulary and policies.

### Corpus and access

The corpus is the frozen current instance's event log. It is read only through `zuihitsu debug events` (`--summary`, `--type`, `--seq`), which takes no write lock; no write-exception `debug` subcommand runs against it. This is the method of the [2026-08-06 log measurements](research/2026-08-06/log-measurements.md), which found 2,361 events, 198 content entries, 132 executed Lua blocks, 417 recorded model calls, 27 sessions, 7 conversations of which 1 is multi-party, and 3 distinct human tellers. Those counts drifted while the instance ran.

The corpus is pinned in the preregistration by its log sequence range and a digest over it, and a result over another range or digest is not a valid gate result. `baseline-measurements` re-takes the counts over the pinned range, and every other experiment is judged against it.

### Controlled multi-party corpora

The frozen log holds one multi-party conversation and one private-to-teller entry, too little to test witness evidence, dependence, principles, taint, or the subject guard. Controlled corpora supply the cases: shared-room exposure, channel and direct-message restrictions, a relay and its return, a confidence told in a group, a teller grant, and a subject who is present, absent, or silent. Each is scripted with the eval harness (`crates/eval`) using invented placeholders, and is pinned like the real corpus. Its results are reported as scripted, never as observed behaviour, and never pooled with real-corpus rates.

### Anonymisation

The raw log, and any labels, extractions, or model outputs over its unanonymised text, stay outside the repository. A committed fixture reproduces the shape of an observation, never its content: every person, handle, platform ID, place, organisation, identifying date, and biographical detail becomes an invented placeholder such as `person/rowan@direct`, and the operator's own identity never appears. Aggregate counts and rates, and the invented controlled corpora, may be committed. This follows the fixture rule in [`../CONTRIBUTING.md`](../CONTRIBUTING.md#testing-conventions).

### Preregistration

Before any experiment observes a result, a dated preregistration under `research/` fixes its metric definitions, gold data, minimum sample, uncertainty treatment, cost and latency budgets, privacy failure tolerance, and pass rule. It also records the corpus pin, the labelling-vocabulary version, the controlled corpora, any fallback that a partial result triggers, and each gate's default when its minimum sample cannot be reached. That default is always the conservative option: `subject-guard/v1` for the guard, the stricter soft-critic threshold, option b left deferred, and authority withheld from an unmeasured treatment. A usefulness or economics measure is an acceptance criterion whenever the treatment changes that read or write path. A result observed before its preregistration is not a valid gate result. A failed criterion sends the treatment back for revision or withholds its authority.

### Experiments

| ID | Decides |
|---|---|
| `baseline-measurements` | The denominators every later budget uses. |
| `extraction-convergence` | Whether structural equality replaces embedding thresholds for deduplication, or the fallback applies. |
| `principle-assignment` | The grant and relation-confirmation soft-critic thresholds, and whether option b reopens. |
| `write-surface-comparison` | The write surface and the structuring schedule. |
| `relation-coinage-replay` | The coinage distance threshold and the promotion count. |
| `query-classification` | The structural query set, the lane set, and the expected manifest size. |
| `taint-breadth` | Whether the delivered-audience stop suffices. |
| `erasure-closure-size` | Whether typed closure and its review load stay at operator scale. |
| `guard-denial-rate` | The genesis subject-guard policy. |
| `witness-assurance-audit` | The connector witness protocol (#123). |
| `dependence-lineage` | Whether the dependence rules hold before any ordinal reports `corroborated`. |
| `log-and-console-budgets` | Whether the console replica needs snapshots or windowing before genesis (#66). |

Settlement is decided and has no experiment ([belief](belief.md#settlement-and-withdrawal)). Only anonymised fixture sets are committed.

`baseline-measurements` re-takes payload composition, latency, posture and teller mix, memories touched per block, brief sizes, derived-structure counts, and duplication. On 2026-08-06, recorded model calls were 95.9% of payload bytes (31.4 MB of 32.7 MB, a mean of 75 KB per call), and call latency was p50 6.8 s, p90 30.8 s, p99 73.7 s, and max 94.9 s. Posture was 157 public, 40 attributed, and 1 private-to-teller entry; tellers were 120 participant, 77 agent, and 1 seed. No claim was asserted by two distinct human tellers. Memories touched per block had a median of 1 and a maximum of 11, and brief sizes ran from 2,240 to 8,106 characters ([log measurements](research/2026-08-06/log-measurements.md)).

`extraction-convergence` measures whether rewordings of one claim produce one Proposition key, separately for subject resolution, relation choice, and the other coordinates, each with sample size and interval, plus false merges, yield per Occasion, and source-only rate. Its primary gold is sampled independently of the consolidation machinery. The 27 `EntriesConsolidated` and 13 `BeliefArbitrated` events are secondary gold, flagged as biased: the embedding-threshold machinery selected them, so they over-represent rewordings that geometry already catches. The 16 exact duplicates are excluded, because 14 are connector-minted boilerplate and none tests rewording ([log measurements](research/2026-08-06/log-measurements.md#duplication)). It also reports frame and `principal` accuracy on the persona-agent memory that holds 77 entries (39%), the 35 attitude entries including any depth-two case, reported speech kept apart from attitudes under milestone 1a's convention, compound entries with three or more clauses (20), hedged entries (8), plan language (17) ([modelling study](research/2026-08-03/modelling-study.md#the-corpus-at-a-glance)), and how often [Assertion reuse](statements.md#assertion-reuse) records an ambiguous attachment. The fallback for partial convergence is a supersession-proposal off-turn job: it proposes that two live Assertions restate one claim for an authorised reviewer, never substitutes content, and adds an [authority class](off-turn.md#authority-classes) but no record kind.

`principle-assignment` scores, against gold over real Occasions and the controlled corpora, how the model narrows a testimony principle below its [floor](privacy-and-provenance.md#testimony-principle-floor), and how the grant and relation-confirmation [soft critics](verified-write.md#soft-critics) judge their spans. Both gates fail closed, so each threshold's report gives its false-confirmation rate, which widens or launders, and its false-refusal rate, which costs recall. The pass rule bounds false confirmation. The recall cost of option a is the share of facts first told in a direct message that a later conversation needed and could not see; option b reopens when it exceeds its preregistered bound.

`write-surface-comparison` replays the corpus locally through milestone 2a's synthetic surface and runs [candidates A and B](write-surface.md#what-the-comparison-measures) over the same Occasions, target model, and critic versions, measuring fidelity, omission, junk fill (where forced choice moves omission variance), constraint tax, blocks and review rounds per write, latency and log growth against the baseline, retries, source-only usefulness, and eager against deferred structuring. It measures the [testimony-grounding critic](verified-write.md#hard-critics) end to end: rejection of genuine testimony, and catches over seeded cases that render a restricted confidence and cite another teller's span.

`relation-coinage-replay` measures near-duplicate and false teaches, one-off share, promotions, and alias correctness when the current vocabulary (92 `LinkCreated` and 133 `LinksInferred` events on 2026-08-06) and the extracted relations pass through the [coinage critics](relations.md#coining-under-critics). The peer survey's 78% one-off share is the single-source comparison ([issue 7 survey](research/2026-07-24/lanes/survey-issue7.md)). The threshold is recorded with its embedding model version.

`query-classification` records the query each real read needs and whether structural queries, search lanes, or only source text answer it, over explicit reads, the 147 ambient recalls, and briefs. Brief content enters context with no read event, so it also estimates a brief's manifest entries.

`taint-breadth`, `erasure-closure-size`, and `guard-denial-rate` compute restrictions under the [Occasion restriction](privacy-and-provenance.md#occasion-restriction) and principle-floor definitions. The current system records no principles, so they import its posture labels as approximate principles and report sensitivity under the narrowest and widest plausible mappings. The real corpus holds one private entry, so the controlled corpora carry the restricted cases.

`taint-breadth` approximates what each of the 417 recorded calls rendered, with the approximation's error, and computes each output's restriction through earlier replies with and without the delivered-audience stop. It reports the share narrower than the conversation's audience by turn position, since drift towards teller-only grows over turns, and gives the first manifest-shaped measure of working-note taint.

`erasure-closure-size` applies the [typed-dependency closure](privacy-and-provenance.md#dependant-closure), over approximated manifests, to one sampled target per teller and entry kind, and to every scripted case. It counts invalidated dependants, deleted model-call payloads, and co-rendered dependants sent to operator review, the operator's labour per erasure.

`guard-denial-rate` reports the share of results each [subject-guard](privacy-and-provenance.md#subject-guard) candidate denies over explicit reads, ambient recalls, and briefs, by read kind and audience size, with protected positions and witness evidence approximated as preregistered. The pass rule bounds each denial rate, and the privacy tolerance bounds what `subject-guard/v1-relaxed` and `subject-guard/v1-own` may admit.

`witness-assurance-audit` is a desk analysis of which assurance kinds, at which scope, each platform can demonstrate; the samples check it and are not its basis. It covers every consumer of the present set that [witness evidence](privacy-and-provenance.md#witness-evidence) names under teller-only fallback, and whether each platform shows history to later channel joiners, which `channel(C)` needs. It needs no successor substrate and can inform a current-system fix.

`dependence-lineage` counts false independence and dependence when the [dependence rules](belief.md#dependence) place scripted relayed, returned, and shared-room tellings, the only pre-genesis evidence on relay chains.

`log-and-console-budgets` measures bytes per turn from manifests, outbound Occasions, candidate A's extraction calls, and proposal records, plus replay time, eval-package size, and browser fold time at realistic sizes, including a scrub that re-folds from zero. The budget gates the affected slice before implementation.

Generated-episode ablation and multimodal recall belong to [deferred capabilities](#deferred-capabilities).

### Produces, falsifies, blocks

Milestone 1 produces a dated research snapshot (preregistration, raw measurements, failures, and uncertainty), the fixture and controlled corpora, and a recorded decision per experiment. The design is falsified in part if extraction does not converge and the fallback cannot absorb the gap, both write surfaces fail, the grounding critic misses seeded laundering beyond tolerance, no soft-critic threshold bounds false confirmation, outputs drift towards teller-only despite the delivered-audience stop, erasure closure or its review load exceeds operator scale, every guard candidate exceeds its bounds, witness data cannot support fail-closed teller-only resolution, provenance cannot place dependence components, or manifests and extraction exceed the budgets. A high option-a recall cost reopens option b and falsifies nothing. The results block authoritative publication in milestone 3 and the policy thresholds in milestone 4.

## Milestone 2a: thin reference model

Milestone 2a builds a thin executable model of the privacy core: Occasion restriction, the Proposition key, Assertions and Attestations with their transitions, the per-audience fold of validity, state, and settlement, and the hard critics. It covers the rules [privacy and provenance](privacy-and-provenance.md) owns, including the three subject-guard candidates, and the scenarios of the owning chapters are its tests. A disposable synthetic write surface drives it, implements both write-surface candidates, and cannot publish.

From here on, privacy semantics are checked in this model in place of further prose review: a question about them is answered by a passing or failing scenario, and a privacy rule change arrives with its scenario. The design is falsified in part if a privacy scenario cannot be expressed over recorded fields, or a fold shows an audience a value, date, closure, or settlement whose source it cannot see. It blocks `write-surface-comparison` and milestone 2.

## Milestone 2: reference model

Milestone 2 extends the thin model to the whole [object model](statements.md), in memory and correctness-first: every [legal transition](statements.md#legal-transitions) with predecessor checks, manifests and their projections, identity hypotheses with the ResolutionEnvironment recomputed from each recorded frontier, Tasks, Artefacts, erasure closure, and support. It uses no production storage, connector, or agent surface. Every scenario table is its test suite, each test carries its scenario ID, and the [README](README.md#scenario-ids)'s CI check is built here. It is falsified in part if a scenario needs an undefined field, a fold admits a forbidden transition, a critic needs a value no record carries, or a manifest projection cannot be computed from recorded edges. It blocks milestone 3.

## Milestone 3: vertical slice

Milestone 3 builds the smallest complete production path. Occasions are retained with their restrictions. The chosen write surface produces a proposal, the hard critics run, and publication is atomic or ends `source_only`; it appends no settlement record, since settlement is a read-time projection. Every renderer goes through the manifest choke point, and every outbound Occasion passes the [pre-delivery check](privacy-and-provenance.md#pre-delivery-check). Central audience resolution produces a source read or an explicit candidate read, and access accounting records rendered content only, per audience. The slice includes in-server exclusive erasure with the salted envelope and payload split, the ledger and its second copy, and boot reconciliation ([privacy and provenance](privacy-and-provenance.md#erasure-ledger-and-boot-reconciliation)); stateless [off-turn workers](off-turn.md#workers); and `wake_turn` scheduling.

It produces a production-shaped implementation whose folded state matches the reference model on every scenario, and budget measurements against milestone 1. It is falsified in part by a renderer bypassing the manifest, a delivered reply failing the pre-delivery check, hidden residue in a visible result, partial publication, source loss, a stale transition or worker result committing, erased content resurfacing after a restore, or a cost outside budget. Authoritative publication stays disabled until the `extraction-convergence` and `write-surface-comparison` gates pass. It blocks milestone 4.

## Milestone 4: policies, then genesis

Milestone 4 selects the initial policies, freezes the design, rehearses it, and creates the first real instance.

The initial policies are:

- temporal: the initial policy in [time](time.md#genesis-boundary), with the `wake_turn` action;
- Event and relation: the seed vocabulary, the universal parent roles, and the coinage threshold and promotion count ([relations](relations.md#seed-vocabulary)); Events have no co-reference, and milestone 1a informs the seed without binding it;
- identity: operator-confirmed disjoint composites with clearance and a primary member, and the one-handle surface ([identity](identity.md));
- privacy: the subject-guard candidate `guard-denial-rate` selects, the principle floor with the grant and relation-confirmation thresholds from `principle-assignment`, and the connector witness protocol from `witness-assurance-audit` ([privacy and provenance](privacy-and-provenance.md));
- support: the read-time settlement projection, dependence, mechanical contest, and the ordinal ([belief](belief.md#settlement-and-withdrawal)), decided and taking no threshold from milestone 1.

The genesis freeze fixes stable IDs, payload meanings, transition folds, selector and definition versions, the seed vocabulary, and policy versions. It rejects the design when a documented scenario ID has no test, a frozen field has no source, or a budget has no measurement.

The rehearsal runs fresh disposable instances of the frozen candidate through channel and direct-message input, outbound Occasions with the pre-delivery check and reply links, source recording, verified writes, teller grants, audience-resolved reads, correction, retraction, identity severance with re-homing review, temporal safety, attachments and page citation, `wake_turn` reminders, erasure, crash recovery, `restore-old-backup-reconciles-at-boot`, `missing-ledger-refuses-boot`, projection deletion, replay, and cost. Each discrepancy is an implementation fault, a specification fault, which returns to the owning chapter and reopens the freeze, or a rejected expectation.

The first real instance follows the approved freeze and rehearsal, with a manifest of every frozen version, policy version, fixture-corpus digest, and deployment configuration needed to reproduce the decision. It is not created when an input differs from the frozen candidate or the rehearsal is stale. After creation, faults are corrected additively or through a recorded upcast. Normative material moves from this tree into `docs/` when genesis makes it as-built, or when a deferred capability is enabled.

## Evaluation discipline

Composite benchmark scores are never acceptance targets. Structural behaviour is checked by deterministic oracles against the system's own event log. Model judges are limited to linguistic judgements ([confidence](confidence.md#claims-deliberately-not-made)).

## Deferred capabilities

Each capability below is outside the genesis design. Its raw inputs are recorded from genesis where the [keep-at-genesis test](overview.md#the-keep-at-genesis-test) requires them, and enabling it is additive. Enabling one never enables another; a named prerequisite is enabled first by its own decision. The reopen condition is the evidence needed before detailed design and enablement. Autonomous identity is the first capability planned after the successor lands.

| ID | Capability | Reopen condition |
|---|---|---|
| `autonomous-identity` | An off-turn authority that scores identity hypotheses and may accept at `recall`; requires `recall-restriction-projection` ([identity](identity.md#evidence-and-authority)). | Evidence on score calibration, overlap, adversarial resistance including recitation and the patient attacker, one-handle behaviour, and composite strengthening and withdrawal thresholds. |
| `recall-restriction-projection` | Marking contexts that render recall-cleared content, propagated through Activity edges ([identity](identity.md#recall-and-disclosure-clearance)). | Before an off-turn Activity renders recall-cleared content; scenarios show the mark keeps it out of every response-affecting context. |
| `event-coreference` | Any Event merge or co-reference hypothesis ([events and roles](events-and-roles.md#co-reference)). | Measured duplicate-Event cost, with evidence that merges reverse without history loss and similar role sets cause no false acceptance. |
| `broad-event-role-vocabulary` | Roles beyond the universal parents and registered subroles ([events and roles](events-and-roles.md#roles-and-attributes)). | Scenarios establish teachability, stable filler constraints, and parent traversal. |
| `collective-plural-readings` | Collective against distributive plurals ([object model](statements.md#proposition)). | Query demand that source-only retention cannot answer. |
| `option-b-widening-defaults` | Per-person or per-conversation widening defaults for the testimony floor ([privacy and provenance](privacy-and-provenance.md#deferred)). | `principle-assignment` measures option a's recall cost above its bound. |
| `support-fusion` | Fusion operators and numeric support ([belief](belief.md)). | The first real claim with independent Attestations from two human tellers. |
| `non-mechanical-contest` | Contest over similar, ambiguous, conditional, or context-dependent Assertions ([belief](belief.md#contest-and-contradiction)). | Multi-teller evidence, and a detector that separates contradiction from legitimate coexistence at a bounded false-contest rate. |
| `autonomous-contradiction-arbitration` | Autonomous arbitration of contests ([belief](belief.md#contest-and-contradiction)). | A proposal-only policy passes contradiction-versus-contest scenarios without authority escalation. |
| `reliability-observations` | Outcome observations projected into domain-specific reliability ([belief](belief.md#genesis-evidence)). | A policy defines evaluator, method, and domain, and outcome data exists. |
| `qualitative-temporal-inference` | `overlaps`, `meets`, `equals`, Assertion-relative bounds, and chain composition ([time](time.md#event-bounded-validity)). | Queries Event-bounded validity cannot answer, and evidence on which tractable subset suffices. |
| `habitual-deontic-inference` | Inference over habitual and deontic modality ([time](time.md#modality)). | Such content matters to queries, and a policy avoids normative invention. |
| `business-calendar-adjustment` | Recurrence under a versioned business calendar ([time](time.md#recurrence-intent)). | A real commitment needs it, and a versioned calendar source exists. |
| `volatility-automation` | Review-horizon jobs and the `validity_boundary_due` mark ([time](time.md#staleness-and-volatility)). | A bounded job budget, and evidence of neither false urgency nor source mutation. |
| `autonomous-recurrence-interpretation` | Background interpretation of recurrence intent ([time](time.md#genesis-boundary)). | Bounded cost, and no Task created from description. |
| `working-note-review` | Scheduled promotion review and the `working_review_due` mark; explicit promotion is at genesis ([memory typology](memory-typology.md#working)). | Evidence, from `taint-breadth` and operation, that useful notes go unpromoted, and a bounded job that never promotes over-tainted notes. |
| `generated-episodes` | `episodic_reconstruction` narratives and the `episode_due` mark. | The evaluation in [two traces](two-traces.md#evidence-and-deferral) passes, with the four wall scenarios; it also measures what the strict wall blocks through delivered Occasions and decides whether a narrower rule is safe. |
| `procedure-extraction` | Automatic procedure extraction ([memory typology](memory-typology.md#procedural)). | Scenarios show bounded cost, review, and authority. |
| `bulk-ingestion` | Source-first ingestion of long documents and media, and the `pending_ingest` mark ([memory typology](memory-typology.md#conversational-artefacts-are-not-bulk-ingestion)). | Bounded calls and log growth, span precision, source-only degradation, audience non-interference, and replay after retry. |
| `media-selectors` | `spatial_region`, `frame_range`, `time_range`, and `byte_range` ([artefacts](artefacts-and-perceptions.md#deferred-capabilities)). | A grounding need for regions, frames, or media time, with a decoder basis stable under versioning. |
| `ocr` | OCR, including scanned documents ([artefacts](artefacts-and-perceptions.md#deferred-capabilities)). | Evidence on invented-text rate and correction, with audience and erasure intact. |
| `generated-captions` | Generated Perceptions outside the current turn ([artefacts](artefacts-and-perceptions.md#deferred-capabilities)). | They are neither invented nor laundered, at bounded cost. |
| `visual-retrieval` | Visual embeddings and the `visual_embedding` lane ([query surface](query-surface.md#search-lanes)). | A recall study against prior Perceptions and `inspect`, with reference-authorised indexing and erasure. |
| `scene-graph-writer` | Scene-graph extraction from media ([artefacts](artefacts-and-perceptions.md#deferred-capabilities)). | A safety plan, and evidence that it neither invents structure nor launders sources. |
| `transmission-consent`, `transmission-purpose-limitation`, `transmission-reciprocity` | Three further principles ([privacy and provenance](privacy-and-provenance.md#deferred)). | Respectively: a scoped, expiring, revocable consent record and a flow that needs it; an execution-purpose model rather than a caller string; a stable definition of comparable disclosure. |
| `connector-assurance-kinds` | Connector-specific kinds that widen disclosure ([privacy and provenance](privacy-and-provenance.md#witness-evidence)). | The connector demonstrates the kind's semantics after `witness-assurance-audit`. |
| `inter-agent-exchange` | Claims quoted from another agent, with provenance and revocation ([privacy and provenance](privacy-and-provenance.md#deferred)). | A second agent exists, and a freshness and revocation record is designed. |
| `hardened-erasure` | Per-payload keys, proof of destruction, and multi-party authorisation ([privacy and provenance](privacy-and-provenance.md#retraction-and-erasure)). | The deployment becomes multi-tenant or publicly exposed. |
| `exploration` | Sampling structure for connections no mark names ([off-turn work](off-turn.md#deferred-exploration)). | A fixed-budget trial shows useful yield without privacy interference. |
| `proactive-initiation` | Messages initiated without a Task ([off-turn work](off-turn.md#deferred-proactive-initiation)). | A salience and interruption policy is proposed and evaluated. |
