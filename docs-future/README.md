# docs-future

**Nothing in this tree describes zuihitsu as it is built.** It describes a proposed successor to the data model, written in the present tense as though it already existed. Every claim about how memories are stored, how facts are shaped, how identity resolves, or how the agent writes to the store is a claim about a system that does not exist and may never be built in this form.

For the system that actually runs, read [`../docs/`](../docs/), which is maintained as as-built documentation and is the only place to look for current behaviour.

## Why the present tense

A design written as "the system would carry a validity interval" reads as speculation and never quite commits. Written as "a relation carries a validity interval", it commits, and the places where it cannot commit become visible as gaps rather than hiding behind a conditional. The normative voice is a drafting discipline, not a claim of implementation status.

The cost of that discipline is that a reader who lands here from a search result could reasonably mistake this for reality, which is why the tree is quarantined out of `docs/` and named for what it is.

## What is here

The design. [`overview.md`](overview.md) is the way in; the rest can be read in any order.

| | |
|---|---|
| [`statements.md`](statements.md) | the keystone: one object carrying a claim, its frame, its gloss, its provenance, its validity, its credence, and its audience |
| [`events-and-roles.md`](events-and-roles.md) | a happening as one node with role-edges, not a copy per participant |
| [`two-traces.md`](two-traces.md) | structure and narrative as complementary rather than ranked |
| [`relations.md`](relations.md) | attribute-bearing, interval-scoped, domain-constrained, and repairable |
| [`identity.md`](identity.md) | revocable graded merges, held below a substrate wall |
| [`belief.md`](belief.md) | credence from counting evidence, never from the model's mouth |
| [`time.md`](time.md) | occurrence, task, and trigger kept apart, so a description cannot fire |
| [`memory-typology.md`](memory-typology.md) | four kinds with four lifecycles, and the self in a slot outside all of them |
| [`privacy-and-provenance.md`](privacy-and-provenance.md) | transmission conditions as data, zero residue, retraction against erasure |
| [`the-seam.md`](the-seam.md) | the model proposes, the critics dispose, and drift is watched from outside |
| [`query-surface.md`](query-surface.md) | structural questions, structural answers, and a deliberately small API |
| [`write-surface.md`](write-surface.md) | two verbs, structuring inside the transaction, and the parse handed back for correction |
| [`lineage.md`](lineage.md) | what each ancestor contributed, and what was deliberately left behind |

The supporting material:

- **[`coverage.md`](coverage.md)** grades the design against the surveyed failures of the current ontology, class by class, and maps it onto the open issues. The grading is deliberately uneven; several classes are answered in design but not yet validated.
- **[`confidence.md`](confidence.md)** registers every load-bearing claim with its evidence and its status. The normative voice above is what makes this file necessary: a design that states everything flatly needs somewhere to record that some of it rests on one paper, one benchmark, or one lane's judgement.
- **[`evolution.md`](evolution.md)** is the prospective build order for getting from the current codebase to this one, with each stage's gating evidence and main risk.
- **[`research/`](research/)** holds the design research this is built on, preserved at the date it was conducted rather than rewritten to match the design it produced. Most of it is literature; one piece is not. The [corpus study](research/2026-08-03/modelling-study.md) tested this model against the running instance's own recorded data before these chapters were written, and changed them: it falsified one design assumption and found two expressiveness gaps that none of the literature lanes anticipated.

## Scope

The design targets a new instance at genesis. **The existing instance is not migrated**, so no upcasting path from the current event log is owed, and several choices here take advantage of that freedom. `evolution.md` is a codebase path, not a data path.

The failure survey that grounds the whole exercise is [`../docs/ontology-failures/2026-07-23.md`](../docs/ontology-failures/2026-07-23.md), which stays in `docs/` because it records real failures of the real system.
