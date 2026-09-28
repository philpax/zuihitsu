# The neural-symbolic seam (the welding)

Research lane for the zuihitsu ontology redesign. The core question: every durable
symbolic knowledge system in the historical record assumed a patient human curator. zuihitsu
substitutes a neural agent that lives in the moment, has a small working set, and whose
load-bearing behaviours ride on prompt wording. The measured fragility is failure class 10:
a capture behaviour moved from ~6% to 75% of eligible cases on a one-sentence scaffold
change. This lane asks where the neural/symbolic boundary should sit, what verifies the
neural writes, which behaviours must move from prompt to structure, how in-block LLM calls
(#100) fit the deterministic log, and how to measure "robust, scalable, accurate, flexible"
welding.

A note on source quality: several primary PDFs (the Zep paper arXiv:2501.13956, FormatSpread
arXiv:2310.11324, the LLM-Modulo paper arXiv:2402.01817) did not extract cleanly through the
fetch tool, so some of their specifics below come from secondary summaries and the abstracts.
Numbers sourced that way are flagged. The conceptual claims are well-corroborated across
multiple sources; the exact figures should be treated as approximate pending a direct read of
the tables.

---

## 1. NELL: a decade of autonomous accumulation, and what drifted

NELL (Never-Ending Language Learner, Carlson, Betteridge, Hruschka, Mitchell et al., CMU) is
the closest historical analogue to zuihitsu's ambition: a system that runs continuously for
years, reads the open web, and grows a knowledge base without stopping. It ran from January
2010 to at least iteration 1115 on 3 September 2018, accumulating over 80 million
confidence-weighted beliefs ([Mitchell et al., "Never-Ending Learning", CACM May
2018](https://cacm.acm.org/magazines/2018/5/227193-never-ending-learning/fulltext); [Wikipedia:
Never-Ending Language Learning](https://en.wikipedia.org/wiki/Never-Ending_Language_Learning)).

### What drifted

The central pathology NELL fought its whole life was **semantic drift**: a bootstrapped
extractor, trained on its own promoted beliefs, gradually generalises the wrong way and starts
promoting garbage, and because the garbage is fed back as training data the error compounds.
The published characterisation is stark: **precision declined to roughly 57% after 66
iterations** when the system was left to self-train, and the per-predicate variance was
enormous, with some relations above 90% precision and others below 50% "due to semantic drift
or web noise" ([search-sourced from the NELL architecture literature; corroborated by the
Grokipedia NELL summary](https://grokipedia.com/page/never_ending_language_learning) and
[Carlson et al. coupling-constraint work](https://www.cs.cmu.edu/~wcohen/postscript/bionlp-2012-bionell.pdf)).
The system's own designers noted they were "still trying to understand what causes NELL to
become increasingly competent at reading some types of information, but less accurate over
time for others" — i.e. drift was directional and category-specific, not uniform noise.

### What the coupling constraints and periodic human touch bought

NELL's defence against drift was **not** a better extractor. It was two things:

1. **Coupled semi-supervised learning under ontology constraints.** NELL never trained one
   extractor in isolation. It trained many extractors simultaneously and forced their outputs
   to agree with structural constraints drawn from the seed ontology: **mutual exclusion**
   (a thing can't be both a `city` and a `person`), **type checking** on relation arguments
   (the domain/range of `playsFor` must be `athlete` x `team`), **subset/superset** relations
   between categories, and agreement between independent views (text-pattern extractors vs.
   HTML-structure extractors vs. morphology) ([Carlson et al., "Coupling Semi-Supervised
   Learning of Categories and Relations", NAACL-HLT SSL workshop
   2009](https://www.cs.cmu.edu/~wcohen/postscript/bionlp-2012-bionell.pdf); CACM 2018). The
   ontology's structure was the error-suppression mechanism: a candidate belief that violated a
   type or mutual-exclusion constraint was rejected before it could contaminate training. **The
   symbolic schema did the disposing; the extractors only proposed.**

2. **A small, periodic human touch.** Even with coupling, NELL degraded without a curator. The
   published operating discipline was roughly **5 minutes of human supervision per relation
   every 10 iterations** to correct promoted beliefs and re-anchor the extractors ([search-sourced;
   corroborated in the NELL semi-supervised bootstrapping literature]). This is a tiny amount of
   human labour, but it was not zero, and it was periodic and structural (per-relation), not
   ad-hoc. NELL later added belief revision via consistency constraints and classifier
   retraining, recovering to ~85% mean precision on top-10 novel predictions by iteration 886.

### Why it ended, and the drift lessons

There is no dramatic published "post-mortem" declaring NELL dead; active public iteration
simply halted in 2018 and the project wound down as its PIs moved on. The retrospective of
record is the CACM 2018 article, which is a triumphal architecture paper, not a failure
autopsy. But the operational record yields hard lessons for **any autonomous accumulator**:

- **Drift is the default, not the exception.** A self-supervised system that trains on or
  reasons over its own outputs will drift unless something outside the neural loop constrains
  it. zuihitsu's maintenance passes (dedup, consolidation, canonical profiles) that
  re-derive structure from the agent's own prose are structurally a self-training loop and are
  exposed to exactly this. (Failure classes 1, 9, 11.)
- **The ontology is the drift brake.** NELL's error suppression came from typed, mutually
  exclusive, constraint-checked structure — precisely what zuihitsu currently lacks, since
  "facts are sentences" (failure class 1) means there is no type or mutual-exclusion constraint
  a candidate write can violate.
- **Autonomy is not the same as zero human touch.** NELL's designers, who wanted the system to
  run forever, still built in a periodic per-relation human check because they could measure
  that precision cratered without it. The relevant design target for zuihitsu (#15's
  "zero-administration endgame, operator intervenes only on exceptions") should be read as
  *exception-triggered* human touch backed by drift *detection*, not *no* human touch.

---

## 2. LLM-extraction pipelines with temporal knowledge graphs — what verification do they run?

The modern lineage most like zuihitsu's write path is the class of systems that use an LLM to
read prose and emit graph structure into a temporal knowledge graph. The headline finding:
**essentially none of them independently verify the neural write against the source. The LLM
is both the writer and its own (only) judge.** This is failure class 11 reproduced across the
whole field.

### Graphiti / Zep (the closest architectural cousin)

Zep is an agent-memory service; Graphiti is its temporal-knowledge-graph engine ([Zep:
A Temporal Knowledge Graph Architecture for Agent Memory,
arXiv:2501.13956](https://arxiv.org/abs/2501.13956); [Neo4j: Graphiti — knowledge graph memory
for an agentic world](https://neo4j.com/blog/developer/graphiti-knowledge-graph-memory/)). Its
mechanics, which map almost one-to-one onto zuihitsu's, are:

- **Bi-temporal edges.** Every edge carries explicit validity intervals (`t_valid`,
  `t_invalid`) separate from ingestion/transaction time. This is exactly the bitemporal
  asserted/occurred split zuihitsu already has, and it is precisely what zuihitsu's **bare
  edges lack** (failure class 3: "worked at X 2019-2021 cannot be a relation"). Graphiti proves
  the temporal-interval-on-edge design is viable at production scale.
- **Contradiction handling by invalidation, not deletion.** When a new fact temporally overlaps
  and contradicts an existing edge, Graphiti sets the old edge's `t_invalid` to the new edge's
  `t_valid` — it invalidates but never discards, preserving the historical record ([Zep blog:
  "Beyond Static Knowledge Graphs"](https://blog.getzep.com/beyond-static-knowledge-graphs/);
  search-corroborated). This is the tombstone/supersession discipline zuihitsu already uses,
  validated as the right shape.
- **Entity resolution is LLM judgement over hybrid retrieval.** New nodes are resolved against
  existing ones using combined semantic, keyword, and graph search, and **the LLM decides**
  whether the new mention is the same entity or a conflict ([Neo4j blog, above]). There is no
  independent structural check and no verification against source text.

**The reported failure mode is the one that matters most for zuihitsu.** The ATOM paper finds
that "Graphiti, whose incremental, LLM-based entity and relation resolution **degrades with
graph expansion**" ([ATOM: AdapTive and OptiMized dynamic temporal knowledge graph
construction using LLMs, arXiv:2510.22590](https://arxiv.org/pdf/2510.22590)). As the graph
grows, the LLM's per-write resolution judgement gets less reliable — the accumulator's own
accuracy decays over its lifetime, the same shape as NELL's drift and zuihitsu's failure class
9 (thresholds that silently rot). Zep's own reported strengths are latency and
benchmark accuracy on DMR and LongMemEval, not write-fidelity guarantees — the paper does not
claim (and the architecture does not provide) verification that an extracted edge is faithful
to the utterance that produced it.

### GraphRAG (Microsoft) and adjacent extractors

MSR GraphRAG runs an LLM over each text chunk to extract entities and relationships **with
natural-language descriptions**, with multiple "gleaning" passes to improve recall
([Microsoft GraphRAG methods](https://microsoft.github.io/graphrag/index/methods/)). The
documented, tracked failure mode is direct: **"LLM tends to / can invent descriptions for
entities and relationships"** that are not supported by the source chunk ([microsoft/graphrag
issue #1543](https://github.com/microsoft/graphrag/issues/1543)). The recommended fix in the
issue thread is *prompt polishing and few-shot examples to discourage invention* — i.e. the
field's default remedy for an unverified neural writer is **more prompting**, which is exactly
the prompt-sensitivity trap (failure class 10). There is no structural verifier; grounding is
aspirational.

LlamaIndex KG extraction and the broader "LLM emits triples" pattern share this shape: the LLM
is the sole author of structure, and validation, where it exists at all, is schema-shape
validation (is this valid JSON / a well-typed triple?) rather than **truth or
source-faithfulness** validation. The one recent counter-move worth noting is treating the LLM
graph as a *noisy sensor* and comparing it against a deterministic rule-built baseline graph
(see §6).

**Synthesis for the lane.** The entire LLM-to-KG field verifies *shape* (schema validity) and,
at best, *internal consistency* (contradiction detection between edges). Almost none of it
verifies *faithfulness* (does this edge actually follow from the text?) or *drift* (is the
writer's accuracy decaying?). zuihitsu's failure class 11 ("the neural writer is unverified")
is not a zuihitsu-specific gap — it is the state of the art's blind spot, which means solving
it is genuinely novel work, and copying Graphiti/GraphRAG wholesale would import the blind
spot.

---

## 3. Differentiable / probabilistic logic and LLM-modulo — where to put the boundary

This section is *not* an adoption proposal (zuihitsu is not going to embed a Datalog
differentiator). It is about the **boundary-placement doctrine** these systems teach, which is
directly transferable.

### Neural proposes, symbolic disposes — at a crisp, typed interface

- **DeepProbLog** places the boundary at the **predicate**: a *neural predicate* is a ground
  atom whose probability is produced by a neural net (`digit(img, 7) :- p=0.87`), and a
  probabilistic-logic program does all the reasoning over those atoms ([DeepProbLog: Neural
  Probabilistic Logic Programming, Manhaeve et al., NeurIPS
  2018](https://dl.acm.org/doi/10.5555/3327144.3327291)). The neural net never reasons; it only
  scores atoms. The logic never perceives; it only composes.
- **Logic Tensor Networks** place the boundary at **fuzzy first-order predicates**, grounding
  logical formulae in real-valued tensors so constraints are differentiable ([Badreddine et
  al., "Logic Tensor Networks", Artificial Intelligence
  2022](https://www.sciencedirect.com/science/article/abs/pii/S0004370221002009)).
- **Scallop** places the boundary at the **relation**, and adds the piece most relevant to
  zuihitsu: **provenance semirings**. Scallop tags every tuple with a provenance value and
  propagates it through Datalog reasoning; by choosing the semiring you get discrete,
  probabilistic (`minmaxprob`), or differentiable reasoning over the *same* program, and the
  provenance records *how* each derived conclusion was obtained ([Scallop: A Language for
  Neurosymbolic Programming, PLDI 2023, arXiv:2304.04812](https://arxiv.org/abs/2304.04812);
  [Scallop: From Probabilistic Deductive Databases..., NeurIPS
  2021](https://www.cis.upenn.edu/~mhnaik/papers/neurips21.pdf)).

**The transferable lessons, in order of importance to zuihitsu:**

1. **The neural component outputs *proposals with weights*, and a separate sound layer composes
   them.** The neural half never gets to be the final authority on a structural question. This
   is the exact inversion of zuihitsu's failure class 11, where the model is the sole writer of
   structure with no disposing layer.
2. **Provenance is first-class, computed *with* the conclusion, not attached as a post-hoc
   note.** Scallop's provenance semiring answers "why does the KB believe this?" as a
   structural artifact of derivation. zuihitsu's judgement provenance currently records
   "model + template" but **not the evidence or criteria** (failure class 11); Scallop shows
   provenance can carry the actual derivation lineage, and that this is what makes belief
   revision (removing a conclusion and everything derived from it) tractable — the ATMS
   direction #94 already points at.
3. **The boundary must be *typed and crisp*.** DeepProbLog's neural predicate has a fixed
   arity and type; the neural output cannot smuggle unstructured prose across the seam. zuihitsu's
   seam is currently a prose sentence — the maximally *un*-typed interface, which is why every
   downstream mechanism must re-parse it (failure class 1).

### LLM-modulo (Kambhampati): the doctrine for LLM-scale neural components

Where DeepProbLog/Scallop assume a small trainable net, LLM-modulo is the same doctrine sized
for an LLM ([Kambhampati et al., "LLMs Can't Plan, But Can Help Planning in LLM-Modulo
Frameworks", ICML 2024 position paper,
arXiv:2402.01817](https://arxiv.org/abs/2402.01817)):

- **Core claim: auto-regressive LLMs cannot self-verify.** Self-verification "is after all a
  form of reasoning," which the position paper argues LLMs cannot reliably do. Therefore
  **an LLM must not be trusted to check its own output** — the check must come from outside.
  This is a direct verdict on the current zuihitsu maintenance passes, which use *model
  judgement* to police *model-written* structure: that is an LLM grading itself.
- **The generate-test loop with a bank of critics.** The LLM generates candidates; a suite of
  external **critics** evaluates them; failing candidates are returned with feedback for
  regeneration. Reported: Blocks World planning rose to 82% within ~15 feedback rounds from a
  sound model-based verifier ([UnfoldAI summary of
  LLM-Modulo](https://unfoldai.com/llm-modulo-framework/); [MLR Press,
  v235/kambhampati24a](https://proceedings.mlr.press/v235/kambhampati24a.html)).
- **The critic taxonomy — and which critics must be sound.** Two classes:
  **model-based critics** that rely on formal domain models and algorithms to guarantee
  *soundness and correctness* (executability, type/constraint satisfaction — these are the ones
  that must be sound and can be symbolic), and **LLM-based critics** for *soft* aspects only
  (style, coherence, user-preference fit) where being occasionally wrong is tolerable ([UnfoldAI,
  above]). The design rule: **hard properties get a sound symbolic verifier; only soft
  properties may be judged by another LLM.**
- **The roles the LLM legitimately plays:** candidate generation, translation between formats
  (prose to formal spec and back), fleshing out an under-specified problem, and helping acquire
  the domain model / critics themselves. In every one of these the LLM is a proposer or a
  translator, never the final arbiter of a hard constraint.

**Boundary-placement synthesis for zuihitsu.** The neural agent should sit on the *proposal*
side of a typed seam. Anything that is a hard structural property of the ontology — a type
constraint, a mutual-exclusion rule, an audience-visibility invariant, a
merge/`same_as` decision, a temporal-interval well-formedness check — must be disposed by a
**sound symbolic verifier**, never by model judgement. Only genuinely soft judgements (is this
prose a good summary? are these two descriptions stylistically the same claim?) may ride on an
LLM critic, and even then should be sampled-and-voted rather than single-shot (§4).

---

## 4. Prompt sensitivity and behavioural robustness — which behaviours must never ride on wording

This is failure class 10 in miniature, and the literature quantifies exactly how dangerous
prompt-borne behaviour is.

### The brittleness is large, persistent, and non-transferable

**FormatSpread** ([Sclar et al., "Quantifying Language Models' Sensitivity to Spurious
Features in Prompt Design", ICLR 2024,
arXiv:2310.11324](https://arxiv.org/abs/2310.11324)) measured accuracy *spread* across
semantically-equivalent prompt formats:

- Up to **76 accuracy points** of spread on LLaMA-2-13B from formatting changes alone that
  preserve meaning; GPT-3.5 showed spreads up to **56 points with a median of ~6.4 points**
  across 320 formats and 53 tasks (numbers search-sourced; the PDF did not extract cleanly).
- **Scale, few-shot examples, and instruction tuning do not fix it** — sensitivity persists
  through all three ([FormatSpread abstract and secondary summaries]).
- **Format rankings do not transfer across models** — the best format for one model is not the
  best for another, so you cannot "solve" it once and reuse the solution across model
  generations.

Corroborating work: "A Single Character can Make or Break Your LLM Evals"
([arXiv:2510.05152](https://arxiv.org/pdf/2510.05152)) and the format-robustness line
([arXiv:2504.06969](https://arxiv.org/abs/2504.06969)) confirm the effect is a general property
of instruction-tuned LLMs, not an artifact of one benchmark.

**The direct implication for zuihitsu:** the ~6%-to-75% capture-rate jump on one scaffold
sentence is not an anomaly or a bug to be prompted away. It is the *expected* behaviour of a
load-bearing action that rides on prompt wording, and it will *reappear on every model
upgrade* because format rankings do not transfer. Any behaviour whose *failure is a
correctness or safety failure* must be moved off the prompt.

### The mitigation architecture: move behaviour from prompt to structure

The robust pattern, well-supported across the structured-output literature, is to replace
"ask the model nicely to do X" with "make X the only structurally available move":

1. **Forced-choice tool calls instead of free recall.** Constraining the model to fill a schema
   via forced tool invocation (`tool_choice` = required) makes it "fill out a structured format
   instead of outputting random text," and providers have fine-tuned models to adhere to the
   schema far more reliably than any prompt instruction ([search-sourced from the structured-output
   guides; e.g. the constrained-decoding literature]). The behaviour "extract the subject" moves
   from *"please remember to record the subject"* (prompt-borne, 6%-brittle) to *a required field
   in a tool schema the turn cannot complete without filling* (structure-borne). Whether the
   agent captures is no longer a function of scaffold wording.
2. **Harness-driven checklists / state machines.** The load-bearing sequence (did we resolve the
   speaker? did we place the fact in time? did we set visibility?) becomes a state machine the
   *harness* steps through, prompting the model only for the leaf judgements, rather than a
   procedure the model is trusted to remember to run. The harness, not the prompt, guarantees the
   steps happen.
3. **Sampling and voting (self-consistency).** For the leaf judgements that remain neural,
   sample N and take the consistent answer; facts that appear consistently across samples are
   substantially more likely to be true, and this improves factuality by a reported 31-35% over
   single-shot ([search-sourced self-consistency / FactSelfCheck literature; arXiv:2503.17229]).
   "Structured Self-Consistency" combines schema validation (catches broken output) with
   structure-aware voting (catches semantic instability).
4. **Regression evals across model generations.** Because brittleness does not transfer, the
   only durable defence is a behavioural eval suite that re-measures every load-bearing rate on
   every model change — which zuihitsu already has the harness for (§6).

**A caution.** Structured constraints are not free. The "Constraint Tax" study finds that
imposing structured-output constraints can *suppress* tool-calling or degrade reasoning in some
open-weight models ([arXiv:2606.25605](https://arxiv.org/pdf/2606.25605)). So the move is not
"schema-constrain everything" — it is "schema-constrain the *load-bearing structural writes*,
and leave the model free-form for the *generative/deliberative* parts," with an eval to confirm
the constraint did not cost more than it bought.

### The decision rule

**Which behaviours must never ride on prompt wording:** any behaviour where the *failure mode
is a silent correctness or safety failure* — capture of a fact, setting of a visibility
posture, resolution of a speaker, placement of a temporal interval, the decision to
fire a wake-up. These are exactly the behaviours where a 6%-vs-75% swing is catastrophic.
**What can stay prompt-borne:** genuinely generative or stylistic behaviour where "the model
did it a bit differently today" is harmless — how a description is phrased, how the agent
converses, which of several valid framings it chooses. The scaffold should teach *principles
and taste* (which tolerate wording drift); the *harness* should enforce *load-bearing steps*
(which do not).

---

## 5. Neural judgement inside symbolic transactions (#100) — the idempotency problem

#100 wants an in-block `llm` call: a Luau block suspends, the model responds, the block acts on
the response and commits a derived result in the same transaction. The block VM already does
this for `web.markdown` (a suspending async call inside the block's timeout budget). The sharp
edge the issue itself identifies: **`web.markdown` deliberately does *not* latch the block's
"made an external call" flag because a GET is idempotent and a retried block re-fetches
harmlessly; an LLM call is *not* idempotent** — it spends tokens and may return a different
answer each time — so a retried block could branch on a *different* response than the one whose
derived result it commits.

This is a solved problem in the durable-execution world, and the solution is precisely the rule
zuihitsu already lives by.

### The prior art: Temporal's determinism model

Temporal (the workflow engine) draws exactly the line zuihitsu needs. Its **workflow code must
be deterministic** to support replay: on replay it does not re-execute, it reconstructs state
from recorded results ([Temporal: Workflow
Definition](https://docs.temporal.io/workflow-definition)). Every **non-deterministic
operation — and the docs name "API calls, LLM/AI invocations, database queries" explicitly —
must live in an *Activity*, which executes *outside the replay path*; its result is recorded
into the workflow's event history the first time and *reused* on replay** ([Temporal docs,
above; "Replay Testing To Avoid Non-Determinism in Temporal
Workflows"](https://www.bitovi.com/blog/replay-testing-to-avoid-non-determinism-in-temporal-workflows)).
The community formulation is blunt and directly on point: **"You cannot replay an LLM call and
pretend it is the same event. The output must be recorded the first time and reused during
recovery"** ([Koshy, "Agent Workflows Are Rediscovering Durable
Execution"](https://nittikkin.medium.com/agent-workflows-are-rediscovering-durable-execution-be110661ed8c)).
And because Activities *are* retried on infrastructure failure, "idempotency is
non-negotiable" — a retried side effect must not double-apply.

This is **the same rule zuihitsu already states**: model and embedder calls happen at record
time only, never at replay; the log is the sole source of truth; replay is a pure function of
the log. Temporal is independent confirmation that zuihitsu's existing determinism discipline
is the correct and standard answer, and it tells us exactly how #100 must be built.

### How #100 fits the deterministic log

The in-block LLM call is a non-deterministic Activity, so it must **record its result into the
event log at call time**, and on replay the block must **read the recorded response rather than
re-calling the model**. Concretely:

- The `llm.complete(prompt)` native function, on first (record-time) execution, calls the model
  and **emits a `ModelCalled`-style event carrying the prompt and the response** into the log
  before returning to the block. On replay, the same call site consumes the recorded response
  from the log — no model call, deterministic continuation. This is the identical treatment
  `ModelCalled` already gets in the turn loop; #100's block-level call is just another
  record-at-call-time site.
- **The latch is right, but the log makes it almost moot.** Because the response is recorded,
  a *replay* never re-spends tokens and never branches on a different answer — the log pins the
  answer. The latch (making a post-LLM-call block timeout a terminal error with no
  *live* retry) protects only the *first, live* execution, where a transient timeout after the
  model already responded must not cause a re-drive that re-calls the model. So: latch on first
  live call (as the issue proposes), *and* record the response so replay is deterministic. The
  two mechanisms cover the two distinct hazards (live re-drive vs. log replay).
- **Provenance.** Per Scallop's lesson (§3) and failure class 11, the recorded `llm` event
  should carry not just model + prompt but enough to reconstruct *why* the block committed what
  it did — the prompt, the response, and ideally the criterion the block applied. This is the
  in-block analogue of judgement provenance, and it is what lets a later audit or belief
  revision (§6) unwind an `llm`-derived write.
- **Supersession.** A block mid-`llm`-call should be cooperatively cancellable at the model
  stream boundary, exactly as a mid-model-stream turn already is (the `select!` on the
  supersession watch). The recorded-partial discipline (`ModelCallAborted`) already exists and
  should extend to the block-level call.

**The design tension #100 surfaces for the redesign.** An in-block `llm` call is *the welding
seam made explicit and reified*: it lets the agent invoke neural judgement inside a symbolic
transaction. That is powerful and dangerous in equal measure. It is powerful because it lets
the *harness* structure the call (a native `llm.extract_subject` with a baked-in schema is a
forced-choice structured elicitation per §4, not a prompt-borne behaviour). It is dangerous
because a *freeform* `llm.complete` puts an unverified neural writer back in the loop with
transaction-commit power (failure class 11). **The recommendation (below) is to expose #100
primarily as *native, schema-constrained* extraction/judgement functions — the structured
elicitation of §4 — and to treat any freeform `llm.complete` as the deliberative-only,
non-committing escape hatch.**

---

## 6. Evaluating the welding — probes for robust / scalable / accurate / flexible

The welding needs measurement along its four claimed virtues. The good news: zuihitsu's
existing eval harness (declarative scenario scripts + oracles + LLM judges over a replayable
event log) already generalises to ontology-level properties, because the log *is* the ground
truth and structural oracles can read it directly.

### Capture-rate and behavioural-robustness probes (accurate + robust; failure class 10)

- **Capture-rate probes with paraphrase batteries.** For each load-bearing behaviour (capture,
  visibility-setting, speaker-resolution, temporal placement), run the *same* scenario across a
  battery of *semantically-equivalent* phrasings and *measure the spread*, borrowing
  FormatSpread's methodology ([arXiv:2310.11324]). A load-bearing behaviour that has been
  correctly moved from prompt to structure (§4) should show **near-zero spread**; a wide spread
  is a red flag that the behaviour is still prompt-borne. This turns failure class 10 from an
  anecdote into a *continuously monitored metric*.
- **Structural oracles over the log, not needle-phrase judges.** The grounding brief's own
  scenario-design discipline ("prefer a structural check for an exact property; a list of needle
  phrases is a smell") is exactly right and is what lets an oracle assert "an entry with subject
  X and visibility Attributed exists in the log" rather than "the agent said the right words."
  Structural oracles are immune to the phrasing brittleness that plagues LLM-judge oracles.

### Extraction-fidelity probes (accurate; failure class 1, 11)

- **Gold-structure scenarios.** Author scenarios whose utterances have a *known-correct*
  structural extraction (the neo-Davidsonian event with its roles, the typed temporal interval,
  the visibility posture), and score the agent's actual writes against the gold structure —
  precision/recall on entities, roles, relations, and intervals. This is the KG-extraction
  field's standard fidelity measure, applied to zuihitsu's write path.
- **Faithfulness-to-source checks.** Directly targeting the GraphRAG failure (§2, LLM invents
  unsupported structure): an oracle that asserts *every* structural write in a scenario is
  entailed by some utterance in that scenario's transcript. A write with no supporting utterance
  is a hallucinated write — the failure class 11 signature — and should fail gating.

### Drift-detection probes over an agent's lifetime (robust + scalable; failure class 9, 11)

This is the NELL lesson operationalised. A one-shot eval cannot catch drift; drift is a
*longitudinal* property.

- **Canary facts.** Seed a set of known-true, known-false, and known-private facts at genesis
  and **re-probe them periodically** over a long simulated lifetime, asserting the true ones
  survive, the false ones stay rejected, and the private ones never leak. A canary that flips is
  a drift alarm. This is the standard "shadow/canary eval set" pattern ([search-sourced;
  model-drift monitoring literature].)
- **Periodic re-derivation audits.** zuihitsu's maintenance passes re-derive structure from
  prose; run them repeatedly over a growing log and **assert the derived structure is stable** —
  that dedup/consolidation does not oscillate, merge and un-merge, or accumulate contradictions.
  This directly probes the "self-training loop drifts" hazard (§1) and the "thresholds are one
  embedder's geometry" hazard (failure class 9): re-run the same audit after an
  `EmbeddingModelChanged` and assert the derived structure did not silently shift.
- **The LLM graph as a noisy sensor vs. a deterministic baseline.** The strongest published
  drift-monitoring design builds *two* graphs — a deterministic rule-built baseline and the
  LLM-generated one — treats both as noisy sensors, and uses **structural metrics plus
  hallucination scores fed to a time-series anomaly detector with dynamic thresholds** to flag
  persistent deviations rather than pointwise errors ([Continuous Monitoring of Large-Scale
  Generative AI via Deterministic Knowledge Graph Structures,
  arXiv:2509.03857](https://arxiv.org/html/2509.03857)). For zuihitsu, the deterministic baseline
  is the *structured* ontology (the neo-Davidsonian events, typed relations, intervals) and the
  neural-written prose entries are the noisy sensor; a growing divergence between what the
  structure says and what the prose says is a drift signal.

### Scalability probes (scalable; #44)

- **Bulk-ingestion fidelity at volume.** Recalling that Graphiti's LLM resolution *degrades with
  graph expansion* (§2), the eval must specifically test write-fidelity *at scale*: ingest a long
  document (#44), then assert the resulting cluster is navigable and the extraction precision did
  not fall off relative to a short-document baseline. Drift-with-size is the specific scalability
  failure this welding must not have.

### Generalising the harness to ontology-level properties

The harness generalises cleanly because **the property under test becomes a structural predicate
over the log**, and the log is deterministic. A behavioural oracle ("the agent kept the
confidence") and an ontology oracle ("no attestation is wider than its founding posture," "every
`same_as` edge has an operator author," "no entry references a retracted memory") are the *same
kind of object* — a pure function of the event log. The `rejudge` replay mode already lets an
oracle change be reclassified over recorded runs without re-running the model, which is exactly
what you want for iterating ontology-level oracles cheaply. The one addition the redesign wants
is **longitudinal scenarios** (a scenario that spans a simulated lifetime with periodic canary
re-probes and maintenance-pass audits) as a first-class scenario shape, because drift is the
failure mode a single-turn scenario structurally cannot see.

---

## Implications for zuihitsu

A concrete welding architecture, mapped to the failure classes this lane owns (1, 9, 10, 11)
and to #100.

### Where neural judgement sits: on the proposal side of a typed seam

Adopt the DeepProbLog/Scallop/LLM-modulo doctrine wholesale as *doctrine* (not as machinery):
**the neural agent proposes; a sound symbolic layer disposes.** The agent's writes cross the
seam as *typed structured proposals* (a neo-Davidsonian event with typed roles, a typed
temporal interval, a visibility posture, a relation instance with a typed schema), never as an
un-typed prose sentence. This is the structural repair for **failure class 1**: if the seam is
typed, no downstream mechanism has to re-parse prose, because the structure was captured at
write time. The prose can still exist as a human-readable *rendering* of the structured fact,
but it is derived-from and subordinate-to the structure, not the source of truth.

### What verifies the neural writes: sound symbolic critics, not model self-grading

Per LLM-modulo's core finding that **LLMs cannot self-verify**, replace "model judgement
polices model-written structure" with a bank of **sound symbolic critics** that run at write
time and can *reject* a write:

- **Hard critics (sound, symbolic, gating):** type/domain-range checks on relation arguments,
  mutual-exclusion constraints (NELL's drift brake), audience-visibility invariants (no
  attestation wider than founding posture — already a zuihitsu invariant, now enforced as a
  write-time critic), `same_as`-authority checks, temporal-interval well-formedness. A write
  that violates a hard critic is rejected with a teachable error — the pedagogy zuihitsu already
  uses, now backed by a sound check. This is the direct fix for **failure class 11**: the writer
  is no longer unverified, because every structural write must pass sound critics that check more
  than authority/visibility — they check *well-formedness against the ontology*.
- **Soft critics (LLM-based, sampled, non-gating):** "is this a good summary?", "are these two
  descriptions the same claim?" — sampled-and-voted (self-consistency), never single-shot, and
  never gating a hard property.
- **Provenance carried with the write (Scallop's lesson):** every structural write records the
  evidence (the utterance it derives from) and the criterion applied, not just model+template.
  This is what makes the drift audits and belief revision (#94's ATMS direction) possible, and
  it closes the specific failure-class-11 gap that "judgement provenance records model+template
  but not evidence or criteria."

### Which behaviours move from prompt to structure (failure class 10)

Draw the line at **failure mode**: any behaviour whose failure is a *silent correctness or
safety failure* moves off the prompt into harness-enforced structure; generative/stylistic
behaviour stays prompt-borne.

- **Move to structure:** fact capture, visibility-posture setting, speaker resolution, temporal
  placement, the schedule-vs-description distinction (failure class 4), the decision to fire a
  wake-up. Implement each as a **forced-choice structured elicitation** — a native tool/`llm`
  function with a required schema (§4) — so the behaviour is a structural requirement of
  completing the turn, not a scaffold sentence the model may or may not honour. The 6%-vs-75%
  swing disappears because capture is no longer a function of wording; it is a field that must be
  filled.
- **Keep on the prompt:** how descriptions are phrased, conversational style, which valid framing
  the agent chooses. The scaffold teaches *principles and taste*; the harness enforces
  *load-bearing steps*.
- **Never trust format-ranking stability across models:** because brittleness does not transfer
  (FormatSpread), the paraphrase-spread eval (§6) runs on every model upgrade as the regression
  gate.

### How #100 fits

Build the in-block `llm` call as a **record-at-call-time non-deterministic Activity**, exactly
per Temporal's determinism model (which independently validates zuihitsu's existing rule): the
call emits a `ModelCalled`-style event carrying prompt and response into the log on first live
execution, replay reads the recorded response (no re-call, deterministic), and the external-call
flag latches on first live call to make a post-response timeout terminal (covering the live
re-drive hazard the log-replay path does not). Expose it **primarily as native,
schema-constrained extraction/judgement functions** (`llm.extract_event`, `llm.classify_visibility`,
`llm.yes_no`) — which *are* the structured elicitation of the previous point, so #100 becomes the
delivery vehicle for moving load-bearing behaviour off the prompt — and treat any freeform
`llm.complete` as a *deliberative, non-committing* escape hatch, because a freeform call with
commit power re-introduces the unverified neural writer (failure class 11). Every `llm`-derived
write still passes the sound critics above.

### The evaluation probes, mapped

- **Failure class 10:** paraphrase-spread probes measuring capture/visibility/resolution rate
  spread across equivalent phrasings; a load-bearing behaviour correctly moved to structure shows
  near-zero spread. Runs as the model-upgrade regression gate.
- **Failure class 1 + 11:** gold-structure extraction-fidelity scenarios (precision/recall of the
  agent's writes vs. known-correct structure) and faithfulness oracles (every structural write is
  entailed by some utterance).
- **Failure class 9:** re-derivation audits after `EmbeddingModelChanged` asserting derived
  structure is stable (the specific probe for "thresholds are one embedder's geometry"), plus the
  deterministic-baseline-vs-neural-sensor divergence monitor.
- **Failure class 11 + NELL's lesson:** longitudinal canary-fact scenarios spanning a simulated
  lifetime, re-probing true/false/private canaries periodically; a flip is a drift alarm. This is
  the first-class *longitudinal scenario* the harness should gain, because drift is the failure a
  single-turn scenario cannot see, and because NELL is the standing proof that an autonomous
  accumulator drifts unless something outside the neural loop watches it.

### The one-line thesis

Every durable symbolic system that worked had a curator; zuihitsu removed the curator and put
an LLM in its seat. The welding repair is not a better prompt — it is to **stop asking the
neural writer to be the curator**: type the seam so the writer proposes structure not prose,
make sound symbolic critics dispose of every hard property, move load-bearing behaviour into
forced-choice structure the harness enforces, carry evidence-level provenance so writes can be
audited and unwound, and run longitudinal drift probes because — per NELL — an autonomous
accumulator with no watcher always drifts.
