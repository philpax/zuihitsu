# Lineage

Almost nothing in this design is new. What follows is where each piece came from, and what was deliberately left behind.

The through-line: **take representations and disciplines, refuse reasoners.** The most comparable prior system ran for fifteen years, and what survived was its storage and bookkeeping layer, while its general inference engine was abandoned and restarted more than once. Reasoning here is focused, task-specific model calls plus sound critics, and there is no general symbolic reasoner anywhere in the design.

## What each ancestor contributed

| Source | Taken | Left behind |
|---|---|---|
| **OWL** | Cardinality constraints, inverse properties, symmetry and reflexivity, binary arity, and the deprecation vocabulary that became [deprecate-and-alias](relations.md) | The reasoner, the open-world assumption, and the absence of a unique-name assumption, which together make an exact count mean something we do not want it to mean. Above all `owl:sameAs`, whose unrestricted transitive substitution is the exact amplifier that makes one bad merge corrupt a whole identity class |
| **Wikidata** | The reified [statement](statements.md) carrying qualifiers and references, the hard-won discipline of keeping qualifiers one level deep, and the quantity datatype with its uncertainty bounds, taken wholesale for [counts and measures](statements.md) | Ranks as a credence proxy. A three-value filter is not a belief model, and we count evidence instead |
| **RDF n-ary patterns** | Reification as [Events with role-edges](events-and-roles.md), which is the standard answer to binary-only relations | The triple store itself |
| **TypeDB** | Role-typed relations with enforced filler types | The vendor performance and scaling claims, which were never relied on. Only the architecture is |
| **PROV** | The derivation record shape: entity, activity, agent, and a plan slot for the template | Its descriptiveness. PROV records that an assumption was used, not that a conclusion's truth is contingent on it, which is the gap the assumption stamp fills |
| **Nanopublications** | Retraction and supersession as first-class signed events against a content-addressed prior, and who-may-retract as an authority question | Leaving the subject-differs-from-author case open. For a personal agent that case is the common one, so it gets [an authority lattice](privacy-and-provenance.md) |
| **Named graphs and CKR** | Quotation against assertion, per-context truth, and justifiable exceptions | CKR's knowledge-propagation engine, which is precisely the opposite of compartmentalisation |
| **Contextual integrity** | The [transmission principle](privacy-and-provenance.md) as the governing condition on a flow, promoted from an enum to data | Nothing substantial; this one transfers cleanly |
| **iCalendar** | The occurrence, task, and trigger split that makes [a description unable to fire](time.md) | The serialisation format, and raw recurrence strings, which can encode traps that constructors cannot |
| **SQL:2011 and bitemporal databases** | Valid time against transaction time, supersession by window-closing | Nothing substantial |
| **Zep and Graphiti** | The decision-time axis, which is what makes delayed ingestion coherent, and keeping the source episode beside the extracted structure | LLM entity resolution as the identity mechanism, which degrades as the graph grows |
| **Allen's interval algebra** | A tractable subset with a composition table, for [qualitative anchoring](time.md) | The full thirteen relations, whose general reasoning is intractable |
| **ACT-R and Soar** | Base-level activation for retrieval and decay, episodic memory as architectural and automatic, and the data/metadata wall that keeps [identity machinery](identity.md) out of the agent's reasoning | The cognitive architecture itself, and any claim stronger than analogy that human recall results transfer to agent salience |
| **Subjective logic** | The representation separating belief strength from evidence quantity, and the trust-discounting operator | The fusion operators and the mapping to evidence counts, both of which have named critics. [`belief.md`](belief.md) relies on dependence detection instead |
| **NELL** | Mutual exclusion as the drift brake, coupling independent signals before promotion, and the finding that the ontology rather than the extractor did the work | Continuous curation. Autonomy here is exception-triggered attention |
| **DeepProbLog and Scallop** | The doctrine that the neural half never decides a structural question, and that provenance is computed with the conclusion rather than attached afterwards | The machinery. There is no differentiable logic layer and no provenance semiring implementation |
| **Cyc** | Microtheories as the ancestor of the [frame](statements.md): context-relative truth, letting a character be a fourth-grader in one context and a cartoon in another | The context logic, the open lattice, and the curation economics recorded in the graveyard lessons below |
| **Conceptual graphs** | The confirmation that plural referents with counts are a solved representation problem | The collective-against-distributive distinction, which CGs mark natively and we decline to pay for |
| **Voyager** | Procedural memory as executable code indexed by a natural-language description | The specific domain |
| **MemGPT and Letta** | The tiered memory framing | Model-authored state as the primary mechanism. It is a live and validated design, and we choose checked deterministic curation instead, at a real cost in agent autonomy over its own salience |
| **Dual-trace encoding** | Structure and narrative as [two traces](two-traces.md), and the finding that elaboration pays specifically on sequencing, aggregation, and change tracking | The claim that it is free, which is an artifact of their harness, and the prompt-borne disclaimer guarding against treating a reconstruction as evidence |

## Two inheritances worth spelling out

### Cardinality moved from the class to the individual

OWL expresses cardinality as a **restriction on a class**: every person has exactly two biological parents, a committee has at least three members. That is a schema-level statement, checked against every instance.

What a personal agent mostly needs is the other thing: a **fact about one individual**, at one time. Someone has five ice creams. A corpus holds nine authors. A session produced three of something.

Both are kept, and they are different mechanisms. Class-level cardinality lives on a [relation definition](relations.md) and is a critic: it rejects a write that would give something two operators when the relation is declared functional. Instance-level counting lives in the object slot as a typed value, and is an ordinary claim with a validity interval, so a count that changes is a window closing rather than a contradiction.

The OWL construct closest to the second is qualified cardinality, which counts fillers of a property restricted to a class. OWL 2 does allow asserting one of a named individual, so "Vertas has exactly five ice creams" is expressible there. Two things stop us borrowing the mechanism rather than the idea. OWL has no unique-name assumption, so five mentions are not five things until inequality is stated explicitly, and it has no temporal scoping, so a count that changes needs the whole assertion reified as a fluent. Our count is a value with a validity interval, which gets both for free.

The value's shape is Wikidata's: an amount, an optional unit, and optional bounds. The bounds matter more than they look, because a corpus is full of approximations stated as though they were exact.

### Binary arity, and where n-ary content goes

Relations are binary, exactly as OWL properties are. Nothing in this design has a three-place edge.

N-ary content goes through Event nodes with role-edges, which is the reification answer the RDF community converged on and which TypeDB implements natively. The gain over a wider edge is that each role becomes independently addressable, so each carries its own provenance, credence, and audience. A three-place edge could not hold three tellers.

## The graveyard lessons

Three, from systems that did not survive or did not scale, and each of them constrains a decision here.

**Curation cost must fall per fact as the store grows.** The most ambitious hand-curated knowledge base did not fail technically; its curation cost never fell, and that was enough. Every mechanism in this design is checked against that constraint, which is why raw experience is retained cheaply and structured selectively rather than uniformly.

**The bookkeeping layer outlives the reasoning layer.** Build the store well and keep reasoning focused.

**Do not let a component's success be measured by a self-reported composite benchmark.** The one memory benchmark surveyed was found by independent audit to have a materially wrong answer key and a judge that accepted most intentionally wrong answers, while two funded companies disputed their scores on it. Success here is measured by structural oracles against our own event log.
