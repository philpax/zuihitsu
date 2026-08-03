# Research

The evidence the design in [`../`](../) is built on, preserved at the dates it was gathered rather than rewritten to match the design it produced. Later evidence arrives as a new dated snapshot beside the existing ones; nothing here is edited in place once its verification pass has run.

Two rules govern this tree. Evidence is not rewritten for readability, because an audit trail edited to read well is an audit trail quietly laundered; formatting and outright errors are fixed, arguments are not. And every claim keeps the confidence its lane gave it, including the ones its lane flagged as uncertain, so that [`../confidence.md`](../confidence.md) can register them honestly rather than inheriting a false uniformity.

## Snapshots

### [2026-07-24](2026-07-24/) — the design-research programme

Seven parallel research lanes, a synthesised report, and two adversarial-verification passes. This is the bulk of the evidence and the source of every structural commitment in the design.

- [`report.md`](2026-07-24/report.md) is the programme's primary deliverable: the argued case for the redesign, with each claim cited into a lane. The design chapters in `../` supersede it as the normative description, but the report holds the reasoning and remains the place to look for *why* rather than *what*.
- [`draft-issue.md`](2026-07-24/draft-issue.md) is the same proposal condensed as a GitHub issue, unfiled.
- [`lanes/`](2026-07-24/lanes/) holds the seven evidence lanes.
- [`verification/`](2026-07-24/verification/) holds the two adversarial passes over the report's claims.
- [`00-grounding.md`](2026-07-24/00-grounding.md) and [`01-synthesis-plan.md`](2026-07-24/01-synthesis-plan.md) are the programme's working notes: the brief the lanes ran under, and the plan the synthesis followed.

The seven lanes:

| Lane | Scope |
|---|---|
| [`fact-shape`](2026-07-24/lanes/fact-shape.md) | How to represent a fact: reified statements, n-ary relations, event semantics, schema-guided extraction, and relation-vocabulary evolution |
| [`provenance-privacy`](2026-07-24/lanes/provenance-privacy.md) | PROV, nanopublications, named graphs, contextual integrity, and the retraction-versus-erasure split |
| [`identity-belief`](2026-07-24/lanes/identity-belief.md) | Identity as graded rather than hard equivalence, collective entity resolution, truth maintenance, and credence over attestations |
| [`time-memory`](2026-07-24/lanes/time-memory.md) | Bitemporal modelling, Allen's interval algebra, iCalendar's component split, and the cognitive memory typology |
| [`welding`](2026-07-24/lanes/welding.md) | The neural/symbolic seam: NELL's drift record, LLM-modulo, structured elicitation, and why no surveyed system verifies its neural writes |
| [`survey-giants`](2026-07-24/lanes/survey-giants.md) | Convergent evolution across seven historical and production knowledge systems, including the graveyard lessons |
| [`survey-issue7`](2026-07-24/lanes/survey-issue7.md) | Ten current-generation persistent-memory agent projects, surveyed from source on 2026-07-23 |

**Verification status.** Both passes have run. Of the report's load-bearing claims, 29 were confirmed against fetched primary sources, 5 were corrected, 0 were unsupported, and 1 remained unreachable. Every future-dated arXiv identifier was fetched directly rather than judged from memory, and all resolved to real papers supporting the claims as reported. The corrections are folded into the report body; what stayed flagged is carried into [`../confidence.md`](../confidence.md).

**One caveat on reading it.** The snapshot's own README records that the codebase moved after the lanes were written, so its descriptions of the "current system" are stale in places. The canonical-identity work in particular shipped a mechanical layer the report presents as unbuilt. The structural proposal is unaffected, but hold the report's present-tense descriptions of today's behaviour against [`../../docs/`](../../docs/) rather than trusting them.

### [2026-08-03](2026-08-03/) — dual-trace encoding, and the corpus study

Added after the design was taken up: one lane on one paper, and one falsification exercise against live data.

- [`dual-trace.md`](2026-08-03/dual-trace.md) covers Stern and Nadel's controlled experiment pairing each stored fact with an elaborated narrative trace, and the four amendments it forces on the design.
- [`modelling-study.md`](2026-08-03/modelling-study.md) tests the Statement model against the running instance's 198 content entries, asking whether it can express what the system actually recorded.

**Verification status of the dual-trace lane: none.** It postdates the adversarial passes and is not covered by them. It rests on one primary source with no corroborating study, a single benchmark, an LLM judge, twenty questions per category, and a missing ablation that happens to be the one that decides the cost for us. It is strong enough to change the design's shape and nowhere near strong enough to settle it, which is why the amendments it drives are paired with an experiment in [`../evolution.md`](../evolution.md) rather than adopted outright.

**Status of the modelling study: primary evidence.** Unlike every other lane, it rests on direct observation of this system's own data rather than on literature, so it needs no external corroboration. It was deliberately run *before* the design chapters were written, and it changed them: it found that the two hypotheses the design most feared were both unfounded, and that two genuine expressiveness gaps none of the seven lanes anticipated were real. Its verdict is that the model is sufficient to proceed with two additions and one correction. The correction, that a gloss belongs to an utterance rather than to a Statement, contradicted a design assumption and is the clearest case in the tree of evidence arriving before commitment rather than after.

## The failure survey

Both snapshots are grounded in [`../../docs/ontology-failures/2026-07-23.md`](../../docs/ontology-failures/2026-07-23.md), which stays in `docs/` because it records observed failures of the system that actually runs. It is the adjustment input for the whole exercise, and [`../coverage.md`](../coverage.md) grades the design against it class by class.
