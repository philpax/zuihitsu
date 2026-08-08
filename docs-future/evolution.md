# Evolution

How the codebase gets from what runs today to what the rest of this tree describes.

## Scope

This is a code path, not a data path. The design targets a new instance at genesis, and the existing instance is not migrated. No upcasting story is owed for existing entries, no dual-read compatibility layer is built, and no stage keeps the old and new fact models coexisting in one log. See [how the work lands](#how-the-work-lands) for what follows from that.

That freedom is load-bearing in several places. Structural equality can be the deduplication primitive because no legacy prose entries need matching against it. The referential frame can be required on every Statement because no entry predates it. Relation definitions can demand domain and range because nothing was registered without them.

The existing instance keeps running on the current model for as long as it is useful. Its log remains the best source of evidence about what a real corpus contains, and [the corpus study](research/2026-08-03/modelling-study.md) is one example of reading it without touching it.

## Ordering principles

Three rules decide the sequence.

Cheap decisions come before expensive commitments. Several questions decide the cost of large parts of the design and can be answered before the substrate exists. They go first.

Containment comes before capability. The episodic wall ships before anything that generates narrative. A guard added alongside the thing it guards was optional during the window that mattered.

External prerequisites come first. One dependency originates outside this design and gates the privacy work regardless of how that work is built.

## Stage 0: the decisions that come first

Four measurements, none of which needs new substrate. Three run against the system that exists today, and the fourth against a synthetic log.

### 0a. The encoding-against-retrieval ablation

The single unresolved question with the largest cost attached. The dual-trace evidence cannot say whether the benefit comes from writing an elaborated narrative or from reading richer context at retrieval, and the difference is a record-time model call and permanent log volume per occasion against nearly nothing.

The existing instance is well placed to answer it, because every content entry already records the turn that produced it. Three arms:

1. Control: current behaviour.
2. Retrieval-side: surface the source turn window alongside a retrieved claim. No new writes, no model calls, no log growth.
3. Encoding-side: generate a narrative at session close and index it.

Scenarios mirror the four capabilities the evidence distinguishes, with single-occasion lookup as a declared null arm. A treatment that improves every category has not been understood.

Unblocks: the entire episodic layer's cost model, and stage 4's scope.
Gating: arm 2 against arm 1 on the three complex capabilities, and no movement on the null.
Risk: the arms differ in context volume as well as in structure, so the comparison needs care to avoid measuring prompt size.

### 0b. Present-set definition

[Issue #123](https://github.com/philpax/zuihitsu/issues/123) reports that the present set conflates being in a channel with being in a conversation, so a confidence can reach a silent member. This design makes audience conditions predicates over who is present, which means a wrong present set produces wrong answers more expressively than the current enum does.

The same definition decides the disclosure set and the exposure set on a gloss, and the two pull in opposite directions. A present set that is too wide leaks a confidence to a silent member. A disclosure set that is too wide licenses repeating something to someone who never saw it, which is the same leak arriving through the other door. All three read the same underlying judgement about who was in the conversation, so a stage that gets it wrong is wrong twice. The disclosure set is the more dangerous of the two, because it widens an audience rather than restricting one.

Unblocks: stage 7, both witness sets that [`privacy-and-provenance.md`](privacy-and-provenance.md) and [`belief.md`](belief.md) rely on, and a class of live leak reduced in the meantime.
Gating: a scenario where a silent channel member does not receive a confidence, and its mirror, where a confidence is not repeated back to a member who was present but silent.
Risk: the correct definition may be platform-specific, in which case it belongs to the connector contract rather than the core.

### 0c. Extraction economics against the existing corpus

The design's central economic claim is that structuring pays for itself. Stages 0a and 0b test representational and privacy questions. This one tests the bill, and it needs no new code beyond a harness, because the data already exists.

Replay the existing instance's 198 content entries through a schema-constrained extractor on the target model and measure four things:

- Yield. What fraction produces a non-junk triple. The corpus is not uniform: roughly a third sits on topic memories holding interpretive readings, conditionals, and generics, where there may be no structure to find. If yield on that third is poor, the junk-fill failure mode is systematic rather than occasional, and a gloss-only write verb is the answer rather than a better prompt.
- Convergence. What fraction of the known re-mentions produce byte-identical claims. Structural equality as a deduplication primitive assumes an extractor converges on the same triple from different prose, and that assumption is currently unmeasured. The exact-textual-duplicate set does not test it, since fourteen of those sixteen are connector boilerplate. The consolidation and arbitration events in the live log name entries the running system judged to be one claim, which is the labelled re-mention set this measurement needs.
- Latency. The p50, p90, and p99 per extraction. Measured against the same instance's recorded model calls as a baseline: p50 6.8s, p90 30.8s, p99 73.7s, max 94.9s over 417 calls.
- Volume. Bytes added to the log per structured write. Recorded model calls are already 95.9% of this log's payload (31.4 MB of 32.7 MB, mean 75 KB per call) against 0.27% for the content entries themselves. Structuring adds a call per write block, of which this log has 132.

The figures above are recorded in [`research/2026-08-06/log-measurements.md`](research/2026-08-06/log-measurements.md).

Unblocks: the cost model for the whole seam, and the choice of default in [`write-surface.md`](write-surface.md).
Gating: none. This is measurement, not a gate, and its output is numbers that stages 2 and 4 are then judged against.
Risk: the extractor used in the harness is not the one that ships, so the numbers are indicative rather than binding. They are still better than the current position, which is no numbers at all.

### 0d. The console fold budget

[`coverage.md`](coverage.md) calls the console's whole-fold a prerequisite rather than a deferred concern, and the classification is correct. Recorded model calls already dominate the log's payload, and putting a structuring call on every write block roughly doubles the dominant term. A replica that folds the whole log in browser memory is the surface that pays for that.

Measure against a synthetic log at realistic sizes: bytes added per turn under the new write path, fold time and peak memory in the browser, and where the curve stops being usable.

Unblocks: knowing whether the replica needs windowing or snapshots before stage 2 rather than after stage 4.
Gating: none. This is measurement, and its output is the threshold the later stages are judged against.
Risk: a synthetic log is not a real one, and the shape of real traffic decides the answer as much as its volume.

## Stage 1: the modelling spike

Model the [Statement](statements.md), the [Event](events-and-roles.md), the [frame](statements.md), and the first hard critics: type, domain and range, frame consistency, and duplicate resolution. Replay them over recorded logs with no live model, in the style of the existing rejudge mode.

Unblocks: confidence that the shape holds real utterances before the seam is committed to.
Gating: the two same-happening entries of the recorded four-entry case resolve to one Event with correct roles, while the distinct causal claim stays distinct; the duplicate critic flags the re-filing.

For the frame, the criterion is deliberately stricter than "the layers separate", because both readings of what a `source` claim's subject is would satisfy that. The observed cat case must resolve to the right subject: a claim about a persona's principal must land on the principal, not on the persona under any frame value. The mechanism under test is [the `principal` redirect](statements.md), resolved against a seeded `presents` edge by a critic at write time, so this stage has a concrete design to falsify rather than a gap to report. If the redirect fails here, the alternative is a referent pointer carried on the Statement, at a cost the redirect avoids.

Risk: the frame's values prove wrong on a corpus other than the one that motivated them. This is why the check happens here rather than after the substrate is built.

## Stage 2: the Statement substrate and the seam

Build the Statement as the atomic write unit, the typed seam, the hard and soft critic banks, and forced-choice elicitation for the load-bearing writes.

Two pieces of configuration land here too, because a new instance needs both at genesis and neither is a memory: [the self slot](memory-typology.md), which holds the charter the prompt reads every turn, and [scoped directives](memory-typology.md), versioned, which is what a connector writes when it opens a context.

One further obligation is easy to miss and belongs here rather than later: the agent's own outbound turns become first-class glosses carrying both witness sets. Today they are recorded as turns and nothing more. Without them, a claim the agent relayed and was later told back is indistinguishable from independent corroboration, so [the dependence rule](belief.md) that stage 6 gates on cannot be evaluated at all. The datum has to exist from the moment Statements do.

Unblocks: everything downstream. Every later stage writes Statements.
Gating: extraction fidelity against gold-structure scenarios, scored on precision and recall; a faithfulness oracle asserting every structural write is entailed by some utterance in the transcript; and a paraphrase-spread probe showing near-zero spread on field-content correctness, not on capture presence, which a required field pins by construction.
Risk: the constraint tax. Schema forcing can suppress tool use or degrade reasoning, and this must be measured on the target model per behaviour rather than assumed. If it bites, the response is to constrain fewer behaviours, not to abandon the seam.

## Stage 3: time

The three-axis split, typed values including quantities, and qualitative anchoring.

Unblocks: correct scheduling semantics, and the temporal graph.
Gating: a dated description never fires, and a genuine task with a trigger does. A claim with no temporal anchor in its utterance stays unstamped rather than taking the day it was heard.
Risk: the chosen interval subset is too weak to infer the orderings that matter in practice.

## Stage 4: the memory typology and episodes

The four kinds with their lifecycles. Scope depends on stage 0a: if the benefit is retrieval-side, this stage is linkage and co-retrieval only, and the narrative generator is not built.

The episodic wall ships first within this stage, as critics, before any generation exists to need containing.

Unblocks: the episodic, procedural, and working kinds; long-document ingestion.
Gating: an episodic trace cannot be made a premise, distilled, or attested, and every attempt is a teachable error. A question answerable only from an episode is answered with hedged provenance. Ingestion precision does not fall against a short-document baseline.
Risk: episode volume. If the gate is too loose the store fills with reconstructions, and the cost per fact stops falling.

## Stage 5: identity

Merges as revocable assumptions, assumption stamps, fold-filter severance, relational evidence, and the substrate wall.

Unblocks: zero-administration identity; the behaviour leak.
Gating: a merge-then-sever scenario folds cleanly to the world as if never merged. Post-merge sibling history relays correctly, against the measured 0.30 baseline. A recitation attack does not raise merge credence.
Risk: crumble and accretion thresholds are unresolved and need real data, because genuine same-person profiles also diverge.

## Stage 6: belief

Credence from evidence counting, trust discounting, dependence detection, non-prioritised revision.

Unblocks: principled contradiction handling; the recitation defence at the belief layer.
Gating: two dependent attestations produce no credence gain, including the two dependence paths that run through the agent, a claim it re-recorded and a claim it relayed and was told back. A low-credibility teller does not overturn a well-corroborated claim. A claim hedged and later asserted flatly by the same teller is one Statement with two tellings whose credence has not moved, while the same claim corroborated by a second independent teller does move.
Risk: the credence shape is a live disagreement. Ship the representation and discounting; leave fusion operators unused until validated.

## Stage 7: transmission principles and forgetting

Postures promoted to registered conditions, retraction against erasure, propagation through the derivation graph, the authority lattice, and inter-agent claims as quotations. Gated on stage 0b.

Unblocks: cross-boundary confidences; a lawful erasure story.
Gating: zero residue holds as non-interference under an erasure that propagates through derived conclusions. A description, which cannot attribute, is composed from public content alone; an episode, which carries a teller list, takes the intersection and never draws on a confidence. A new read path cannot leak by omission, because visibility is computed once rather than per path.
Risk: the combinatorial blowup if audiences ever become richer than predicates over the present set.

## Stage 8: drift and the exception queue

Longitudinal scenarios with canary re-probes and re-derivation audits; the operator exception queue.

Unblocks: the zero-administration endgame, meaning exception-triggered attention backed by detection.
Gating: a canary flip, an audit oscillation, or a post-embedder-change structural shift raises exactly one alarm, with no false positives over a clean run.
Risk: drift is a longitudinal property a single-turn scenario structurally cannot see, so the harness itself is novel work, and canaries are weakest against the drift shape that actually occurs.

## Stage 9: off-turn work

[`off-turn.md`](off-turn.md) is a property every stage owes rather than a stage of its own, and it is listed last because that is where it can be asserted over the assembled system rather than intended.

Two halves land at different times. Retiring the parts of consolidation that recover structure rides stage 2: the moment writes carry structure, a pass that deduplicates by similarity is doing work the write path already did, and leaving it running would mean two mechanisms deciding sameness by different rules. The queue machinery accretes stage by stage instead, because each stage introduces its own marks: voided derivations at stage 5, contested pairs at stage 6, erasure propagation at stage 7.

What is left for this stage is the assertion over the whole assembled set.

Unblocks: a maintenance budget that does not grow with the store.
Gating: no pass writes anything the critics would reject on the hot path; no pass widens an audience, promotes an episode to a premise, or reaches the self slot; a tick with nothing marked performs no model call; and a due trigger fires on a tick whose maintenance queue is saturated.
Risk: the queues are only as good as the marks, and a condition nobody thought to mark is a silent gap where a sweep would at least have been slow and correct. The drift audits of stage 8 are the backstop, which is an argument for not reordering this ahead of them.

## How the work lands

Three decisions govern the transition. They are stated here because a reviewer needs them before agreeing to the sequence, and an implementer needs them before starting.

### No compatibility layer

The new model replaces the old outright. There is no flag selecting between fact models, no parallel implementation, no dual-read path, and no migration of existing entries. A stage that would be easier with a compatibility shim is implemented without one, and the shim is not built.

The running instance keeps its current build until an instance born under the new model replaces it. Its log stays readable as evidence for as long as that is useful, which is a concern separate from the codebase, and [`research/`](research/) is where such readings are recorded.

### Each stage acquires its plan when work starts on it

The chapters are the design input to an implementation plan. They are not the plan. A stage is planned at the point it is picked up, through the repository's plan-and-execute workflow, which produces the module boundaries, event payloads, wire types, genesis path, and eval scenarios that stage needs. Stage 0's four measurements are scoped the same way, and each eval run they require is the operator's to authorise.

Nothing in this file is sufficient to implement from.

### The tree drains into `docs/`

`docs-future/` is temporary by construction. As each change lands, the commit that implements it also removes it from the chapter that proposed it and writes it into [`../docs/`](../docs/) as as-built documentation. A chapter empties over the stages that build it rather than being copied across at the end, so the two trees never describe the same mechanism at once.

Three things remain when the chapters are empty:

- The meta-documents describe a proposal rather than a system. [`coverage.md`](coverage.md), [`confidence.md`](confidence.md), and this file are deleted at that point, and git history holds them.
- [`research/`](research/) returns to `docs/` with its dates intact, because evidence about why the system has its shape outlives the proposal that used it.
- Nothing else. A `docs-future/` still standing after the last stage means a change was documented and not landed.

## Reordering

Stages 0 and 1 are the load-bearing bet and should not be reordered. Stage 2 gates everything after it.

Stages 3 through 9 are independently valuable and can be resequenced against evidence, with three constraints: the episodic wall precedes narrative generation, stage 0b precedes stage 7, and stage 9 follows stage 8 so the audits exist before the sweeps they replace are retired. If the constraint tax turns out to be severe at stage 2, the right response is to reduce the number of schema-constrained behaviours and continue, not to reorder around the problem.

## Success criteria

Success is not "all eleven failure classes closed". [`coverage.md`](coverage.md) grades six as closed structurally and five as answered without validation, and the stages above are how the five acquire evidence. A stage that ships without its gating evidence has moved the design forward and moved the confidence register backward, which is a trade worth making occasionally and worth noticing every time.
