# Convergent-evolution survey, part 1: the major systems, dead and living

Purpose: this is a survey for convergent evolution, not a shopping list for adoption. For each
system: source of truth, fact shape, identity model, forgetting/decay story, neural-symbolic seam,
what killed it or keeps it alive, and one model-agnostic insight. A synthesis section follows, then
implications for zuihitsu.

Confidence markers used throughout: claims drawn from primary sources (papers, official docs,
project maintainers) are stated plainly; claims drawn from secondary commentary, blog posts, or
third-party benchmarking are flagged **[secondary]**; anything I could not corroborate from more
than one source is flagged **[unverified]**.

---

## 1. Cyc

**Source of truth.** A single hand-curated symbolic knowledge base in CycL (a higher-order logic
dialect), built by paid ontologists and knowledge engineers over four decades starting 1984.
Assertions and rules are the unit; there is no automated ingestion pipeline analogous to NELL's web
reading — humans write the logic directly. By 2002 the project had consumed "$60 million and 600
person-years of effort from programmers, philosophers and others" [secondary, aiwiki.ai summary of
public reporting: https://aiwiki.ai/wiki/cyc]. A later retrospective figure puts the total at "30
million rules, $200 million, 2000 person-years" over Cyc's full lifespan [secondary,
https://yuxi-liu-wired.github.io/essays/posts/cyc/]. Even the *migration* cost of curated knowledge
was itself expensive: a 1995 progress report describes manually re-entering 100K concepts and 1M
assertions into a new representation language at a cost of 100 person-years
[https://aiwiki.ai/wiki/cyc] — i.e., the curated asset did not survive a representation-language
change for free; humans had to re-author it.

**Fact shape.** First-order-plus predicate logic assertions (`(predicate arg1 arg2 ...)`), each one
scoped to a **microtheory** (see below) rather than asserted globally.

**Microtheories as the context/privacy cousin.** A microtheory ("Mt") is a labelled bundle of
assertions sharing background assumptions — a domain, a time period, a naive physical theory, a
work setting. Axioms within one microtheory must be mutually consistent; axioms across different
microtheories need not be. Microtheories are first-class objects Cyc can reason about, and they
form an inheritance hierarchy (e.g. `#$GeometryGMt` specialises `#$MathMt`)
[http://www.gabormelli.com/RKB/CYC_Ontology]. The stated advantages: (1) shared background
assumptions are stated once at the microtheory level instead of repeated per-assertion, and (2)
terms with multiple meanings are disambiguated by which microtheory scopes them, permitting
modular growth — a new microtheory can be added for a new domain without disturbing the core
[same source]. This is structurally the closest thing in the pre-LLM literature to zuihitsu's
visibility postures: both are "the same referent means/behaves differently depending on which
frame of reference you view it through," solved by binding a scope tag to the assertion rather
than trying to write one global-consistent fact. The difference: microtheories are *logical*
consistency scopes chosen by the ontologist for reasoning tractability, not *audience* scopes
chosen for confidentiality — Cyc's microtheories were never built to keep a secret from a party
that could otherwise see the KB. They are a coupling-avoidance mechanism, not an access-control
mechanism.

**Identity model.** Constants (`#$Dave`) are the atomic named individuals; CycL has explicit
equality/identity assertions but the curation-first design means identity resolution was itself a
hand job — an ontologist decides two constants denote the same thing. No probabilistic or
programmatic merge process is documented as central to Cyc's design in the sources reviewed here
**[unverified beyond this]**.

**Forgetting/decay story.** None found. Cyc's assertions are additive and curated; there is no
documented automatic decay, supersession, or retraction mechanism analogous to what zuihitsu or the
cognitive architectures below have. This is consistent with hand-curation: a human ontologist is
the only party who retracts, and retraction is exceptional, not a routine operational primitive.

**Neural-symbolic seam.** For most of its life, none — Cyc predates the current neural-symbolic
framing. Lenat's final position, in a paper co-authored with Gary Marcus and submitted the month
before his death (31 July 2023), argued that "any trustworthy general AI will need to hybridize the
approaches, the LLM approach and more formal approach," combining LLM breadth with Cyc-style
explicit, inspectable knowledge and reasoning [secondary summary,
https://garymarcus.substack.com/p/doug-lenat-1950-2023; the paper itself is arXiv, not indexed by
title in my searches — treat the "16 desiderata" framing as **[secondary]** pending direct
verification]. So Cyc's own founder converged, at the very end, on the same neural-writes /
symbolic-checks seam zuihitsu already assumes — he just never got to build it.

**What killed it (partially) / keeps it alive.** Cyc has not fully died: Cycorp continues operating
commercially as of the sources reviewed, and Gary Marcus's 2023 retrospective notes it is "still in
business 40 years later," itself a rarity in AI-company survival terms [secondary,
https://garymarcus.substack.com/p/doug-lenat-1950-2023]. What did *not* happen is the thing Cyc was
supposed to prove: 40 years and $200M of hand curation did not compound into general common-sense
competence competitive with what LLMs produce for near-zero marginal cost per fact. The economic
argument is the load-bearing one: a curation pipeline whose unit cost is a human ontologist's
time does not scale faster than the world's fact-generation rate, and every representation-language
change re-taxes the entire accumulated asset (the 100-person-year re-entry event above is direct
evidence of this — curated knowledge is not portable for free across schema versions).

**One insight zuihitsu should take.** *Curation cost must be sub-linear in fact volume, or the
system dies of its own weight before its knowledge compounds.* Any mechanism zuihitsu adds that
requires a human (or even the agent, at model-call cost) to hand-verify each new structural
fact is importing Cyc's failure mode at a smaller scale. The microtheory pattern is validated
as an "assertions grouped by consistency-and-scope context" idea — but it teaches an *architectural
form*, not a licence to add curation cost: keep contexts to make consistency-checking local, but
never let entering a context/microtheory become a manual step.

---

## 2. NELL (Never-Ending Language Learning)

Kept brief per instructions; this lane covers only the structural convergence, since another lane
covers NELL's drift lessons in depth.

**Source of truth.** A perpetually running system, begun January 2010 by Tom Mitchell's group at
CMU (DARPA/Google-supported), that reads the public web continuously and accumulates beliefs into a
knowledge base — "over 80 million interconnected beliefs" at its peak
[https://en.wikipedia.org/wiki/Never-Ending_Language_Learning]. NELL's input at bootstrap is a seed
ontology of categories (e.g. `Sport`, `Athlete`) and binary relations between category members (e.g.
`AthletePlaysSport(x,y)`) plus a handful of labelled seed examples per predicate
[https://en.wikipedia.org/wiki/Never-Ending_Language_Learning].

**Structural convergence — categories/relations, coupling, promotion.** NELL runs many independent
extraction components in parallel (pattern-based text extractors, morphological/orthographic
classifiers, an image classifier, and structured-source readers), and couples them: a function
learned by one component (e.g. mapping a noun phrase to a category label) is constrained to be
*consistent* with what another component's learned relations imply about the same entities — the
"coupled learning" design [https://www.cs.cmu.edu/~tom/pubs/NELL_aaai15.pdf]. Each extracted
statement starts as a **candidate belief** with a confidence score; only candidates crossing a
threshold (documented at 0.9) are **promoted** into the trusted knowledge base that downstream
learners can condition on [secondary summary of the promotion threshold,
https://medium.com/@shagun/never-ending-learning-e7b78006e713 — corroborated at a system level by
the CACM/AAAI primary sources' description of "candidate" vs. "promoted" beliefs]. This
candidate/promoted two-tier structure is the same shape as zuihitsu's confidence-vs-asserted-fact
distinction the redesign is contemplating (issue #94's credence model), independently reinvented
for a fully automated pipeline instead of a partly-symbolic one.

**What NELL converged on structurally, in short.** (1) a fixed initial ontology of categories and
relations that the learners are coupled against, rather than free-form extraction; (2) multiple
independent, mutually-constraining extractors, because a single extractor's error mode is not
self-correcting; (3) a confidence-gated promotion boundary between "the system believes this
tentatively" and "the system will act on this as fact" — precisely the split zuihitsu's
"confidence vs. attested fact" credence model (issue #94) is reaching for independently.

**One insight zuihitsu should take.** Coupling and thresholded promotion are separable concerns
that reinforce each other: coupling is what keeps per-extractor error from compounding silently;
promotion thresholds are what keep tentative beliefs from contaminating the surface the rest of the
system treats as ground truth. A credence model for zuihitsu should keep the promotion boundary
*explicit and inspectable* (a number and a threshold, not a vibe), and where possible should couple
independent evidence sources (e.g., different tellers, or structural cross-checks) rather than
trusting a single extraction pass — which for zuihitsu is a single LLM call.

---

## 3. Soar and ACT-R, as living systems

Both are still actively developed cognitive architectures (Soar: John Laird's group, University of
Michigan; ACT-R: John Anderson's group, CMU), each with 30–40+ years of continuous use. Treat their
longevity itself as the primary evidence: they did not die, so what they converged on for memory
and learning is worth taking seriously as load-bearing rather than merely proposed.

### Soar

**Memory kinds.** Soar's architecture has working memory (the current state), procedural memory
(rules), and long-term declarative memory split into **semantic** and **episodic** stores
[https://arxiv.org/pdf/2205.03854]. Semantic memory holds context-independent facts as a graph and
retrieves by a combination of spreading activation and base-level activation (recency/frequency)
[https://www.researchgate.net/publication/221328941_Extending_the_Soar_Cognitive_Architecture, and
consistent secondary summaries]. Episodic memory records a snapshot of the top-state at every
decision cycle, but stores only *changes* since the last episode (a delta-encoding, not full
snapshots), and retrieves by cue similarity for recounting and lookahead planning
[https://www.researchgate.net/publication/315543437_An_Episodic_Memory_Retrieval_Algorithm_for_the_Soar_Cognitive_Architecture].
Efficient retrieval at scale (no significant slowdown as episode count grows) is achieved by
exploiting temporal contiguity, structural regularity of states, and cue selectivity
[https://www.sciencedirect.com/science/article/abs/pii/S2212683X14000164] — i.e., Soar treats
episodic-memory scaling as a first-class engineering problem, not an afterthought.

**Learning mechanism: chunking, driven by impasses.** When Soar's current knowledge is insufficient
to select or apply an operator, it hits an **impasse** and automatically creates a substate — a new
problem space — to resolve it, using the same problem-solving machinery recursively (giving a
uniform account of task decomposition, reflection, and planning, all via one mechanism)
[https://arxiv.org/pdf/2205.03854]. **Chunking** then compiles the substate's processing into a new
production rule that would produce the substate's result directly, so the *same* impasse does not
recur — chunking fires automatically whenever a substate produces a result, with the impasse and
its resolution as the trigger [https://intelligence.worldofcomputing.net/machine-learning/learning-by-chunking.html,
corroborated at architecture level by https://arxiv.org/pdf/2205.03854]. Chunking further composes
with reinforcement learning (RL values can be bootstrapped from chunked lookahead rollouts) and
with episodic/semantic retrieval (a substate's deliberation may itself invoke episodic recall, and
that recall's outcome gets chunked too) [https://arxiv.org/pdf/2205.03854].

### ACT-R

**Base-level activation and forgetting.** Each declarative memory chunk carries a base-level
activation (BLA) computed from the **power law of forgetting**:
`B_i = ln(Σ_j t_j^(-d))`, summed over the chunk's `n` prior uses, where `t_j` is time since the
j-th use and `d` is a decay parameter
[http://act-r.psy.cmu.edu/wordpress/wp-content/uploads/2012/12/652petrovAbstract.pdf, and
corroborated at a textbook level across multiple ACT-R sources]. Activation determines both
*whether* a chunk clears a retrieval threshold at all (below threshold, retrieval fails outright —
"forgetting" as a real functional event, not just slow recall) and, above threshold, how fast it is
retrieved [https://www.linkedin.com/pulse/act-r-architecture-from-cognitive-modeling-real-world-ankit-kashyap-ozzve,
https://arxiv.org/html/2505.05083v1]. This is recency+frequency decay applied uniformly across
*all* declarative memory, not a per-domain heuristic — a single formula, tuned by one global decay
parameter `d`, governs every chunk's forgetting curve.

**Convergence with Soar.** Both architectures independently landed on: (a) an explicit
semantic/episodic split in long-term declarative memory; (b) activation/strength as a first-class
scalar attached to each memory unit, combining recency and frequency, that gates both retrievability
and retrieval speed; (c) learning as *compiling deliberation into a faster-to-retrieve or
faster-to-apply form*, triggered by the deliberation event itself (an impasse in Soar; a retrieval
or production-firing event in ACT-R), not by a separate offline consolidation pass. Neither treats
"forgetting" as deletion — in both, a low-activation/never-chunked memory persists in the store
but becomes practically unreachable, which is a different design point from zuihitsu's current
retraction/supersession (explicit tombstones) or a hard delete.

**One insight zuihitsu should take.** Decay-as-retrievability-not-deletion, driven by a single
scalar computed from recency+frequency of *use* (not just of creation), is a 40-year-validated
alternative to zuihitsu's current binary retract/supersede model. It suggests a credence or
salience score on memories/entries that decays with disuse and refreshes on access, rather than
purely on explicit contradiction — useful for issue #94's credence direction, and orthogonal to (not
a replacement for) the visibility/audience model, which governs a different axis (who can see it)
from activation (how readily it surfaces).

---

## 4. OpenCog / AtomSpace

**Source of truth.** The AtomSpace: a hypergraph database where vertices and edges ("Atoms")
represent both data and procedure, queryable and rewritable in-place
[https://github.com/opencog/atomspace, https://wiki.opencog.org/w/AtomSpace]. Every atom carries a
**TruthValue**; the simplest, `SimpleTruthValue`, is a pair `(strength, confidence)` — strength
being the estimated probability of the assertion, confidence being the estimate's own reliability
[https://amit02093.medium.com/atomspace-hyper-graph-information-retrieval-system-450cab9d751e,
corroborated by https://wiki.opencog.org/w/AtomSpace]. This (strength, confidence) pair is
structurally identical to a Bayesian point-estimate-plus-precision, and is the closest prior-art
match to what issue #94 is asking for as a "credence model" — an explicit *strength of belief*
riding alongside every fact, distinct from whether the fact is present at all.

**Fact shape.** Hypergraph edges/links, not flat triples — an Atom can itself be a link connecting
other links, so higher-order relations (a relation about a relation) are native.

**Attention allocation — the STI/LTI economy.** Economic Attention Allocation (ECAN) assigns every
atom a **Short-Term Importance (STI)** and **Long-Term Importance (LTI)** value, modelled as
artificial currencies that spread through the hypergraph based on which atoms participate in
actions serving the system's current goals
[https://wiki.opencog.org/w/Attention_allocation]. STI governs which atoms are "in working memory"
(get processing/reasoning attention right now); LTI governs which atoms stay resident at all versus
get evicted from RAM/persisted store [https://wiki.opencog.org/w/Attention_allocation]. This is an
explicit economic model of the same problem zuihitsu's brief-composition and maintenance passes
solve heuristically (what goes in the prompt now vs. what stays in cold storage): OpenCog's answer
is a currency with conservation and spreading rules, not a fixed recency/relevance score.

**Neural-symbolic seam.** Historically thin and later formally attempted via PLN (Probabilistic
Logic Networks), begun by Ben Goertzel's group circa 2006 to unify symbolic inference with
probability theory over the AtomSpace [https://wiki.opencog.org/w/Probabilistic_logic_networks].
Notably, **PLN itself is now dead**: it "has been abandoned as of 2021 and is one of the OpenCog
Fossils" [secondary, search-result summary of
https://wiki.opencog.org/w/Probabilistic_logic_networks — the "Fossils" characterisation should be
treated as an OpenCog-community-internal judgement, not independently verified beyond the wiki
page]. The successor generation, OpenCog Hyperon, restates the same ambition — "combines
probabilistic logic, neural-symbolic reasoning, and multi-agent learning" — as of its 2023 paper
[https://arxiv.org/pdf/2310.18318], suggesting the neural-symbolic integration problem was never
actually solved by PLN and the project is on its second attempt.

**What stalled / keeps it alive.** The core AtomSpace graph database is "active, stable and
supported" as of the 2025 sources reviewed [secondary, search-result characterisation corroborated
by ongoing GitHub activity at https://github.com/opencog/atomspace]. But the *reasoning* layer (PLN)
is abandoned, and the embodied-agent layer is explicitly stalled: "progress on the Agents project is
stalled, until the deeper issues explored in the OpenCog (Sensori-)Motor project are resolved...
action-perception turned out to be far more complicated than initially thought"
[https://github.com/opencog/agents/blob/master/README.md]. So the honest read is: OpenCog's
*storage and representation* layer (hypergraph + truth values + attention economy) survived and is
still used; its *reasoning* layer, meant to be the actual intelligence, did not converge on a
working design in ~15+ years and was abandoned, then reattempted from scratch under a new name
(Hyperon). This is a sharper and more cautionary story than "OpenCog stalled" — the parts that are
*structural bookkeeping* (graph + scalar annotations + economic scheduling) outlived the parts that
were *supposed to reason*, which is a strong signal about which layer is actually tractable to get
right versus which is a standing research problem.

**One insight zuihitsu should take.** The **shape** of (strength, confidence) as a two-number
credence — not one scalar, but a value and a meta-value about how much to trust the value — is a
mature, 15+-year-tested representation for exactly the "belief has no credence model" failure
class (#8 in the grounding brief). But the cautionary half is equally load-bearing: OpenCog spent
comparable effort trying to make the *reasoning engine* over that representation general and
automatic (PLN) and that effort is the abandoned half. zuihitsu's redesign should feel entitled to
borrow the representation (strength+confidence riding on a fact) without committing to building a
general symbolic inference engine on top of it — that ambition is the one that didn't survive
contact with reality in the most directly comparable prior system.

---

## 5. MemGPT / Letta

**Source of truth.** No independent store beyond what any LLM agent already has — the innovation is
entirely architectural/orchestration, not a new database. MemGPT (2023 paper, "Towards LLMs as
Operating Systems") frames the LLM's context window as **main context** (= RAM: in-context, directly
attended-to prompt tokens) and everything else as **external context** (= disk: outside the fixed
window, not directly attended to) [https://arxiv.org/pdf/2310.08560, corroborated by
https://medium.com/@ahmadareeb3026/llm-as-operating-systems-agent-memory-b70c1213a5f7]. The
paging metaphor is deliberate and explicit: hierarchical memory with paging between a fast, small
tier and a slow, large tier [https://arxiv.org/pdf/2310.08560].

**The self-editing mechanism.** The load-bearing convergent idea: **the LLM itself decides**, via
ordinary function/tool calls, what moves between main and external context — there is no separate
framework-level heuristic doing the promotion/eviction. The model calls memory-edit tools to
archive, retrieve, and rewrite its own persistent state
[https://vectorize.io/articles/hindsight-vs-letta]. This inverts NELL/Cyc's split (where a
non-agent process or a human ontologist decides what's promoted): here the same neural component
that writes facts also decides what stays salient. zuihitsu's design already has a version of this
(the agent authors entries and links directly via the Luau API) but currently keeps a *separate,
deterministic* visibility/materialisation layer outside the model's control — MemGPT's family
converged on giving the model direct read/write control over its own "what's in front of me" state,
which is a different and more model-authority-heavy point on the same design axis.

**What the Letta family converged on since.** Letta (the commercial continuation of MemGPT)
has added shared memory across concurrent sessions, skill learning from experience, and
programmatic tool calling [secondary, https://vectorize.io/articles/hindsight-vs-letta] — i.e., the
family's own evolution has been toward *more* agent-authored structure (skills, shared memory)
rather than toward more externally-imposed structure, suggesting the OS metaphor's natural
extension is "give the agent more kinds of self-editable state," not "constrain what it can edit."

**Published lessons / critiques.** The clearest published critique available in this search pass is
comparative rather than a post-mortem: MemGPT/Letta established the **Deep Memory Retrieval (DMR)**
benchmark as its own primary evaluation metric, but this benchmark's limitations became apparent as
LLM capabilities advanced past what DMR could discriminate — competitors (Zep) reported outperforming
Letta on DMR (94.8% vs 93.4%) shortly after [secondary,
https://blog.getzep.com/state-of-the-art-agent-memory/], and DMR itself is now treated across the
field as a benchmark whose ceiling was reached and whose discriminative power decayed as models
improved [secondary, same source]. This is a mild but real lesson: a benchmark authored by the
system's own team, to demonstrate that system's own novel mechanism, has a short shelf life once
the mechanism becomes commonplace.

**One insight zuihitsu should take.** The OS metaphor's real contribution isn't the tiering (paging
in/out of a context window is close to what any RAG/retrieval system already does) — it's putting
the eviction/promotion *decision* under the same agent that writes facts, using the same tool-call
surface it already has. zuihitsu's redesign already keeps the neural writer separate from a
deterministic materialisation layer (descriptions, briefs) by design, for exactly the reason
failure class #11 (unverified neural writer) flags — MemGPT's convergence is a useful counter-data-point
showing that the alternative (model directly curates its own front-of-mind state) is a live,
validated design, not a strawman, but it comes with the same "who checks the model's edits" problem
zuihitsu is trying to avoid by keeping curation checked/deterministic. Worth naming explicitly as
the road not taken and why.

---

## 6. Graphiti / Zep

**Source of truth.** A "Context Graph" per subject: a temporal knowledge graph of entities and
relationships, each fact a (subject, relation, object) triplet extracted by an LLM from
conversation episodes [https://www.getzep.com/platform/graphiti/,
https://neo4j.com/blog/developer/graphiti-knowledge-graph-memory/]. This is architecturally the
closest of all seven systems to zuihitsu's own links model — bare relation triples over named
entities, extracted from prose by a model.

**Fact shape and the bi-temporal model.** Every edge carries an explicit **validity interval**
(`t_valid`, `t_invalid`), separate from when the edge was *recorded* — i.e., Graphiti's bitemporal
model, like zuihitsu's asserted/occurred split, distinguishes "when we learned this" from "when
this was/is true." A dedicated LLM extraction step (`tref`) runs specifically to pull the temporal
context out of the episode text and populate these fields
[https://help.getzep.com/graphiti/getting-started/overview].

**Edge invalidation instead of deletion.** When a new edge is added, the system uses an LLM to
compare it against semantically related existing edges to detect contradiction; on a temporally
overlapping contradiction, the *existing* edge is invalidated by setting its `t_invalid` to the new
edge's `t_valid` — the old edge is never deleted, only closed off
[https://blog.getzep.com/beyond-static-knowledge-graphs/]. This is functionally identical in shape
to zuihitsu's supersession/retraction tombstones (failure class discussion in the grounding brief),
independently reinvented for an LLM-native temporal graph, and is good corroborating evidence that
"never delete, only close the interval and let history stand" is a convergent, not idiosyncratic,
answer to this problem.

**Published eval claims — read with real skepticism, as instructed.** Zep's own paper claims
"substantial results with accuracy improvements of up to 18.5% while simultaneously reducing
response latency by 90%" versus baseline on LongMemEval [secondary self-reported,
https://blog.getzep.com/content/files/2025/01/ZEP__USING_KNOWLEDGE_GRAPHS_TO_POWER_LLM_AGENT_MEMORY_2025011700.pdf],
and separately reported outperforming Letta/MemGPT on the DMR benchmark (94.8% vs 93.4%)
[https://blog.getzep.com/state-of-the-art-agent-memory/]. **This should be discounted heavily**: the
same benchmark family (LoCoMo) is at the centre of a public, unresolved cross-vendor dispute — Mem0
reported Zep at 58.44% accuracy after attempting to replicate Zep's claimed number; Zep countered
that Mem0 had misconfigured their system (wrong role assignment for both speakers, timestamps
appended to message text instead of populating the `created_at` field, and sequential rather than
parallel search runs inflating Zep's reported latency), and re-ran to claim 75.14%
[secondary, cross-corroborated by
https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/ and
independent summary at
https://essays.bloo-mind.ai/posts/2026-05-20-mem-eval/]. An independent audit of the underlying
LoCoMo benchmark itself found "6.4% of the answer key is wrong, the LLM judge accepts 63% of
intentionally wrong answers, and 56% of per-category system comparisons are statistically
indistinguishable from noise," including hallucinated facts in the answer key (e.g. a "Ferrari 488
GTB" that exists only in an internal annotator field no memory system actually ingests) and 24
questions with the wrong speaker attributed [secondary,
https://essays.bloo-mind.ai/posts/2026-05-20-mem-eval/]. The essay's conclusion — "the best
strategy for a high LoCoMo score is unfortunately context-stuffing and generating long,
topically-adjacent answers that fool the judge" — should itself be read as one commentator's
characterisation, not an established fact, but it is corroborated by the structural point (a judge
that accepts 63% of wrong answers cannot discriminate memory quality from verbosity). **Net
assessment: treat every specific percentage figure attributed to any of these systems (Zep, Mem0,
Letta) as unreliable pending independent, third-party-run benchmarking; the qualitative
architectural claims (bitemporal edges, invalidation-not-deletion, LLM extraction) are far better
supported than the quantitative claims.**

**What keeps it alive.** Zep is a funded commercial product (Zep Inc/getzep.com) with Graphiti as
its open-source core component, both still actively maintained and marketed as of the 2026 sources
found in this search pass. No post-mortem exists because it has not died — noted for completeness,
not as a claim about its ultimate fate.

**One insight zuihitsu should take.** Zep/Graphiti is the strongest existing validation that
zuihitsu's own bitemporal-edges + never-delete-only-invalidate design is not an idiosyncratic
choice but a convergent one, arrived at independently by a team solving the same "LLM extracts
structure from prose, structure needs to age" problem. The benchmark chaos is a governance lesson,
not a technical one: **do not let any redesigned zuihitsu component's success be measured by a
self-reported, judge-scored composite benchmark with no independently agreed protocol** — this
directly reinforces CONTRIBUTING.md's existing eval discipline (structural checks for exact
properties, judge only for genuine language judgement, gating bars with a defined `min_rate`) and
should be treated as an outside data point *for* keeping that discipline, not a reason to imitate
Zep's PR.

---

## 7. Mem0

**Source of truth.** No persistent graph is required in the base variant — a vector store of
short natural-language memory statements, plus (in the `Mem0ᵍ` graph variant) a Neo4j-backed
directed labelled graph of typed entity nodes and relation-triplet edges
[https://memo.d.foundation/breakdown/mem0, https://www.emergentmind.com/topics/mem0-system].

**Fact shape and the pipeline.** Two-phase: an **extraction** phase uses the conversation summary
plus recent messages to have an LLM identify candidate salient facts; an **update** phase compares
each candidate against existing memories by vector similarity and has an LLM decide, per candidate,
one of four operations: **ADD** (no semantically equivalent memory exists), **UPDATE** (augment an
existing memory with complementary information), **DELETE** (the new information contradicts and
supersedes an existing memory), or **NOOP** (no change warranted)
[https://arxiv.org/html/2504.19413v1, corroborated by
https://medium.com/@EleventhHourEnthusiast/mem0-building-production-ready-ai-agents-with-scalable-long-term-memory-9c534cd39264].
This four-way decision, made by an LLM per-candidate at write time, is structurally the same
"model is the sole writer of structure" pattern the grounding brief flags as failure class #11 —
Mem0 does not describe an independent check on these LLM decisions in the sources reviewed.

**Graph variant.** `Mem0ᵍ` extends the same pipeline with entity/relation extraction into a graph:
nodes carry types, embeddings, and metadata; edges are typed relation triplets
[https://memo.d.foundation/breakdown/mem0]. The published evaluation reports "significant latency
and token cost savings while balancing trade-offs in multi-hop accuracy" versus the non-graph
variant [https://arxiv.org/html/2504.19413v1] — i.e., Mem0's own paper is candid that the graph
variant trades some multi-hop reasoning accuracy for efficiency, which is a useful, non-self-serving
admission.

**Forgetting/decay story.** DELETE is explicit and LLM-decided, contradiction-triggered — closer to
zuihitsu's current retraction than to Soar/ACT-R's activation decay. No decay-by-disuse mechanism is
described.

**Published eval claims and critiques.** Same LoCoMo dispute as covered under Zep above applies
symmetrically: "the Mem0 paper's claims of SOTA performance appear to be based on a flawed benchmark
(LoCoMo) and a demonstrably incorrect implementation of a competitor system (Zep)" [secondary,
direct quote surfaced in search results, likely originating from Zep's rebuttal blog post
https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/ — read as an
interested party's characterisation, not a neutral audit, though it is consistent with the
independent LoCoMo audit findings cited under Zep above]. Apply the same discount here as for Zep's
numbers: qualitative pipeline design (four-way ADD/UPDATE/DELETE/NOOP classification) is credible
and independently corroborated across multiple sources; specific percentage superiority claims over
named competitors are not.

**What keeps it alive.** Mem0 is an actively maintained, well-starred open-source project with a
hosted commercial offering; no indication of decline in the sources reviewed.

**One insight zuihitsu should take.** The ADD/UPDATE/DELETE/NOOP four-way classification is a clean,
minimal vocabulary for "what does a new utterance do to existing structured memory" — arguably a
tighter primitive set than zuihitsu's current append/supersede/retract/attest, worth checking
zuihitsu's operations against for completeness (does zuihitsu have a clean NOOP path today, or does
every entry always get appended even when it's a pure repeat, relying entirely on the maintenance
pass to dedup after the fact at cosine 0.95? If so, Mem0's convergence suggests the ADD-vs-NOOP
distinction is cheap enough to make at write time rather than deferring entirely to an offline
maintenance pass — an argument for moving *some* of zuihitsu's dedup judgement earlier, in-line,
rather than only as a batch pass). The graph variant's candid multi-hop-accuracy-vs-efficiency
trade-off is a useful reminder that adding graph structure over an LLM-extraction pipeline is not
free even when it succeeds — worth keeping in mind for the eventual redesign's own graph richening.

---

## Synthesis: what independent systems keep reinventing

1. **A confidence/strength value riding alongside a fact, separate from the fact's mere presence.**
   NELL's candidate/promoted threshold, OpenCog's (strength, confidence) TruthValue pair, and
   Mem0's ADD/UPDATE-vs-NOOP decision are three structurally distinct systems converging on "a fact
   needs a number (or a coarse decision) attached that says how much to trust or act on it,
   separate from storing it at all." zuihitsu's issue #94 credence-model direction is squarely on
   this convergent path, not a novel idea — which is reassuring but also means the design should
   look hard at *why* each of these representations differ (a single scalar vs. a pair vs. a
   four-way categorical) before picking one.

2. **Never delete, only close the interval.** Graphiti/Zep's edge invalidation (`t_invalid` set,
   edge kept) and zuihitsu's own existing supersession/retraction tombstones are the same design,
   arrived at independently for the same reason: an LLM-extraction-driven system needs to preserve
   the ability to explain *why* it once believed something different, and hard deletion destroys
   that audit trail. Soar/ACT-R's activation-decay is a third variant of the same principle at a
   different layer (never delete, just make less likely to surface).

3. **Context/scope as the mechanism for handling "the same referent means something different
   depending on who's looking / what frame you're in."** Cyc's microtheories are the clearest
   ancestor of zuihitsu's visibility postures, though built for logical-consistency scoping rather
   than confidentiality. The pattern that recurs: bind a scope tag to the assertion, keep global
   consistency-checking local to the scope, allow the scope to have inheritance/hierarchy.

4. **Coupling independent evidence sources against each other rather than trusting one channel.**
   NELL's coupled learning (a classifier's output must cohere with a relation-extractor's output on
   the same entity) is the clearest instance; it is the automated-systems' answer to the same
   "the neural writer is unverified" problem (#11) that zuihitsu names as unsolved. None of the
   other six systems reviewed here implement anything as explicit as NELL's coupling constraint —
   this may be an underused idea worth zuihitsu picking up rather than reinventing, e.g., requiring
   two independent signals (embedding geometry *and* model judgement, already used together in
   zuihitsu's maintenance passes; or two independent tellers) before a structural fact gets
   "promoted" to a status the rest of the system treats as settled.

5. **Learning as compiling deliberation, triggered by the deliberation event itself.** Soar's
   chunking-on-impasse is the sharpest version of this; it has no analogue among the LLM-native
   systems reviewed (MemGPT, Graphiti, Mem0), all of which treat memory writes as *facts recorded*,
   never as *reasoning paths compressed for reuse*. zuihitsu's issue #58 (procedural memories —
   saved Luau functions, "nowhere principled to live") is exactly this gap: Soar/ACT-R's 40-year
   answer is that procedural learning should be triggered automatically by the deliberation event
   (an impasse, a costly multi-step derivation) rather than the agent having to decide to save a
   procedure — a design zuihitsu could study directly for #58.

## Graveyard lessons

- **Cyc**: curation cost that does not fall with scale kills a project slowly, over decades, by
  simple economic exhaustion — even when the project never technically "fails" outright (Cycorp
  still operates). The lesson is about marginal cost per fact, not about correctness or ambition.
- **OpenCog/PLN**: the storage-and-bookkeeping layer (hypergraph, truth values, attention economy)
  is the part that survived 15+ years of an AGI research project; the reasoning layer meant to be
  the actual point of the project was abandoned and restarted from scratch under a new name. This
  is a caution against over-investing zuihitsu's redesign effort in a general inference/reasoning
  layer over the richer ontology, versus investing in the representation itself being sound and
  letting reasoning stay as focused, task-specific model calls (which is what zuihitsu already does
  and should probably keep doing).
- **Benchmark self-report culture (Zep/Mem0/Letta/LoCoMo)**: an unresolved, mutually-contradicting
  public dispute between two funded companies over the same benchmark, plus an independent audit
  finding the benchmark's answer key and judge are both unreliable, is recent (2025–2026) direct
  evidence that this entire subfield's published numbers should not inform design decisions without
  independent replication. This is not really a "why did it die" lesson (none of these systems have
  died) — it's a "how not to evaluate your own redesign" lesson, and it reinforces zuihitsu's
  existing eval-harness discipline rather than adding anything new to it.
- **NELL**: went quiet after 2018 with no clear public post-mortem found in this pass — flagged
  **[unverified]**, and left to the sibling lane covering NELL's drift lessons in depth.

---

## Implications for zuihitsu

Mapping onto the recorded failure classes and open issues from the grounding brief:

- **#8 (belief has no credence model)**: three independent convergent precedents exist for a
  credence value riding on a fact — NELL's confidence-gated promotion, OpenCog's (strength,
  confidence) pair, Mem0's ADD/UPDATE/DELETE/NOOP categorical decision. The redesign should pick a
  representation shape deliberately (a scalar strength, a strength+confidence pair, or a categorical
  decision) rather than treating "add a credence model" as underspecified — these are three
  concretely different, tested shapes to choose among.

- **#11 (neural writer unverified) / "hygiene thresholds are embedder geometry" (#9)**: NELL's
  coupled-learning constraint is the one convergent design directly aimed at "don't trust a single
  extraction channel" — worth exploring as a structural check (require agreement between two
  independent signals before a structural fact is promoted to settled status) rather than relying
  solely on embedding-geometry thresholds, which failure class #9 already flags as fragile to
  embedder changes.

- **#58 (procedural memories, nowhere principled to live)**: Soar's chunking-on-impasse is a direct,
  well-tested design pattern — trigger procedural-memory creation automatically from a costly or
  repeated deliberation event, not from the agent's own decision to "save a function." Worth a
  dedicated look at how Soar's substate/impasse machinery could map onto zuihitsu's turn loop
  (a turn that required unusually deep multi-step Luau reasoning is the zuihitsu analogue of an
  impasse).

- **Relations are bare edges (#3), no validity interval**: Graphiti/Zep's bitemporal edges with
  `t_valid`/`t_invalid` and invalidate-don't-delete are the single most directly transplantable
  precedent in this survey — it's effectively the same shape as zuihitsu's existing
  asserted/occurred temporal model, just applied to links instead of only to entries. This is strong
  corroborating evidence (not novel information) that the "relations need validity intervals"
  direction the failure-class list already identifies is a convergent, sound one.

- **Microtheories vs. visibility postures**: Cyc's microtheory hierarchy is a validated precedent for
  scope-as-context, but the survey found no precedent system that used context/microtheory-like
  scoping specifically for *confidentiality* rather than *logical consistency* — this seam
  (confidentiality-as-scope) appears to be closer to zuihitsu's own genuine contribution than
  something to import wholesale from Cyc. Worth flagging to the synthesis pass as an area where
  zuihitsu's existing design may be ahead of, not behind, the prior art.

- **Eval discipline**: the Zep/Mem0/LoCoMo benchmark chaos is an argument *for* zuihitsu's existing
  CONTRIBUTING.md eval discipline (structural checks for exact properties, judge reserved for
  genuine language judgement, gating bars with explicit `min_rate`), not a reason to change it —
  flagged here so the synthesis pass has an external data point for why that discipline matters.

- **Cost-of-curation discipline (Cyc)**: any redesign mechanism that adds a mandatory
  human-in-the-loop or per-fact model-judgement-call step for structural promotion should be
  checked against Cyc's economics — the failure mode is not "the mechanism is wrong" but "the
  mechanism's marginal cost per fact doesn't fall as the log grows," which is a scaling property
  worth stating explicitly as a design constraint (echoing the grounding brief's "scale
  deliberately unbounded," #44) for any newly proposed curation/promotion step.
