# docs-future

**Nothing in this tree describes zuihitsu as it is built.** It describes a proposed successor to the data model, written in the present tense as though it already existed. Every claim about how memories are stored, how facts are shaped, how identity resolves, or how the agent writes to the store is a claim about a system that does not exist and may never be built in this form.

For the system that actually runs, read [`../docs/`](../docs/), which is maintained as as-built documentation and is the only place to look for current behaviour.

## Why the present tense

A design written as "the system would carry a validity interval" reads as speculation and never quite commits. Written as "a relation carries a validity interval", it commits, and the places where it cannot commit become visible as gaps rather than hiding behind a conditional. The normative voice is a drafting discipline, not a claim of implementation status.

The cost of that discipline is that a reader who lands here from a search result could reasonably mistake this for reality, which is why the tree is quarantined out of `docs/` and named for what it is.

## What is here

The design:

- **[`statements.md`](statements.md)** is the keystone: the single reified object that carries a claim, its provenance, its temporal placement, its credence, and its audience.
- The remaining chapters cover events and roles, the two traces, relations, identity, belief, time, the memory typology, privacy and provenance, the write seam, and the query surface.

The supporting material:

- **[`coverage.md`](coverage.md)** grades the design against the surveyed failures of the current ontology, class by class, and maps it onto the open issues. The grading is deliberately uneven; several classes are answered in design but not yet validated.
- **[`confidence.md`](confidence.md)** registers every load-bearing claim with its evidence and its status. The normative voice above is what makes this file necessary: a design that states everything flatly needs somewhere to record that some of it rests on one paper, one benchmark, or one lane's judgement.
- **[`evolution.md`](evolution.md)** is the prospective build order for getting from the current codebase to this one, with each stage's gating evidence and main risk.
- **[`research/`](research/)** holds the design research this is built on, preserved at the date it was conducted rather than rewritten to match the design it produced.

## Scope

The design targets a new instance at genesis. **The existing instance is not migrated**, so no upcasting path from the current event log is owed, and several choices here take advantage of that freedom. `evolution.md` is a codebase path, not a data path.

The failure survey that grounds the whole exercise is [`../docs/ontology-failures/2026-07-23.md`](../docs/ontology-failures/2026-07-23.md), which stays in `docs/` because it records real failures of the real system.
