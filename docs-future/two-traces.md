# The two traces

Every recorded occasion leaves two traces: the **structure** extracted from it, and the **narrative** of the occasion itself. Neither is subordinate to the other. They are indexed separately, retrieved together, and they answer different questions.

Structure serves **precision**. Deduplication by structural equality, reads that traverse role-edges to answer who did what, critics that can type-check a write, audience conditions that evaluate deterministically: all of these need a claim to be a claim rather than a sentence.

Narrative serves **recall**. Distinctiveness between similar memories, temporal anchoring, and the sequencing and synthesis that spans occasions: these run on the elaborated language of the occasion, and structure alone does not supply them.

## Why they are complementary rather than ranked

The temptation is to treat prose as a safety net for when structure is wrong, kept but demoted. The evidence says the two carry different load.

In a controlled experiment ([`research/2026-08-03/dual-trace.md`](research/2026-08-03/dual-trace.md)) pairing each structured record with an elaborated narrative, against a structured-record-only control at matched coverage, the pair gained 40 points on temporal reasoning, 30 on multi-session aggregation, and 25 on tracking how information changed. On single-occasion lookup the gain was **exactly zero**, with no discordant questions in either direction.

The null is what makes the result usable. A treatment that improved everything would be indistinguishable from simply having more text available. A treatment that helps only where a question spans occasions, and demonstrably not where one lookup suffices, is telling us something specific about what the second trace does: it distinguishes and orders memories rather than making individual facts easier to find.

Two caveats travel with this and are recorded in [`confidence.md`](confidence.md). The experiment is a single unreplicated study on one benchmark with an automated judge. And it could not separate whether the benefit comes from generating the narrative at encoding or from reading it at retrieval, which is the difference between a design that costs a model call per occasion and one that costs almost nothing. [`evolution.md`](evolution.md) resolves that with an experiment before committing to the expensive arm.

## A gloss belongs to an utterance

A gloss is not a property of a Statement. Many Statements point at one gloss.

This follows from how people talk. One sentence routinely carries many claims: [the corpus study](research/2026-08-03/modelling-study.md) found a single observed entry carrying eight, and twenty entries in a 198-entry corpus carrying three or more. Attaching a private gloss to each of the eight would mean inventing eight phrases nobody uttered, which is worse than useless: it manufactures evidence.

```
g1  utterance, turn:01J7…
s8, s9, s10, …  →  g1
```

Two consequences matter.

**Visibility stays per-Statement.** One utterance can yield claims with different audiences, and the corpus contains the case: a biography recorded as one public entry including a private detail, later split so the detail could be held back. Structure fragments; posture rides the fragments.

**Some content is only a gloss.** Metaphor, analogy, and reframing have no claim to extract, and decomposing them destroys what was said. A Statement over such an utterance carries a thin claim and leans on the narrative for everything. This is a designed outcome, and the corpus study found it independently: a real fraction of what a personal agent records is content whose only faithful representation is the prose.

## Deduplicate claims, preserve occasions

This is the seam where the two traces pull against each other, and it is stated here because getting it wrong reinstates a failure the model exists to fix.

A re-mention of a known fact resolves to the existing Statement. It does not create a second one. That is what kills the observed failure of one happening recorded four times in subject-appropriate rephrasings.

A re-mention is nonetheless a **second occasion**, and the occasion is kept: its own gloss, its own turn reference, its own teller added to the Statement. The store ends with one claim and two occasions.

The distinction is load-bearing in both directions.

Collapsing occasions along with claims discards the redundancy that lets an agent cross-check a claim against its own record. The dual-trace study reports a case where an agent retrieved the wrong answer, then caught itself because a second occasion's narrative carried an incidental anchor that contradicted the first. That mechanism requires two narratives of overlapping content to survive.

Failing to collapse claims reinstates the copies. If "a second occasion" is read loosely enough, every rephrasing qualifies, and the store fills with near-duplicates again wearing a new name.

The boundary is: **same claim, same frame, same validity interval means one Statement**, regardless of how many times or how differently it is said. Everything else about the occasion is retained. Where the boundary is genuinely unclear, the resolution is a rejectable proposal and the ambiguity is a teachable error, not a silent choice.

## Episodes

An occasion is addressable. A session that warrants it produces an **episode**: a first-class memory carrying the session's span, its participants, the salient turn references, and a narrative body.

Episodes are the unit the second trace is organised around. A Statement knows the occasion that produced it, and an occasion knows its Statements, so retrieving either surfaces the other structurally rather than through a second search. This is a **linked companion** relationship, not a fallback tier consulted when structure fails, which is what the experimental evidence supports: the gain lives where both traces are present and their anchors can be cross-referenced.

Not every session earns an episode. In the study's protocol roughly four in five sessions correctly produced nothing, and the same experiment found that increasing coverage bought a few points while increasing depth bought twenty. A low episode rate is the correct outcome rather than a coverage failure to be tuned away.

## The wall around the narrative

A narrative is composed by the agent, which makes it the surface where invention is most likely and most consequential. The current system has already produced the failure in its unelaborated form: content invented for a document that was never read, and attributed to the person who mentioned it.

Generating narrative deliberately asks a model to commit to concrete detail it was not given. That licence needs a structural boundary, not a sentence in a template asking the model to be careful.

An episodic trace:

- is always told by the agent, never by a participant, so it cannot launder an inference into someone's testimony
- is never a premise in a derivation
- is never distilled into a description of another memory
- never accrues attestation or corroboration
- is marked as a reconstruction wherever it surfaces

These are enforced by the critic bank, not by instruction. See [the seam](the-seam.md).
