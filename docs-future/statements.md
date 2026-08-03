# Statements

A Statement is the atomic unit of the store. Everything the agent knows is one, everything it writes produces one, and every read resolves to a set of them. There is no smaller unit and no parallel container: a fact, an endorsement of a fact, and the provenance of a fact are the same object seen from different angles.

A Statement carries eight things:

| | |
|---|---|
| **Claim** | a typed triple, or a reference to an [Event](events-and-roles.md) |
| **Frame** | which referential layer the claim is made in |
| **Gloss** | a reference to the utterance the claim was drawn from |
| **Provenance** | who said it, where, and when it was observed against when it was recorded |
| **Validity** | the interval over which the claim holds in the world |
| **Credence** | how strongly it is held, and on what evidence |
| **Transmission principle** | who may learn it, and under what condition |
| **Derivation** | for a derived Statement, what it was derived from and under which assumptions |

The rest of this chapter takes each in turn. Two of them, the frame and the gloss, exist because [the modelling study](research/2026-08-03/modelling-study.md) found the model without them could not hold real recorded content.

## The claim

A claim takes one of two shapes.

A **triple** relates a subject to an object through a registered relation:

```
s1  (person/rowan, worked_at, org/northwind)
```

An **event reference** points at an Event node carrying role-edges, which is how a happening with more than one participant is held:

```
s2  → e1
e1  event/create
    agent   person/wren
    theme   person/quill
    source  topic/instance_architecture
    time    [2026-07-14, 2026-07-16)
```

The subject is always a memory handle. The object is one of four things:

- **a memory handle**, for a claim relating two known entities
- **a typed value**: a date, a duration, a quantity with a unit, or a recurrence, each of which is a first-class value rather than a string that happens to parse
- **an opaque literal**, for content that has internal structure the store does not model, such as a formula or a code fragment
- **another Statement**, for a propositional attitude

### A Statement as an object

A large part of what a personal agent records is not a claim about the world but a claim about someone's stance toward the world: what they argue, deny, warn about, or concede. The object of such a claim is itself a claim.

```
s3  (revolution/1789, was, rewrite_of_legacy_system)
s4  (person/quill, argues, s3)
```

`s4` is asserted. `s3` is **quoted, not asserted**: it is visible only as the object of `s4`, never as a fact about `revolution/1789`, and never returned by a read that asks what the store believes about the revolution. This is the same quotation boundary that governs claims arriving from another agent, applied one level inward.

Nesting is bounded to one level. A claim about a claim about a claim is expressible as prose in the gloss and is not worth the machinery.

The alternative, putting an unparsed sentence in the object slot, reinstates prose-as-fact one level down and is the specific thing this model exists to prevent.

## The frame

Every Statement declares which referential layer it is made in. The layer is a small closed set:

- **`actual`**: the claim is about the entity as it exists. A bot's model version, a person's employer, a room's topic.
- **`persona`**: the claim is about a character the entity presents. A persona agent's opinions, its stated history, its manner.
- **`source`**: the claim is about the material a persona is drawn from. The historical figure behind a character, the corpus behind a voice.

```
s5  (person/quill, runs_on, model/opus-4.8)          frame actual
s6  (person/quill, admires, doctrine/single_chamber)  frame persona
s7  (person/ferrer, executed_in, 1794)                frame source
```

Without the frame, `s7` written against `person/quill` is well-typed, passes every critic, and is false. Domain and range checks cannot catch it because the types are correct; the error is in which layer the predicate applies to. The study found this failure live, in a corpus where 39% of entries sat on a single persona agent, a quarter of them touching the historical layer and several mixing two layers inside one entry.

The frame is load-bearing in three places. A read defaults to `actual` and must opt into the others, so a question about what a bot runs on never returns what its character believes. A `source` claim never propagates to the entity presenting the persona. And a frame mismatch between subject and relation is a checkable condition, so the critic bank has something to check.

The frame is not a hedge and not a credence. A `persona` claim can be perfectly certain; it is simply certain about a character.

## The gloss

A Statement points at the utterance it was drawn from. It does not contain it.

This distinction is the one the modelling study forced. A single sentence routinely yields many claims: one observed biography entry carried nationality, residence, four employers, two projects, and a name. Those are eight Statements and one utterance. Splitting the sentence eight ways would manufacture eight phrases nobody said, so the gloss is shared:

```
g1  utterance, turn:01J7…
    "Australian programmer living in Sweden, worked at Northwind and
     three others, designed the file format, real name withheld"

s8  (person/rowan, nationality, country/au)      gloss g1
s9  (person/rowan, resides_in, country/se)       gloss g1
s10 (person/rowan, worked_at, org/northwind)     gloss g1
…
```

The gloss is a **second trace, not a fallback**. It is indexed in its own right and retrieved alongside the structure rather than consulted when structure fails. The two carry different things and answer different questions: the structure supports precision, and the narrative supports recall. Where a question turns on sequencing, change over time, or synthesis across occasions, the gloss is what carries the answer, and where a question is a single lookup it adds nothing. See [the two traces](two-traces.md).

Sharing a gloss has a consequence worth stating: **visibility is per-Statement, not per-utterance**. The biography above yields seven public Statements and one private-to-teller, from one sentence. A compound utterance cannot carry a single transmission principle, and the store does not ask it to.

Some content survives only as a gloss. Metaphor, analogy, and reframing have no claim to extract, and any structural decomposition destroys what was said. A Statement over such an utterance carries a weak claim about its subject and leans on the gloss for everything else. This is a designed outcome rather than a shortfall.

## Provenance

Provenance qualifiers sit exactly one level deep. A qualifier never carries its own qualifier, because a qualifier modifying a qualifier makes scope ambiguous and the ambiguity is not worth what it buys.

```
s11 (person/wren, keeps_pet, animal/pepper)
    told_by   person/wren
    told_in   turn:01J7…
    observed  2026-07-14
    recorded  2026-07-16
```

`told_by` names a teller. A Statement with several tellers is a fact a set of people stand behind, which is what an endorsement is: there is no separate attestation object, and each teller's endorsement carries its own transmission principle and its own retraction authority. The last teller's retraction ends the Statement's life; an earlier one's does not.

The observed-against-recorded pair is a genuine axis, not bookkeeping. It is what lets a document authored years ago and ingested today record both truthfully, and it is what relieves the pressure to date a claim by the day it was heard. A claim whose utterance anchors no time leaves its validity open and still sorts correctly, because the occasion of learning is held by the [episode](memory-typology.md), not smuggled into the claim.

## Validity

A claim holds over an interval, which may be open at either end.

```
s12 (person/rowan, worked_at, org/northwind)  valid [2019-03, 2021-06)
s13 (person/quill, runs_on, model/opus-4.8)   valid [2026-07-16, …)
```

Supersession closes a window; it never deletes. Learning that someone has changed employer closes the old interval and opens a new one, and both remain readable. This is what makes "where did they work in 2020" answerable at all, and it is why a time-bounded fact stops having to be prose.

Two supersession axes stay distinct. Closing a validity window says the claim stopped being true. Retracting says it was never true. Conflating them loses the difference between a person changing jobs and the store having been wrong about their job.

Dates, durations, quantities, and recurrences are typed values throughout, with strings only at the input boundary. See [time](time.md).

## Credence

Credence is derived from evidence, never from a model stating a number. It moves on the count and independence of the tellers who stand behind a claim, weighted by how reliable each has been.

The agent sees a coarse ordinal with its evidence attached, because that is the granularity the distinction actually supports:

```
s14 (person/quill, persona_of, person/ferrer)
    credence  confirmed · two independent tellers
```

A claim recorded as a hedge does not need the hedge in its text. "Likely drawn from Ferrer" and a later flat assertion of the same relation are one Statement whose credence moved, not two entries whose relationship is invisible.

Two tellers who are repeating each other are not two pieces of evidence. Dependence between attestations is a provenance determination, and dependent evidence produces no confidence gain. See [belief](belief.md).

## Transmission principle

Each Statement carries the condition under which it may travel, as data rather than as a fixed enum: in confidence, attributed to its teller, public, reciprocal, or restricted to a named purpose.

The evaluator resolves a principle against the present audience and the log's history, cheaply and fail-closed. A principle is a predicate, not a query, and the surface stays a small vocabulary the agent can reason about. See [privacy and provenance](privacy-and-provenance.md).

## Derivation

A derived Statement, one produced by consolidation, distillation, or inference rather than by an utterance, records what it came from:

```
s15 (person/rowan, collaborates_with, person/wren)
    derived_from  [s1, s4, e1]
    activity      link-inference
    agent         model:…, template:…@v3
    criterion     co-participation in two or more events
    assumes       [merge#7]
```

Provenance is computed with the conclusion rather than attached afterwards, and it records the evidence and the criterion, not merely which model and template ran. The assumption stamp lists the revocable assumptions the derivation treated as holding, which is what makes retraction propagate: withdrawing `merge#7` voids everything stamped with it and re-derives from what remains.

## Equality

Two Statements are the same Statement when their claim, frame, and validity interval agree. This is a structural test, not a similarity threshold.

The consequence is that a re-mention resolves to the existing Statement rather than appending a near-copy, and that the commonest form of duplication stops needing a similarity threshold to catch. In the observed corpus, sixteen entries were exact textual duplicates of another entry and many more were rewordings of one claim; all of them collapse structurally.

Deduplicating claims does **not** deduplicate occasions. Two Statements that resolve to one may still have been learned on two occasions, from two tellers, in two utterances, and all of that is retained: the tellers accumulate on the Statement, and each occasion keeps its own gloss. The distinction is load-bearing, because the redundancy across occasions is what supports cross-checking a claim against its own record, and collapsing it would discard the thing that makes multi-occasion recall work.

## What a Statement is not

Three kinds of content are deliberately outside this model.

**Directives.** Instructions about how to behave in a context, and the agent's own charter, are configuration rather than memory. They have no teller, no truth value, no credence, no validity interval, and no audience. They live in their own kind, with their own lifecycle, and are never mistaken for facts. The observed corpus held twenty-two such entries filed as ordinary content, which is a category error this model declines to inherit.

**Formal content.** A proof, a formula, or a fragment of code has internal structure the store does not model. It is held as an opaque literal under a typed relation, which is honest: the structure adds nothing, queries nothing, and checks nothing, and pretending otherwise would be decoration.

**Figurative content.** Metaphor and analogy are carried by the gloss, as described above. There is no claim to extract, and extraction would destroy the content.

Naming these three is part of the model. A representation that accommodates everything constrains nothing, and the value of the Statement is precisely in what it refuses to hold.
