# Overview

zuihitsu is a neurosymbolic personal-agent harness. The symbolic half is an append-only event log, the sole source of truth, replayed deterministically and materialised into a knowledge graph. The neural half is a language model living in conversation, acting through a sandboxed Luau API. One instance is one agent is one log. The agent meets people across platforms, remembers what each said, and keeps confidences between them.

This tree describes the data model that agent thinks in.

## The shape

Everything rests on one object. A [**Statement**](statements.md) carries a typed claim, the referential layer that claim is made in, a pointer to the utterance it came from, its provenance, its validity interval, its credence, its audience condition, and, if it was derived, what it was derived from. A fact, an endorsement of a fact, and the provenance of a fact are the same object seen from different angles.

Around that keystone:

- A happening with more than one participant is one [**Event**](events-and-roles.md) with role-edges, not a copy per participant.
- A [**relation**](relations.md) is itself attribute-bearing and interval-scoped, with declared domain and range, and a vocabulary that evolves by deprecation and aliasing rather than being frozen at first use.
- Structure and narrative are [**two traces**](two-traces.md) of the same content, indexed separately and retrieved together, serving precision and recall respectively.
- [**Identity**](identity.md) is a revocable, graded assumption held below a substrate wall, so the agent sees one resolved handle per person and never reasons about the machinery.
- [**Belief**](belief.md) is credence derived from counting evidence, never from a model stating a number.
- [**Time**](time.md) separates what happened, what the agent must do, and what fires, so a dated description can never wake anything up.
- [**Memory**](memory-typology.md) is four kinds with four lifecycles, not one container with one decay rule, and the agent's own charter is a slot outside all four rather than an entry inside one.
- [**Privacy**](privacy-and-provenance.md) is a transmission condition carried as data on each Statement, evaluated deterministically and fail-closed.
- The [**write seam**](the-seam.md) is typed: the model proposes structure and a bank of critics disposes, so a write can be rejected rather than merely recorded.
- The [**query surface**](query-surface.md) stays handle-shaped and small, however rich the substrate gets, and the [**write surface**](write-surface.md) is two verbs that hand the parse back for correction.

[`lineage.md`](lineage.md) records where each of these came from. Almost none of it is new; the design's contribution is the selection and the refusals.

## What holds it together

Four commitments constrain every chapter, and a proposal that violates one is wrong rather than interesting.

**The log is the only truth, and replay is deterministic.** Every model and embedder call that *affects stored state* happens at record time and is written to the log, so the fold calls nothing and no derived state exists that a fold cannot reproduce. A live read may still call the embedder to vectorise a query, as `memory.search` does today; that is a transient ranking input, never a stored derivation.

**Privacy is at least as strong as it is today.** Per-fact audience conditions, zero residue from an uncleared confidence, and a subject guard that holds by construction rather than by convention.

**The agent-facing surface stays simple.** The agent addresses people and memories by handle and asks structured questions. It never speaks ontology-language, never picks between sibling handles, and never manages the machinery. Richness lives in the substrate; the surface exposes a small vocabulary and teaches through errors at the point of failure.

**Scale is unbounded.** Ingesting a long document is a normal operation rather than an exception, and any mechanism whose cost per fact fails to fall as the log grows is a mechanism that eventually kills the instance.

## What is different

Against the system that runs today, four changes carry the weight.

**A fact stops being a sentence.** Every downstream mechanism currently re-derives structure from prose: deduplication through embedding geometry, arbitration through model judgement, temporal placement through date extraction, each paid repeatedly and fallibly. Capturing structure at write time means nothing has to recover it later, and a structural question gets a structural answer.

**Prose stops being the only representation, without stopping being a representation.** The narrative is kept and indexed rather than discarded or demoted, because sequencing, change-tracking, and synthesis across occasions run on it.

**The writer stops being unchecked.** Today the guards check authority and visibility but never well-formedness, so a confidently recorded falsehood meets nothing until a contradiction happens to collide with it. Writes now cross a typed seam into sound symbolic critics that can reject them.

**Load-bearing behaviour stops riding on wording.** A behaviour whose failure is silent moves off the prompt and into structure the harness enforces. The prompt keeps what tolerates drift: principles, taste, and voice.

## Reading this tree

The chapters above are the design. Beside them:

- [`coverage.md`](coverage.md) grades the design against the surveyed failures of the current system and maps it onto the open issues. The grades are uneven on purpose.
- [`confidence.md`](confidence.md) records what each load-bearing claim rests on, and which ones are unsettled.
- [`evolution.md`](evolution.md) is the build order from here to there.
- [`research/`](research/) is the evidence, including the [corpus study](research/2026-08-03/modelling-study.md) that tested this model against real recorded data before these chapters were written, and changed them.

The design targets a new instance at genesis. The existing instance is not migrated.
