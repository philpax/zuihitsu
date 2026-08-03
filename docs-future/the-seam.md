# The seam

The model proposes. The store disposes. The boundary between them is typed, and a write that crosses it can be rejected.

This is the part of the design that is genuinely unfinished elsewhere. Essentially no system that fills a knowledge graph from a language model verifies the model's writes against their source: the model is both the writer and its own only judge, and the field's usual remedy for a bad write is a better prompt.

## The writer proposes structure

Writes cross as typed structured proposals: an [Event](events-and-roles.md) with typed roles, a [relation](relations.md) instance against a declared schema, a typed interval, a [frame](statements.md), a transmission principle. Never an untyped sentence.

If the seam is typed, nothing downstream has to recover structure from prose, because the structure was captured where it was known. The current seam is a prose sentence, which is the maximally untyped interface and the reason every downstream mechanism re-parses.

The doctrine is adopted as doctrine rather than as machinery: **the neural half is never the final authority on a structural question**. There is no differentiable logic layer here and no theorem prover. There is a typed interface and a bank of checks.

## Hard critics

Sound, symbolic, and gating. A write that violates one is rejected with a teachable error, which is the existing pedagogy now backed by a check rather than a hope.

- **Type, domain, and range** on every relation argument. A reversed or mistyped edge is a rejected write. The current link graph contains edges asserting that a room operates a person and that an event participates in a person; all are well-formed as bare edges and all fail a declared range.
- **Frame consistency** between a relation and the layer it is asserted in, so a claim about a historical source cannot attach to the software presenting a persona.
- **Mutual exclusion**: a thing cannot be two incompatible kinds at once. This is the constraint that historically did the most to suppress drift in a long-running autonomous accumulator.
- **Temporal well-formedness**: intervals that begin before they end, occurrences that do not silently become triggers.
- **Audience invariants**: no endorsement wider than the posture it was founded under, checked at write time rather than hoped for as an emergent property.
- **Merge authority**: who may assert an identity claim.
- **Duplicate resolution**: a re-mention resolves to the existing Statement rather than appending a near-copy, and an ambiguous match is a teachable error rather than a silent merge.
- **The episodic wall**: an episodic trace can never be a premise, never be distilled, never accrue attestation, and never carry a teller other than the agent.

That last one deserves its own note. Generating narrative deliberately asks a model to commit to concrete detail it was not given, and the current system has already produced the unelaborated version of that failure: content invented for a document that was never read, attributed to the person who mentioned it. The prior art's only guard is a sentence in the template asking the model to remember that its reconstruction is not evidence. A prompt-borne guard on a safety property is precisely the failure mode this chapter exists to remove, so the wall is a critic.

## Soft critics

LLM-based, sampled and voted, and never gating.

They handle judgements that are genuinely linguistic: whether a summary reads well, whether two descriptions say the same thing, whether a narrative is a fair account of an occasion. Being occasionally wrong is tolerable, so they are used where wrongness is recoverable.

They are never single-shot, because a single sample from a model judging its own kind of output is the weakest possible check, and they never decide a hard property.

## Load-bearing behaviour moves off the prompt

The decision rule is **failure mode**. Any behaviour whose failure is silent and consequential moves into structure the harness enforces. Capture, audience setting, speaker resolution, temporal placement, the description-against-task distinction, and the decision to schedule: each becomes a required field a turn cannot complete without filling.

The motivating datum is that a load-bearing capture behaviour moved from roughly 6% to 75% of eligible cases on a single sentence changed in a template. A behaviour that swings that far on wording is not a behaviour, it is an accident, and it will re-roll on every model change.

Three honest qualifications.

A required field eliminates the **omission** variance and relocates variance into the field's **content**. Capture stops depending on wording; whether the captured content is right does not.

It introduces **junk fill**. A model that must put something in a slot will put something in a slot. The answer is an explicit "nothing to record" option, so declining is a recorded decision rather than an empty string, plus the hard critics rejecting malformed content.

And structured constraints are **not free**. Forcing schemas can suppress tool use or degrade reasoning in some models. The move is to constrain the load-bearing structural writes and leave the generative work free, decided per behaviour by measurement rather than assumed.

What stays on the prompt is what tolerates drift: how something is phrased, conversational manner, which of several valid framings to choose. The prompt teaches principles and taste. The harness enforces steps.

## Agreement before promotion

A claim is not settled because it was written.

Two independent signals must agree before a claim is promoted from candidate to settled: two independent tellers, or structural and semantic evidence concurring. Coupling independent signals is what keeps per-channel error from compounding silently, and the candidate boundary is an explicit, inspectable credence and threshold rather than a feeling.

## Neural calls inside a block

The agent can invoke model judgement inside a symbolic transaction. It is a **record-at-call-time non-deterministic activity**: the first execution writes the prompt and response into the log, and replay consumes the recorded response without calling anything. This is the same treatment ordinary model calls already get, and it is what keeps determinism intact.

The call is exposed primarily as schema-constrained functions with named purposes, extraction, classification, resolution, and yes-or-no judgements, which *are* the forced-choice elicitation described above. A freeform completion exists as a deliberative escape hatch that cannot commit anything, because a freeform call with transaction-commit power reintroduces the unchecked writer this whole chapter is built to remove.

Every write derived from such a call still passes the hard critics.

## Drift

An autonomous accumulator drifts unless something outside the loop watches it. Four mechanisms, none of which is a better prompt.

**The ontology is the brake.** Typed, mutually exclusive, constraint-checked structure is what a bad write can violate. This is the primary defence and the reason structure matters more than any detector; everything else is detection.

**Canaries.** Known-true, known-false, and known-private claims seeded at genesis and re-probed over a long simulated lifetime. A flip is an alarm.

**Re-derivation audits.** Run the maintenance passes repeatedly over a growing log and assert the derived structure is stable: that deduplication does not oscillate, that merges do not thrash, that contradictions do not accumulate. Re-run after any embedding model change and assert nothing silently shifted.

**Calibrated thresholds.** Similarity thresholds are fitted to the current embedder's own distribution and recomputed when it changes, rather than being constants in one model's geometry. Retrieval and decay rank on access recency and frequency as well as similarity, both of which are embedder-independent and replay-deterministic. Similarity is one term, not the ranking.

Canaries and audits are necessary and not sufficient. They see seeded regions and they test stability rather than correctness, which makes them weakest against the drift shape a long-running accumulator actually suffers: a gradual precision decline in newly written claims. That is what agreement-before-promotion is for.

Autonomy here means **exception-triggered attention, not no attention**. A small queue reaches a person: an identity-boundary disclosure the substrate cannot clear, a forget request whose authority is unclear, a drift alarm, and a persistent critic rejection that suggests a schema gap rather than a mistake. The expectation is that this queue is small and that its per-fact cost falls as the store grows, because any mechanism whose cost per fact does not fall eventually kills the instance.
