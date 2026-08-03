# Evolution

How the codebase gets from what runs today to what the rest of this tree describes.

## Scope

**This is a code path, not a data path.** The design targets a new instance at genesis, and the existing instance is not migrated. No upcasting story is owed for existing entries, no dual-read compatibility layer is needed, and no stage has to keep the old and new fact models coexisting in one log.

That freedom is load-bearing in several places. Structural equality can be the deduplication primitive because no legacy prose entries need matching against it. The referential frame can be required on every Statement because no entry predates it. Relation definitions can demand domain and range because nothing was registered without them.

The existing instance keeps running on the current model for as long as it is useful. Its log remains the best source of evidence about what a real corpus contains, and [the corpus study](research/2026-08-03/modelling-study.md) is one example of reading it without touching it.

## Ordering principles

Three rules decide the sequence.

**Cheap decisions before expensive commitments.** Two questions decide the cost of large parts of the design, and both can be answered before the substrate exists. They go first.

**Containment before capability.** The episodic wall ships before anything that generates narrative. A guard added alongside the thing it guards is a guard that was optional during the window that mattered.

**Prerequisites that are not ours.** One dependency comes from outside this design and gates the privacy work regardless of how it is built.

## Stage 0: the two decisions

Neither stage needs any new substrate, and both can run against the system that exists today.

### 0a. The encoding-against-retrieval ablation

The single unresolved question with the largest cost attached. The dual-trace evidence cannot say whether the benefit comes from *writing* an elaborated narrative or from *reading* richer context at retrieval, and the difference is a record-time model call and permanent log volume per occasion against nearly nothing.

We are unusually well placed to answer it, because every content entry already records the turn that produced it. Three arms:

1. Control: current behaviour.
2. Retrieval-side: surface the source turn window alongside a retrieved claim. No new writes, no model calls, no log growth.
3. Encoding-side: generate a narrative at session close and index it.

Scenarios mirror the four capabilities the evidence distinguishes, with single-occasion lookup as a **declared null arm**. A treatment that improves every category has not been understood.

*Unblocks:* the entire episodic layer's cost model, and stage 4's scope.
*Gating:* arm 2 against arm 1 on the three complex capabilities, and no movement on the null.
*Risk:* the arms differ in context volume as well as in structure, so the comparison needs care to avoid measuring prompt size.

### 0b. Present-set definition

[Issue #123](https://github.com/philpax/zuihitsu/issues/123) reports that the present set conflates being in a channel with being in a conversation, so a confidence can reach a silent member. This design makes audience conditions predicates over who is present, which means a wrong present set produces wrong answers more expressively than the current enum does.

*Unblocks:* stage 7, and reduces a class of live leak in the meantime.
*Gating:* a scenario where a silent channel member does not receive a confidence.
*Risk:* the correct definition may be platform-specific, in which case it belongs to the connector contract rather than the core.

## Stage 1: the modelling spike

Model the [Statement](statements.md), the [Event](events-and-roles.md), the [frame](statements.md), and the first hard critics: type, domain and range, frame consistency, and duplicate resolution. Replay them over recorded logs with no live model, in the style of the existing rejudge mode.

*Unblocks:* confidence that the shape holds real utterances before the seam is committed to.
*Gating:* the recorded four-copy happening resolves to one Event with correct roles; the duplicate critic flags the re-filings; the frame separates the three referential layers on the persona-agent memory that the corpus study found mixed.
*Risk:* the frame's three values prove wrong on a corpus other than the one that motivated them. This is why it is checked here rather than after the substrate is built.

## Stage 2: the Statement substrate and the seam

Build the Statement as the atomic write unit, the typed seam, the hard and soft critic banks, and forced-choice elicitation for the load-bearing writes.

*Unblocks:* everything downstream. Every later stage writes Statements.
*Gating:* extraction fidelity against gold-structure scenarios, scored on precision and recall; a faithfulness oracle asserting every structural write is entailed by some utterance in the transcript; and a paraphrase-spread probe showing near-zero spread on **field-content correctness**, not on capture presence, which a required field pins by construction.
*Risk:* the constraint tax. Schema forcing can suppress tool use or degrade reasoning, and this must be measured on the target model per behaviour rather than assumed. If it bites, the response is to constrain fewer behaviours, not to abandon the seam.

## Stage 3: time

The three-axis split, typed values including quantities, and qualitative anchoring.

*Unblocks:* correct scheduling semantics, and the temporal graph.
*Gating:* a dated description never fires, and a genuine task with a trigger does. A claim with no temporal anchor in its utterance stays unstamped rather than taking the day it was heard.
*Risk:* the chosen interval subset is too weak to infer the orderings that matter in practice.

## Stage 4: the memory typology and episodes

The four kinds with their lifecycles. Scope depends on stage 0a: if the benefit is retrieval-side, this stage is linkage and co-retrieval only, and the narrative generator is not built.

**The episodic wall ships first within this stage**, as critics, before any generation exists to need containing.

*Unblocks:* the episodic, procedural, and working kinds; long-document ingestion.
*Gating:* an episodic trace cannot be made a premise, distilled, or attested, and every attempt is a teachable error. A question answerable only from an episode is answered with hedged provenance. Ingestion precision does not fall against a short-document baseline.
*Risk:* episode volume. If the gate is too loose the store fills with reconstructions, and the cost per fact stops falling.

## Stage 5: identity

Merges as revocable assumptions, assumption stamps, fold-filter severance, relational evidence, and the substrate wall.

*Unblocks:* zero-administration identity; the behaviour leak.
*Gating:* a merge-then-sever scenario folds cleanly to the world as if never merged. Post-merge sibling history relays correctly, against the measured 0.30 baseline. A recitation attack does not raise merge credence.
*Risk:* crumble and accretion thresholds are unresolved and need real data, because genuine same-person profiles also diverge.

## Stage 6: belief

Credence from evidence counting, trust discounting, dependence detection, non-prioritised revision.

*Unblocks:* principled contradiction handling; the recitation defence at the belief layer.
*Gating:* two dependent attestations produce no credence gain. A low-credibility teller does not overturn a well-corroborated claim. A hedged claim later corroborated is one Statement whose credence moved.
*Risk:* the credence shape is a live disagreement. Ship the representation and discounting; leave fusion operators unused until validated.

## Stage 7: transmission principles and forgetting

Postures promoted to registered conditions, retraction against erasure, propagation through the derivation graph, the authority lattice, and inter-agent claims as quotations. Gated on stage 0b.

*Unblocks:* cross-boundary confidences; a lawful erasure story.
*Gating:* zero residue holds as non-interference under an erasure that propagates through derived conclusions. Public-only distillation never leaks an attributed or private claim. A new read path cannot leak by omission, because visibility is computed once rather than per path.
*Risk:* the combinatorial blowup if audiences ever become richer than predicates over the present set.

## Stage 8: drift and the exception queue

Longitudinal scenarios with canary re-probes and re-derivation audits; the operator exception queue.

*Unblocks:* the zero-administration endgame, meaning exception-triggered attention backed by detection.
*Gating:* a canary flip, an audit oscillation, or a post-embedder-change structural shift raises exactly one alarm, with no false positives over a clean run.
*Risk:* drift is a longitudinal property a single-turn scenario structurally cannot see, so the harness itself is novel work, and canaries are weakest against the drift shape that actually occurs.

## Reordering

Stages 0 and 1 are the load-bearing bet and should not be reordered. Stage 2 gates everything after it.

Stages 3 through 8 are independently valuable and can be resequenced against evidence, with two constraints: the episodic wall precedes narrative generation, and stage 0b precedes stage 7. If the constraint tax turns out to be severe at stage 2, the right response is to reduce the number of schema-constrained behaviours and continue, not to reorder around the problem.

## What success looks like

Not "all eleven failure classes closed". [`coverage.md`](coverage.md) grades six as closed structurally and five as answered without validation, and the stages above are how the five acquire evidence. A stage that ships without its gating evidence has moved the design forward and moved the confidence register backward, which is a trade worth making occasionally and worth noticing every time.
