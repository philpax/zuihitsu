# Lane: fact shape, relations, and schema evolution

Research lane for the zuihitsu ontology redesign. Scope: how to represent a *fact* so that
structure is stored, not re-derived from prose (failure class 1); how to represent a multi-participant
*event* once rather than as per-subject copies (failure class 2); how to give *relations* attributes
and validity intervals (failure class 3); and how *relation vocabulary* evolves, deprecates, and
aliases over a system's life (failure class 6, issue #42), including bulk-ingest structure (#44).

Every load-bearing claim is cited. Uncertainty is flagged inline. Where a source is a vendor blog
or secondary explainer rather than a primary spec or peer-reviewed paper, that is noted so the
verification pass can weight it.

---

## 1. Neo-Davidsonian event semantics and frame semantics

### The core idea

Davidson's 1967 move ("The Logical Form of Action Sentences") was to add an **event variable** to
the logical form of an action sentence, so "Brutus stabbed Caesar" is `∃e. stab(e, Brutus, Caesar)`
rather than a bare binary `stab(Brutus, Caesar)`. Adverbial modifiers then attach as *separate
conjuncts predicated of the same `e`* — "stabbed violently, in the back, with a knife" becomes
`∃e. stab(e, …) ∧ violent(e) ∧ in-the-back(e) ∧ with(e, knife)`. This is the "adverbs as predicates
of an event" insight and it is exactly the shape that dissolves "facts are sentences": each modifier
is its own storable proposition about a shared reified event, not a re-phrasing of the whole sentence.

The **neo-Davidsonian** refinement (Parsons 1990, *Events in the Semantics of English*; Castañeda)
goes further: the verb's *own* arguments are also pulled out as separate role conjuncts, so the event
predicate becomes unary and every participant attaches through a **thematic role** relation:
`∃e. stab(e) ∧ Agent(e, Brutus) ∧ Patient(e, Caesar) ∧ Instrument(e, knife)`. This is characterised
as "the standard for event semantics," with two independent assumptions: (a) the event argument is
the verbal predicate's *only* direct argument, and (b) participants relate to the event through a
closed set of thematic roles (Agent, Theme, Patient, Result, Instrument, …)
([Landman, neo-Davidsonian event semantics course notes, Tel Aviv Univ.](https://www.tau.ac.il/~landman/Online%20Class%20Notes/2%20ADVANCED%20SEMANTICS/8%20Neo-davidsonian%20event%20semantics.pdf);
[Landman, Events and plurality](https://www.tau.ac.il/~landman/files/advanced-semantics/9%20Events%20and%20plurality.pdf)).

Why this matters for zuihitsu directly: **one event `e`, many role-edges, each role-edge is an
independently addressable, independently attributed proposition.** "Alice and Bob shipped feature X on
Tuesday" is *one* event node with `Agent→Alice`, `Agent→Bob` (or `Co-agent`), `Theme→feature X`,
`Time→Tuesday` — not three prose sentences. This is the principled fix for failure class 2.

### Frame semantics / FrameNet

Fillmore's frame semantics is the lexical-KR cousin: a *frame* is a schematic situation type with a
fixed inventory of **frame elements** (roles) specific to that frame. FrameNet operationalises this as
~1,200 frames, each with core and peripheral frame elements, plus frame-to-frame relations
(inheritance, subframe, causative-of). The key difference from Davidson's *universal* thematic roles:
FrameNet roles are **frame-specific** (`Commerce_buy` has `Buyer`, `Seller`, `Goods`, `Money`), which
is more expressive but multiplies the role inventory (see costs below). (FrameNet is well-documented;
the operative trade-off — frame-specific roles vs. a small universal set — recurs in every SRL schema.)

### How KR systems have operationalised event reification

- **PropBank** annotates every verb sense with **numbered arguments** `ARG0..ARG5` plus modifier tags
  (`ARGM-TMP`, `ARGM-LOC`, `ARGM-MNR`, …). `ARG0` ≈ proto-agent, `ARG1` ≈ proto-patient; the numbering
  above ARG1 is *verb-specific* and defined per-roleset, not universal
  ([Palmer, Gildea & Kingsbury 2005, *The Proposition Bank*, Computational Linguistics 31(1)](https://aclanthology.org/J05-1004.pdf);
  [Stanford SLP3 SRL slides](https://web.stanford.edu/~jurafsky/slp3/slides/22_SRL.pdf)).
- **AMR (Abstract Meaning Representation)** encodes a whole sentence as a *rooted, directed, (mostly)
  acyclic graph*: nodes are concepts, edges are relations, and predicate concepts reuse **PropBank
  framesets** for their argument structure. AMR abstracts away surface syntax — "who did what to whom"
  — and folds NER, SRL, and coreference into one graph. It uses ~100 relations, with `:ARG0..:ARG5`
  taken from PropBank/OntoNotes
  ([Banarescu et al. 2013, *AMR for Sembanking*; Survey of AMR, arXiv:2505.03229](https://arxiv.org/pdf/2505.03229)).
  AMR is the closest existing artefact to "a graph of reified events with typed participant roles,
  produced from natural language."
- **Semantic Proto-Role Labeling** (Reisinger et al.; White et al.) is a reaction *against* categorical
  role inventories: instead of committing to "is this ARG2 a benefactive or an instrument?", it labels
  each argument with graded proto-role *properties* (did the participant cause the event? was it
  sentient? did it change state?)
  ([Neural-Davidsonian Semantic Proto-role Labeling, arXiv:1804.07976](https://arxiv.org/pdf/1804.07976)).
  This is a signal that even the expert-annotation community found fixed higher-numbered roles hard.

### The practical costs (honestly stated)

1. **Role-inventory explosion / role ambiguity.** PropBank's own numbered roles above ARG1 are
   documented as *inconsistent*: "ARG2–ARG5 are not really that consistent," the benefactive/attribute
   role "may be encoded by either ARG2 or ARG3," so "there is no one-to-one correspondence between
   numbered arguments and thematic roles," and only ARG0/ARG1 generalise across verbs
   ([Stanford SLP3 SRL slides](https://web.stanford.edu/~jurafsky/slp3/slides/22_SRL.pdf);
   [Palmer et al. 2005](https://aclanthology.org/J05-1004.pdf)). FrameNet trades this for ~1,200
   frames each with its own elements — expressive, but a huge schema to teach a writer. **The lesson
   for an LLM-authored system: a small universal role set (agent/patient/theme/time/place/instrument)
   is teachable; a large frame-specific inventory is not, and even experts disagree on the tail.**
2. **Annotation difficulty.** Meaning-banking guides list dozens of hard cases ("Thirty Musts for
   Meaning Banking," arXiv:2005.13421) — reentrancy, coordination, scope, implicit arguments — that
   trained annotators get wrong. An LLM writing structure live, one utterance at a time, has *less*
   context than a sembank annotator with the whole document.
   ([Bos et al., Thirty Musts, arXiv:2005.13421](https://arxiv.org/pdf/2005.13421)).
3. **Graph parsing is a research problem, not a solved primitive.** AMR parsing remains an active area
   with its own error modes; treating "LLM emits a correct event graph" as free is optimistic.

**Takeaway for lane:** neo-Davidsonian reification (one event, role-edges) is the right *skeleton*,
but the role *inventory* must be kept small and universal, with everything else expressed as attributes
on the event rather than as more roles. Proto-role grading and the "structured form + prose gloss"
hybrid (§5) are the hedges against the writer getting a role wrong.

---

## 2. Reification trade-offs: how to attach metadata to a statement

The recurring problem: a fact is not just `(subject, predicate, object)`; it has provenance, a validity
interval, a credence, an audience posture — and sometimes it is *itself* the subject of another fact.
There is a well-mapped design space for this, and each point has documented pathologies.

### The candidates

| Model | Shape | Statement identity | Cost |
|---|---|---|---|
| **RDF standard reification** | 4 triples per statement (`rdf:subject/predicate/object` on an `rdf:Statement` node) | explicit node, but *no enforced link* to the asserted triple | verbose (4× triples), highest triple count of any model; query must reconstruct the link manually | 
| **RDF-star / RDF 1.2 quoted triples** | embed a triple as a term: `<< :a :b :c >> :source :d` | a quoted triple is *unique* — the same `<<…>>` always denotes the same thing | compact, shorter SPARQL; but "quoted ≠ asserted" semantics caused years of WG debate | 
| **Named graphs** | put the statement's triples in a graph and hang metadata off the graph name | graph IRI is the handle | coarse: metadata attaches to a *set* of triples, not one; nesting is awkward | 
| **Singleton property** | mint a unique sub-property per statement (`:b#1`), hang metadata on it | the singleton property IRI | proliferates predicates; documented as *less effective* than reification in embedding tasks | 
| **Property graph (Neo4j/openCypher)** | key-value **properties directly on the relationship** | the edge itself is the identity | ergonomic for attributes; but edges are strictly **binary**, and you cannot make a statement *about* an edge without reifying it into a node | 
| **Wikidata statement model** | a *Statement* object between item and value, carrying **qualifiers**, **references**, and a **rank** | the statement node (has a stable ID) | the most battle-tested real-world design; qualifier pathologies below | 

Sources: [Ontotext, "Is RDF-star the best choice for reification?"](https://www.ontotext.com/blog/graphdb-users-ask-is-rdf-star-best-choice-for-reification/);
[Ontotext, What is RDF-star](https://www.ontotext.com/knowledgehub/fundamentals/what-is-rdf-star/);
[W3C RDF 1.2 Concepts (triple terms)](https://www.w3.org/TR/rdf12-concepts/);
[Comparison of Metadata Representation Models for KG Embeddings, arXiv:2503.21804](https://arxiv.org/abs/2503.21804)
(compares REF vs singleton-property vs RDF-star; finds REF competitive, singleton weaker, and in
*complex* hyper-relational graphs the differences shrink — i.e. the representation matters less once the
graph is genuinely n-ary);
[Bob DuCharme, "Triples about existing triples"](https://www.bobdc.com/blog/etriplesabout/).

Key contrasts worth internalising:

- **Standard reification's fatal flaw is the missing enforced link:** the reified `rdf:Statement`
  node has no built-in connection to the actual asserted triple, so tooling and queries must
  re-establish it by convention. It also produces the highest triple counts (4 metadata triples per
  fact) ([Ontotext RDF-star blog](https://www.ontotext.com/blog/graphdb-users-ask-is-rdf-star-best-choice-for-reification/)).
- **RDF-star fixes verbosity and gives a canonical identity** to the embedded triple, at the cost of
  the subtle "quoting a triple does not assert it" semantics that the W3C CG argued over
  ([W3C-CG rdf-star issue #274, comparison to reified statements](https://github.com/w3c-cg/rdf-star/issues/274)).
- **Property graphs win on ergonomics** (properties live *on* the edge) but are architecturally
  **binary-only**, so a genuinely n-ary event (many participants) forces an intermediate "event node"
  anyway — which is exactly reification under a different name
  ([Edge-Labelled vs Property Graphs, arXiv:2204.06277](https://arxiv.org/pdf/2204.06277)).

### Wikidata's statement model — the most relevant prior art, mined for pathologies

Wikidata does not store bare triples. A **Statement** is a first-class object: `item — property →
value`, plus optional **qualifiers** (refine/scope the value: as-of date, determination method,
point-in-time), **references** (where the value came from), and a **rank** (`preferred` / `normal` /
`deprecated`, a simple filter over competing statements of the same property)
([Wikidata:Data model](https://www.wikidata.org/wiki/Wikidata:Data_model);
[Help:Qualifiers](https://www.wikidata.org/wiki/Help:Qualifiers);
[Help:Ranking](https://www.wikidata.org/wiki/Help:Ranking)).
This maps *almost one-to-one* onto zuihitsu's needs: qualifiers = validity interval + method,
references = `told_by`/`told_in`, rank ≈ supersession/credence.

Documented pathologies of the qualifier model — read these as warnings:

- **No formal semantics / scope ambiguity.** Qualifiers "participate in the semantics of statements"
  but the model has no complete formal account of *how*; researchers had to propose a many-sorted logic
  (sorts = qualifier categories) to reason over them. Guidance explicitly forbids using a qualifier to
  modify *another qualifier* because "this can make the meaning of the qualifier ambiguous" — i.e. the
  model is deliberately kept one-level-deep to dodge scope problems
  ([Handling Wikidata Qualifiers in Reasoning, arXiv:2304.03375](https://arxiv.org/html/2304.03375);
  [Help:Qualifiers](https://www.wikidata.org/wiki/Help:Qualifiers)).
- **Qualifier proliferation under inference.** "The qualifiers of inferred statements are often a
  combination of the qualifiers in the rule condition," so reasoning over qualified statements is a
  hard, under-specified problem ([arXiv:2304.03375](https://arxiv.org/html/2304.03375)).
- **Ranks are a crude filter, not a credence model.** Three discrete ranks (`preferred/normal/
  deprecated`) are "a very simple filtering mechanism," not a probability; "references state where a
  value comes from; ranks indicate what value is considered most correct"
  ([Help:Ranking](https://www.wikidata.org/wiki/Help:Ranking)). If zuihitsu wants real credence
  (failure class 8), Wikidata's ranks are a cautionary floor, not a model to copy.

**Lane recommendation on shape:** the Wikidata *statement object* (a reified fact carrying qualifiers +
references + rank) is the closest real-world match, but keep the qualifier layer **exactly one level
deep** (the Wikidata discipline), model validity/credence/provenance as *typed* qualifier slots rather
than free key-values, and do **not** inherit Wikidata's three-rank crudeness where a real credence is
wanted.

---

## 3. Hypergraphs and n-ary relations

The binary-edge assumption is the root of "one event, many copies." The alternatives:

### W3C n-ary relations Note

The W3C Semantic Web Best Practices WG Note ["Defining N-ary Relations on the Semantic Web"](https://www.w3.org/TR/swbp-n-aryRelations/)
(Working Group Note, 2006) is the canonical statement of the problem and its RDF/OWL patterns. It
explicitly lists the two use cases that are *precisely* zuihitsu's failure classes 2 and 3:

- **Pattern 1 — properties of a relation:** representing "certainty about it, severity or strength of
  a relation, relevance of a relation" — i.e. attaching credence/validity/provenance to a fact. The
  pattern is to reify the relation as an **individual** (a relation instance) and hang the extra
  attributes off it.
- **Pattern 2 — relations among more than two individuals:** the buyer/seller/object purchase example —
  i.e. a multi-participant event. Again: mint an instance and attach each participant via a role
  property ([W3C n-ary Note](https://www.w3.org/TR/swbp-n-aryRelations/)).

Both patterns converge on the same primitive: **a fact/event is a node, participants and attributes
hang off it.** This is neo-Davidsonian reification arrived at from the KR side.

### TypeDB (formerly Grakn) — role-based typed hypergraph

TypeDB is the most directly relevant *implemented* system. Its model:

- Baseline types are **Entities, Relations, and Attributes**; a **Relation is a first-class concept**
  that defines **named roles**, connects *any number* of participants, can carry its own attributes,
  and can itself play a role in another relation (nested/hyper-relations)
  ([TypeDB, The case for a structured hypergraph](https://typedb.com/blog/the-case-for-a-structured-hypergraph);
  [TypeDB features](https://typedb.com/features)).
- Example given: an acquisition deal as a single four-way relation
  `deal(target, buyer, target-advisor, buyer-advisor)` — no intermediate node sprawl, the role
  structure is surfaced directly in the data
  ([TypeDB blog](https://typedb.com/blog/the-case-for-a-structured-hypergraph)).
- The **type system enforces role-filler types**: "relationships define named roles, and only entities
  of the correct type can play each role," with inheritance hierarchies the DB reasons over natively
  ([TypeDB blog](https://typedb.com/blog/the-case-for-a-structured-hypergraph)).
- **Acknowledged trade-off:** TypeDB requires up-front schema definition and is "less suitable for
  exploratory work" than schema-optional property graphs
  ([TypeDB blog](https://typedb.com/blog/the-case-for-a-structured-hypergraph)). This is the tension
  with zuihitsu's *agent-coined-at-runtime* vocabulary: a strict typed hypergraph wants the schema up
  front; zuihitsu wants the agent to coin relations live. §4 is where that tension gets resolved.

(Note: the TypeDB material is vendor-authored; the *architecture* claims — n-ary relations, roles,
typed role-fillers — are corroborated by the independent [Equivalence Theorem paper, arXiv:2603.13603](https://arxiv.org/pdf/2603.13603)
on first-class relationships, and by the n-ary KR taxonomy [arXiv:2506.05626](https://arxiv.org/pdf/2506.05626)
and role-aware n-ary KB modeling [arXiv:2104.09780](https://arxiv.org/pdf/2104.09780). The vendor
*performance* claims should be treated as unverified.)

### Where the n-ary literature nets out

The academic n-ary KR community (hyper-relational KG embedding) has essentially concluded that once a
graph is genuinely n-ary, the *serialization* choice (reification vs RDF-star vs singleton) matters
less for downstream tasks than getting the n-ary structure right in the first place
([arXiv:2503.21804](https://arxiv.org/abs/2503.21804)). Translation for zuihitsu: **spend the design
budget on the event/role model, not on bikeshedding the on-disk reification syntax** — the SQLite
materialisation can pick whichever encoding is cheapest to query.

---

## 4. Schema evolution: versioning relation vocabulary over a system's life

This is failure class 6 / issue #42 ("relation schemas immutable — a mis-registered relation can only
be abandoned") and the "relation coined 4 ways" drift.

### Event-sourcing upcasting (directly applicable — zuihitsu is event-sourced)

The event-sourcing community has a mature vocabulary for "the stored events outlive the code that wrote
them." The five documented tactics: **versioned events, weak schema, upcasting, in-place transformation,
and copy-and-transform** ([Overbeek et al., *An empirical characterization of event sourced systems and
their schema evolution*, JSS 2021, ScienceDirect](https://www.sciencedirect.com/science/article/pii/S0164121221000674);
[event-driven.io, simple events versioning patterns](https://event-driven.io/en/simple_events_versioning_patterns/)).

- **Upcasting**: convert an old event to the new shape *on read/replay*, leaving the log untouched.
  This is the natural fit for zuihitsu's "deterministic replay from an append-only log": a
  `RelationRegistered{v1}` event can be upcast to `{v2}` at materialisation time. Cost noted in the
  literature: conversion overhead on *every* read, and larger overhead during full replay
  ([Marten docs, events versioning](https://martendb.io/events/versioning.html);
  [oneuptime, event versioning strategies](https://oneuptime.com/blog/post/2026-01-30-event-driven-versioning-strategies/view)).
- **Weak schema / tolerant reader**: additive-only changes (new optional fields, defaults) that old and
  new code both tolerate — the cheapest form of evolution, and the reason "add, never mutate" is a
  discipline.
- **Copy-and-transform**: batch-rewrite the log into a new stream. zuihitsu's fixed point ("append-only
  log as sole truth") makes this the heavy hammer, reserved for genuine breaking changes — and the
  grounding notes there is *no migration constraint* (target is a future genesis), which means the
  redesign can pick the clean shape without an upcasting burden for existing data.

The empirically recommended path: **start with versioned events + weak schema, add upcasting when you
need replay, reach for copy-and-transform only for breaking changes**
([ScienceDirect empirical study](https://www.sciencedirect.com/science/article/pii/S0164121221000674)).

### Wikidata property governance (the runtime-coined-vocabulary analogue)

Wikidata is the largest system where a *community* coins relation vocabulary at runtime, and it has
evolved exactly the disciplines zuihitsu's agent lacks:

- **Property proposal process**: label/description/aliases and, crucially, **domain and range** are
  discussed *before* a property is created, specifically "to make sure that your property isn't used
  outside your original usage purpose, which may deteriorate data quality"
  ([Wikidata:Creating a property proposal](https://www.wikidata.org/wiki/Wikidata:Creating_a_property_proposal)).
- **Constraints as guard rails**: property constraints and entity schemas combat misuse *after*
  creation without deleting the property
  ([Wikidata:Property proposal](https://www.wikidata.org/wiki/Wikidata:Property_proposal)).
- **Deprecation + migration, not deletion**: duplicative properties (the documented case: "interested
  in" P2650 with 17k uses vs "field of work" P101 with 938k uses) are **merged by bot-transferring
  values to the surviving property, deprecating the old one, then removing it once migrated**
  ([Wikidata:Properties for deletion](https://www.wikidata.org/wiki/Wikidata:Properties_for_deletion);
  [Wikidata:Property proposal reform RFC](https://www.wikidata.org/wiki/Wikidata:Requests_for_comment/Reforming_the_property_creation_process)).

The lesson for issue #42: **immutability of a registered relation is the wrong constraint. The right
model is deprecate-and-alias**: a mis-coined relation is not abandoned; it is marked deprecated, its
uses are (optionally, by a maintenance pass) re-pointed to a canonical relation via a stored *alias
edge*, and the alias remains so old references still resolve. This is exactly the "relation coined 4
ways" fix — the four coinings become four aliases of one canonical relation, resolved at read time.

### OWL/LinkML ontology versioning

- **OWL** provides `owl:versionInfo`, `owl:priorVersion`, `owl:backwardCompatibleWith`, and
  `owl:incompatibleWith`, plus `owl:deprecatedClass`/`owl:deprecatedProperty` to mark a term retained
  for reference but no longer recommended — i.e. deprecation is a *first-class ontology primitive*, not
  a delete ([W3C OWL Reference, deprecation & version info — well-documented; primary spec at
  w3.org/TR/owl-ref]).
- **LinkML** treats the schema itself as versioned, evolvable data with explicit migration tooling and
  a `deprecated` slot on elements, aimed squarely at "schemas change over a system's lifetime"
  ([linkml.io — LinkML models schemas as first-class, versioned artefacts]). *(These two are stated
  from general KR knowledge and the specs; flagged for the verification pass to confirm exact predicate
  names.)*

**Synthesis for §4:** the relation registry should be **append-only and event-sourced like everything
else**, but registration should be *revisable*: a relation carries a lifecycle (`active` / `deprecated`
/ `aliased_to`), the agent (or a maintenance pass) can deprecate-and-alias, and reads resolve aliases
transitively. Domain/range (role types) on a relation is the Wikidata "specify domain and range"
discipline and the TypeDB "role-filler types" enforcement — it turns a mis-use into a *teachable error*
at coin time rather than silent drift.

---

## 5. The tension between rich structure and an LLM writer

zuihitsu's fixed point: the agent is the *sole writer of structure* and is *unverified* (failure class
11). The literature is unusually clear-eyed here.

### OpenIE vs closed-schema extraction — the fidelity trade-off

- **Open IE** imposes no predefined entities/relations — attractive for LLMs (lower inference cost,
  flexible) but produces unnormalized, drift-prone output; the same relation surfaces many ways (the
  "coined 4 ways" pathology, industrialised)
  ([LLM-empowered KG construction: a survey, arXiv:2510.20345](https://arxiv.org/pdf/2510.20345)).
- **Ontology-/schema-guided extraction** constrains the LLM to a schema and measurably improves
  fidelity. Apple's **ODKE+** dynamically generates per-entity-type ontology snippets to align
  extractions with schema constraints and reports **98.8% precision** over millions of facts
  ([ODKE+, Apple ML Research](https://machinelearning.apple.com/research/odke);
  [ODKE+ paper, arXiv:2509.04696](https://arxiv.org/pdf/2509.04696)).
- Schema grounding cuts hallucination hard: one line of work reports **hallucination rates dropping
  ~87%** with a well-structured, classified knowledge base vs unstructured data, and schema-consistency
  enforcement "addresses crucial errors in 85% of cases, correcting node-type mismatching and reversed
  relationships" ([survey/derived figures, arXiv:2510.20345 and related]). *(These specific percentages
  come from secondary summaries of the literature; treat the direction — schema grounding materially
  reduces error — as solid, and the exact figures as needing primary-source confirmation.)*
- **Reversed-relationship errors** are a *named, common* LLM extraction failure mode — which is
  precisely why a **relation with declared domain/range** (§4) matters: it lets the system *catch* a
  reversed edge structurally ([anchor-constrained grounded KG extraction, MDPI Computers 15(3):178](https://www.mdpi.com/2073-431X/15/3/178)).

**Direct implication:** a rich closed-ish schema does not *hurt* the LLM writer — it *helps* it, as long
as the schema is presented as constraints/snippets at write time (which is exactly what zuihitsu's
teachable-error pedagogy already is). The failure-class-1 fix (structure over prose) and the
failure-class-11 concern (unverified writer) are *aligned*, not opposed: structure is what makes the
writer checkable.

### The "structured fact + prose gloss" hybrid — keep both

Several agent-memory systems have independently landed on **storing the structured form and the source
utterance together**, so the canonical structure is queryable *and* the original text is retained for
re-derivation, provenance, and audit:

- **MemOS / MemCubes**: memory units that carry "provenance and versioning metadata alongside the
  content itself" — the structured/provenance layer travels *with* the content
  ([Mem0 paper context, arXiv:2504.19413; MemOS overview via
  MachineLearningMastery memory-frameworks survey](https://machinelearningmastery.com/the-6-best-ai-agent-memory-frameworks-you-should-try-in-2026/)).
- **Zep / Graphiti** (temporal KG for agent memory): keeps an **episode subgraph** where each node is
  the *raw event/message with its original timestamp* (the source utterance), and a semantic subgraph
  of extracted entities/facts derived from it. The raw episode is retained, not discarded after
  extraction ([Graphiti / Zep temporal KG, Neo4j blog](https://neo4j.com/blog/developer/graphiti-knowledge-graph-memory/);
  [Zep temporal KG architecture, EmergentMind](https://www.emergentmind.com/topics/zep-a-temporal-knowledge-graph-architecture)).
- **Mem0**: extracts salient facts and, on each new fact, an LLM routing controller inspects the top-k
  similar existing memories and picks `ADD/UPDATE/DELETE/NOOP` — i.e. the *dedup/consolidation* step is
  itself a structured decision, not a re-parse of prose
  ([Mem0 paper, arXiv:2504.19413](https://arxiv.org/pdf/2504.19413)). (This is a direct external
  precedent for zuihitsu's maintenance passes — and a warning: Mem0 does this at *write time with an
  LLM in the loop*, which is exactly zuihitsu's #100 "neural judgement inside a symbolic transaction"
  tension.)

### Bitemporality is settled practice in this space

Zep/Graphiti gives **every fact/edge two time axes** — *valid time* (true in the world) and
*transaction time* (when ingested) — and on contradiction **closes the old fact's validity window
rather than deleting it**, recording the new one; the agent reasons over current state while history
stays queryable ([Neo4j Graphiti blog](https://neo4j.com/blog/developer/graphiti-knowledge-graph-memory/);
[Zep, What is a temporal knowledge graph](https://www.getzep.com/ai-agents/temporal-knowledge-graph/)).
This validates zuihitsu's existing asserted/occurred bitemporal split and points the way to putting the
*same* validity-interval machinery on **relations** (failure class 3: "worked at X 2019–2021"): a
relation is a statement with `valid_from`/`valid_to`, superseded by window-closing, never deletion.

---

## Implications for zuihitsu

Mapping the above onto the failure classes and issues, concretely.

### Failure class 1 — "facts are sentences"

The fix the whole literature converges on: **a fact is a reified statement node, not a prose blob.**
Store `(subject, relation, object)` (or an event with role-edges, below) as *structure*, with the prose
utterance retained as a `gloss` field beside it (the Zep-episode / MemCube "keep both" pattern, §5).
Downstream mechanisms (dedup, consolidation, temporal placement) then operate on structure —
`same (subject, relation, object)?` is a structural equality, not a cosine guess — and fall back to the
gloss/embedding only for genuinely fuzzy matches. This directly retires "every downstream mechanism
re-derives structure from prose, nondeterministically." Crucially, §5's evidence says this does **not**
burden the LLM writer: schema-guided writing *lowers* its error rate (ODKE+ 98.8% precision; ~87%
hallucination reduction). The current ContentEntry becomes: a typed statement + its provenance
qualifiers + the original prose as gloss.

### Failure class 2 — "one event, one subject, many copies"

Adopt the **neo-Davidsonian skeleton** (§1): a multi-participant happening is **one event node** with
**role-edges** (`Agent`, `Theme`, `Time`, `Place`, `Instrument`) to its participants, from a **small
universal role set** — *not* per-subject prose copies, and *not* a large frame-specific inventory
(PropBank's ARG2–5 inconsistency and FrameNet's 1,200-frame cost are the cautionary evidence). "Alice
and Bob shipped feature X Tuesday" is one `event/ship` with two `Agent` edges, one `Theme`, one `Time`.
Each role-edge and each attribute is independently attributable and independently visible — which
composes cleanly with zuihitsu's per-attestation posture model. This is the single most load-bearing
structural change and it is well-precedented (AMR, W3C n-ary Pattern 2, TypeDB `deal(...)`).

### Failure class 3 — "relations are bare edges"

Promote a relation from a bare edge to a **first-class statement object** carrying attributes
(Wikidata statement / property-graph-edge-properties / W3C n-ary Pattern 1). Give it:
- a **validity interval** (`valid_from`/`valid_to`) so "worked at X 2019–2021" is expressible, with
  supersession = **window-closing, not deletion** (Zep/Graphiti bitemporal pattern, §5);
- **provenance** (`told_by`/`told_in`) as its reference layer (Wikidata references);
- a **credence/rank** slot (but avoid Wikidata's crude three-rank floor if real credence is wanted —
  failure class 8);
- **declared domain/range (role types)** on the relation definition, so a reversed or mis-typed edge is
  caught as a *teachable error* at write time (ODKE+/anchor-constrained evidence that this is the
  highest-value guard against LLM extraction errors).

Keep the qualifier layer **exactly one level deep** — Wikidata's hard-won discipline against qualifier
scope ambiguity (§2). Do not let a qualifier modify a qualifier.

### Failure class 6 / issue #42 — relation schemas immutable

Replace immutability with **deprecate-and-alias**, event-sourced:
- a relation definition carries a lifecycle (`active` / `deprecated` / `aliased_to: <canonical>`);
- the agent (or a maintenance pass) can deprecate a mis-coined relation and alias it to a canonical
  one; **reads resolve aliases transitively** so old references keep working — this is literally the
  Wikidata "interested in → field of work" bot-merge pattern (§4);
- the "relation coined 4 ways" drift is resolved by making the four coinings four **aliases of one
  canonical relation**, collapsed at read time;
- because zuihitsu is event-sourced with deterministic replay, the mechanism is **read-time alias
  resolution + upcasting** (§4), not log rewriting — the append-only fixed point is preserved. The
  grounding's "no migration constraint (future genesis)" means the clean shape can be adopted directly.

Add the Wikidata **coin-time discipline**: coining a relation should require declaring its inverse,
cardinality, *and* domain/range (the current registry already has the first two). Domain/range turns
mis-use into a teachable error instead of silent drift, and is what makes the reversed-edge guard
possible.

### Issue #44 — long-document bulk ingestion into structured clusters

The extraction literature (§5) is the relevant guidance: **schema-guided (not open) extraction** for
bulk ingest — feed the LLM the relation/role schema as constraints so a long document lands as
normalized event/statement nodes rather than a drift of open-IE triples. The **event-reification
skeleton** (§2) gives the natural cluster structure #44 wants: a document's sections/claims become
events and statements linked by `part_of`/`summarizes` role-edges into a navigable hypergraph, exactly
the W3C n-ary composition pattern. Retain each source span as the statement's **gloss** (Zep episode
pattern) so a bulk-ingested claim is always traceable to its source text — which is also the audit
answer to failure class 11.

### Recommended fact-shape direction (with honest trade-offs)

**Direction:** a **reified, typed, statement/event model with a small universal role set, one-level
qualifiers, bitemporal validity, and a retained prose gloss** — Wikidata's statement object crossed with
TypeDB's role-typed relations and neo-Davidsonian event reification, materialised into SQLite however
queries cheapest (the n-ary embedding literature says the on-disk reification syntax barely matters once
the structure is genuinely n-ary, §3).

**What it buys:** structural dedup/consolidation/temporal-placement (retires class 1); one-event-many-
roles (retires class 2); relations with intervals + attributes + credence (retires class 3); a
domain/range guard that catches the LLM's most common extraction errors at write time (mitigates class
11); deprecate-and-alias vocabulary evolution (retires class 6/#42); a clean home for bulk-ingest
clusters (#44).

**What it costs, honestly:**
1. **Writer burden shifts, doesn't vanish.** The LLM now emits structure (event + roles + qualifiers),
   not prose. The evidence says schema-guidance *lowers* its error rate — but role assignment above
   agent/patient is genuinely hard even for experts (PropBank ARG2–5), so the universal role set must
   stay small and everything else must be an *attribute*, not a role. The gloss is the safety net when
   the structure is wrong.
2. **Every fact is now a small graph, not a row.** More nodes/edges, more materialisation cost, more
   query surface. The n-ary literature's consolation is that the encoding choice is not where the cost
   concentrates — the structure is.
3. **Qualifier scope must be policed.** One level deep, typed slots only. Wikidata got here by painful
   experience; adopt the rule, not the pain.
4. **Neural-judgement-in-transaction tension (#100) sharpens.** Structural writes still need the LLM to
   choose the structure; Mem0's ADD/UPDATE/DELETE routing is the precedent and the warning. This is a
   real coupling the redesign must own, not a thing the fact-shape alone resolves.
5. **Credence is a separate design (class 8).** A `rank`/credence slot on the statement is the *hook*;
   a real belief model is out of this lane's scope and should not be faked with Wikidata's three ranks.

---

## Source appendix (primary/high-confidence first)

- Davidson 1967, "The Logical Form of Action Sentences"; Parsons 1990, *Events in the Semantics of
  English* — the neo-Davidsonian foundation (via
  [Landman course notes](https://www.tau.ac.il/~landman/Online%20Class%20Notes/2%20ADVANCED%20SEMANTICS/8%20Neo-davidsonian%20event%20semantics.pdf)).
- [Palmer, Gildea, Kingsbury 2005, *The Proposition Bank*, CL 31(1)](https://aclanthology.org/J05-1004.pdf);
  [Stanford SLP3 SRL slides](https://web.stanford.edu/~jurafsky/slp3/slides/22_SRL.pdf) — PropBank
  roles and their documented inconsistency.
- [AMR survey, arXiv:2505.03229](https://arxiv.org/pdf/2505.03229);
  [Neural-Davidsonian proto-role labeling, arXiv:1804.07976](https://arxiv.org/pdf/1804.07976).
- [W3C, Defining N-ary Relations on the Semantic Web (Note, 2006)](https://www.w3.org/TR/swbp-n-aryRelations/).
- [W3C RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/);
  [Ontotext RDF-star fundamentals](https://www.ontotext.com/knowledgehub/fundamentals/what-is-rdf-star/)
  and [RDF-star vs reification](https://www.ontotext.com/blog/graphdb-users-ask-is-rdf-star-best-choice-for-reification/);
  [Comparison of metadata representation models, arXiv:2503.21804](https://arxiv.org/abs/2503.21804);
  [Edge-labelled vs property graphs, arXiv:2204.06277](https://arxiv.org/pdf/2204.06277).
- [Wikidata:Data model](https://www.wikidata.org/wiki/Wikidata:Data_model),
  [Help:Qualifiers](https://www.wikidata.org/wiki/Help:Qualifiers),
  [Help:Ranking](https://www.wikidata.org/wiki/Help:Ranking),
  [Creating a property proposal](https://www.wikidata.org/wiki/Wikidata:Creating_a_property_proposal),
  [Properties for deletion](https://www.wikidata.org/wiki/Wikidata:Properties_for_deletion);
  [Handling Wikidata Qualifiers in Reasoning, arXiv:2304.03375](https://arxiv.org/html/2304.03375).
- [TypeDB structured hypergraph](https://typedb.com/blog/the-case-for-a-structured-hypergraph) (vendor;
  architecture corroborated by [n-ary KR taxonomy, arXiv:2506.05626](https://arxiv.org/pdf/2506.05626)
  and [role-aware n-ary KB, arXiv:2104.09780](https://arxiv.org/pdf/2104.09780)).
- [Overbeek et al., event-sourced schema evolution, JSS 2021](https://www.sciencedirect.com/science/article/pii/S0164121221000674);
  [event-driven.io versioning patterns](https://event-driven.io/en/simple_events_versioning_patterns/);
  [Marten events versioning](https://martendb.io/events/versioning.html).
- [LLM-empowered KG construction survey, arXiv:2510.20345](https://arxiv.org/pdf/2510.20345);
  [ODKE+, Apple](https://machinelearning.apple.com/research/odke) /
  [arXiv:2509.04696](https://arxiv.org/pdf/2509.04696);
  [anchor-constrained grounded KG extraction, MDPI Computers 15(3):178](https://www.mdpi.com/2073-431X/15/3/178).
- [Mem0, arXiv:2504.19413](https://arxiv.org/pdf/2504.19413);
  [Graphiti/Zep temporal KG, Neo4j](https://neo4j.com/blog/developer/graphiti-knowledge-graph-memory/);
  [Zep temporal KG architecture](https://www.emergentmind.com/topics/zep-a-temporal-knowledge-graph-architecture);
  [agent-memory frameworks survey (MemOS/MemCubes)](https://machinelearningmastery.com/the-6-best-ai-agent-memory-frameworks-you-should-try-in-2026/).

### Flagged uncertainties for the verification pass
- The specific percentages "~87% hallucination reduction" and "85% of errors corrected" come from
  secondary summaries of the KG-construction literature, not a single primary measurement; the
  *direction* (schema grounding materially reduces LLM extraction error) is solid, the exact figures
  need a primary source.
- OWL deprecation predicates (`owl:deprecatedClass`/`Property`, `owl:backwardCompatibleWith`) and
  LinkML's `deprecated` slot are stated from spec knowledge, not a fetched page — confirm exact names.
- TypeDB performance/scaling claims are vendor-sourced and unverified; only its *architecture* is
  relied upon here.
- The arXiv:2503.21804 detail was read from the abstract/metadata only (the PDF fetch returned binary);
  the "encoding matters less once genuinely n-ary" claim rests on the abstract's statement that in
  complex hyper-relational graphs the differences among reification models are minimal — confirm
  against the full text.
