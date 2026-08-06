# Coverage

What this design addresses, graded honestly, and what it makes worse.

The grading is uneven on purpose. Six of the eleven surveyed failures are closed structurally, meaning the failure becomes unrepresentable rather than discouraged. Five are answered in design and rest on evidence that has not been gathered. Presenting all eleven as "addressed" would be the more comfortable claim and the less useful one.

The failures are those recorded in [`../docs/ontology-failures/2026-07-23.md`](../docs/ontology-failures/2026-07-23.md).

## The eleven classes

### Closed structurally

**Facts are sentences.** The [Statement](statements.md) is a typed claim, and prose is a second trace rather than the only representation. Deduplication becomes structural equality instead of a cosine guess, arbitration operates on claims instead of re-parsing sentences, and a structural question reaches a structural answer through the [query surface](query-surface.md). Consolidation, whose output was a new sentence whose relationship to its sources was recoverable only through metadata, stops being necessary: two facts were always two Statements.

**One event, one subject, many copies.** One [Event](events-and-roles.md) with role-edges, and a re-mention resolving to it as a structural no-op. On the flagship case, four entries filed for one happening, two are the same happening rotated onto different participants and collapse to one node; the third is a genuinely distinct causal claim; the fourth is a dispositional generalisation the model cannot hold at all. The per-subject rephrasing is what this closes, and it is half of that case rather than all of it.

**Relations are bare edges.** A [relation](relations.md) instance is a Statement carrying a validity interval, provenance, credence, and frame, with declared domain and range on the definition. Time-bounded facts stop degrading into prose.

**Schedule and description conflate.** [Three axes](time.md), and only a Trigger fires. A Trigger hangs off a Task the agent authored for itself, so a fact describing someone else's recurring job has no path to waking anything. One qualification, carried from the research: no surveyed peer has this failure, so the *problem* may be specific to us even though the *solution shape* is mature.

**Relation schemas are immutable.** Deprecate-and-alias with read-time transitive resolution. The same relation coined four ways collapses to four aliases of one canonical form, without rewriting history.

**Identity complexity leaks into behaviour.** The [substrate wall](identity.md). The agent receives one resolved handle and never holds two, so there is nothing to test for equality and nothing to second-guess. The measured failure, a 0.30 relay rate after a confirmed merge, was the agent's model of the merged identity faltering while the machinery held. Removing the model removes the faltering.

### Answered in design, not yet validated

**Identity is binary and entangled with storage.** [Revocable graded merges](identity.md) with assumption-stamped derivations and fold-filter severance. The design is coherent and the literature is unanimous that hard equivalence is wrong. What is missing is data: crumble and accretion thresholds are unresolved, the recitation attack is made expensive rather than closed, and the claim that re-derivation is cheap is unmeasured.

**Belief has no credence model.** [Credence from counting evidence](belief.md), with dependence detection as the load-bearing part. The representation is settled; the arithmetic is deliberately minimal because named critics attack the operators the richer version would need. The exact shape remains a live disagreement between the research lanes.

**Hygiene thresholds are embedder geometry.** Better than it first appears, and incomplete. Structural equality removes similarity from the deduplication path entirely, which is where the measured damage was: one re-phrased fact sitting at 0.966, 0.851, and 0.757 under three phrasings. Consolidation and retrieval still use geometry, now calibrated to the current embedder's own distribution and recomputed when it changes, with ranking drawing on access recency and frequency as well. The residual is that the analogy from human recall to agent salience is an analogy.

**Load-bearing behaviour is prompt-sensitive.** [Forced-choice elicitation](the-seam.md) collapses omission variance, which is what produced the 6%-to-75% swing. It relocates variance into field content, introduces junk fill, and costs an unmeasured constraint tax. This is the shakiest of the five, and the design's own answer is to measure per behaviour rather than assume.

**The neural writer is unverified.** [Hard critics](the-seam.md) check typing, domain and range, mutual exclusion, temporal well-formedness, audience invariants, and duplicate resolution. **They do not check truth.** A confidently recorded, well-typed falsehood passes at write time exactly as it does today. Faithfulness checking at runtime is unsolved here as it is across the field, and the mitigations, agreement before promotion and drift detection from outside the loop, reduce the rate rather than close the gap. This is the largest residual in the design.

## What this design makes worse

The second trace is not free, and three of the five partial answers get harder because of it.

**Narrative generation is a new prompt-borne load-bearing surface.** The design's rule is that generative work stays on the prompt because it tolerates drift. A narrative is generative by construction, and it is now load-bearing. The prior art's own pilot reports that a directive to use its protocol was reliably overridden by the model's trained default and had to be moved into the system prompt to take effect, which is class 10 reappearing inside the fix. The containment is to make the existence and linkage of a trace structural while leaving only its quality soft, and to measure content correctness rather than presence.

**Narrative is a new geometry-sensitive index.** The survey measured its widest similarity variance in exactly the long-text regime a narrative occupies: 0.80 against 0.94 for the same content under different prefixes, varying by length. The new index needs the same distribution-relative calibration as every other, and inherits none of the protection that structural equality gives the deduplication path.

**Narrative licenses invention.** The mechanism works by asking a model to commit to concrete detail it was not told. The instance has already produced the unelaborated version of that failure. This is why the [episodic wall](the-seam.md) is a critic rather than a sentence, and why it should ship before any narrative generation rather than alongside it.

## Two mitigations the design erases

The survey notes that a mitigation in the current ontology is itself evidence of a workaround tax the redesign should erase. Two prompt-borne mitigations become structure:

- The write-time cross-subject advisory, which steers the agent around the one-subject representation, becomes the Event node. There is nothing left to steer around.
- The temporal extraction's third-party-routine rule, which teaches the model not to stamp a recurrence on a fact describing someone else's job, becomes the absence of a Trigger. The rule is unnecessary because the outcome is unrepresentable.

Both are load-bearing behaviours moving from wording into structure, which is the class 10 remedy applied to classes 2 and 4.

## Issues

### Directly addressed

| Issue | Becomes |
|---|---|
| **#112** episodic session recaps | An [episode](memory-typology.md): a first-class memory with span, participants, turn references, and a narrative body, linked bidirectionally to the Statements recorded during it |
| **#74** search past conversations | Reframed from fallback to companion. The episode anchor rides the search result; verbatim turn search remains as the tier below, under the existing audience gate |
| **#114** fabricated content attributed to a teller | The [episodic wall](the-seam.md) as a hard critic: agent-told only, never a premise, never distilled, never attested. Prerequisite for shipping narrative generation |
| **#113** undated events stamped with the assertion day | The episode holds "when I learned this", so [`valid`](time.md) can stay open without the claim falling out of the timeline |
| **#115** date correction needs a full-text supersede | The occurrence is a field; correcting it leaves the gloss untouched, because the person's words did not change |
| **#106** volatile facts never age | The temporalise-annotate-reverify-retire ladder, using the episode date as the "as of" rather than a guess |
| **#44** long-document ingestion | Semantic Statement clusters plus an episodic source layer, with the observed-against-recorded split making delayed ingestion coherent |
| **#42** relation schemas cannot be edited | Deprecate-and-alias |
| **#94** autonomous identity unification | Revocable graded merges, relational evidence, tiered reversibility |
| **#104** merged identity fails to relay sibling history | The substrate wall, with unified reads across the class |
| **#100** in-block neural calls | Record-at-call-time activities, exposed primarily as schema-constrained functions |
| **#103** typed dates and durations | First-class typed values, extended to quantities |
| **#58** procedural memories | The procedural kind, indexed by description, decayed by invocation |
| **#59** persistent scratchpad | The working kind, outside the visibility model. Storage remains open |
| **#90** eval corpus redundancy | The four-capability taxonomy with a required null arm |
| **#125** agent-authored occurrence dates an entry to another referent's date | The [referential frame](statements.md). This is the frame failure in its temporal form, and it was filed independently of the corpus study that found the general case |
| **#126** brief names a participant by their arrival stub | The [substrate wall](identity.md): one resolved handle, resolved before anything is composed |
| **#127** redaction decided per read path, so a new path leaks by omission | Visibility is computed once in the substrate before rendering, and zero residue is held as a [non-interference invariant](privacy-and-provenance.md) rather than as a rule each read path must remember |
| **#124** agent refuses a fact its own brief surfaced | The same single resolution point: what was surfaced and what is sayable are computed from one predicate, so they cannot disagree |

### Answered obliquely

| Issue | Note |
|---|---|
| **#7** persistent-memory landscape | Discharged by the surveys in [`research/`](research/) |
| **#93** challenge-response for cross-platform merges | Retained as the gate on irreversible disclosure. The design makes it load-bearing rather than optional, since recall and disclosure are separated |
| **#15** self-observations | The agent writes observations about itself as an ordinary fallible teller; the operator-fixed charter is a directive and outside the fact model |
| **#20** autonomous activity | The exception queue and drift detection are its skeleton |
| **#105** API reference cost | A standing constraint on the [query surface](query-surface.md), which is why co-retrieval rides the search result rather than adding a call |

### Made worse

**#66** (the console replica must bound its event-log mirror and time-travel window) moves from deferred to blocking. The console holds the whole log in browser memory and re-folds it from zero on every time-travel scrub, and this design multiplies the dominant term in log size: recorded model calls are already 96% of payload bytes in the live instance, and structuring adds a call per write block on top. Everything the design moves into the fold, severance filtering, alias resolution, credence derivation, frame defaulting, is then paid per scrub. A measured budget, bytes added per turn and browser fold time at realistic log sizes, is a prerequisite rather than a follow-up.

### Inherited, not solved

**#123** (the present set conflates audience with participation, leaking confidences to silent channel members) is a warning this design must take seriously rather than a problem it fixes. [Transmission principles](privacy-and-provenance.md) are predicates over *who is present*, which makes the definition of "present" load-bearing for every audience decision in the model. If that set is wrong, richer conditions evaluated against it are wrong more expressively. Defining presence correctly, separating being in a channel from being in the conversation, is a prerequisite for the privacy chapter rather than a consequence of it. The same definition decides the witness set on a gloss, which is the one field in the model that widens an audience rather than restricting it, so a wrong answer here is wrong in both directions at once.

### Not addressed

**#96**, **#97** (mint races), **#99**, **#118**, **#119**, **#120**, **#121**, **#72**, **#75**, and **#1** are implementation and tooling concerns orthogonal to the data model. **#109** and **#110** are connector gaps. **#116** is a logging gap. **#18** (subagent spawning) is a capability question this design neither needs nor blocks. None is made harder by this design, and none is made easier.

## New work this design creates

Not everything here is a fix. Three items are new obligations that did not exist before:

- **The referential frame** is a new concept the agent must be taught to set, and a new axis every read must resolve. It buys correctness on 39% of the observed corpus and costs a field on every Statement and a decision on every write.
- **The episodic layer** costs a record-time model call and permanent log volume per occasion, unless the ablation shows the benefit is retrieval-side.
- **The critic bank** is code that did not exist, and every critic is a place a correct write can be wrongly rejected. A persistent rejection that indicates a schema gap rather than a mistake is one of the four things that reaches a person.
