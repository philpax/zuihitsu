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
- **a typed value**: a date, a duration, a measure with a unit, a count over a kind, or a recurrence, each of which is a first-class value rather than a string that happens to parse
- **an opaque literal**, for content that has internal structure the store does not model, such as a formula or a code fragment
- **another Statement**, for a propositional attitude
- **a gloss or turn reference**, for a claim about something that was said

The last of these is what a metalinguistic claim needs. A great deal of what a personal agent records is somebody's stance toward a specific past utterance: rating a line, conceding one phrase and disputing another, quoting how they were described. The target is a thing that was said, which the store already holds as a first-class node, and neither of the alternatives works: an opaque literal duplicates text the gloss already carries and is unqueryable, while minting a handle for a passing phrase is the same absurdity that [counting](#counting) exists to avoid.

### A Statement as an object

A large part of what a personal agent records is not a claim about the world but a claim about someone's stance toward the world: what they argue, deny, warn about, or concede. The object of such a claim is itself a claim.

```
s3  (revolution/1789, was, rewrite_of_legacy_system)
s4  (person/quill, argues, s3)
```

`s4` is asserted. `s3` is **quoted, not asserted**: it is visible only as the object of `s4`, never as a fact about `revolution/1789`, and never returned by a read that asks what the store believes about the revolution. This is the same quotation boundary that governs claims arriving from another agent, applied one level inward.

Nesting is bounded to one level. A claim about a claim about a claim is expressible as prose in the gloss and is not worth the machinery.

The bound costs something real, and the corpus shows where. An attitude toward a *position* rather than toward a proposition, rebutting the view that something was a crash rather than rebutting the claim itself, is depth two, and flattening it to depth one discards that the disagreement is with a stance somebody holds. Part of the attitude class this mechanism is justified by has that shape, so the fix covers most of the class rather than all of it.

The alternative, putting an unparsed sentence in the object slot, reinstates prose-as-fact one level down and is the specific thing this model exists to prevent.

### Counting

"Quill has five ice creams" is neither one claim about an entity nor five claims about five entities. Minting five ice-cream handles nobody will ever refer to individually is absurd, and putting the number in the prose puts it beyond every query.

A **count over a kind** is a typed value in the object slot:

```
s5  (person/quill, possesses, count(5, kind/ice_cream))
    valid  [2026-07-14, 2026-07-16)
```

A count carries an amount, an optional unit, and optional upper and lower bounds, which is Wikidata's quantity shape adopted wholesale. The bounds are what distinguish "about a hundred thousand words" from "exactly a hundred thousand words", and [the corpus study](research/2026-08-03/modelling-study.md) found both shapes recorded, along with several that were explicitly approximate.

Three further properties make this worth having as a value rather than a qualifier.

**A count that changes is a window closing.** Eating one does not contradict the claim that there were five; it ends it. The count-of-five interval closes and a count-of-four opens, and both remain readable, which is the same mechanism every other time-bounded claim uses.

**A count declares whether it is closed.** "Has five" and "has at least five" are different claims, and only the first is contradicted by a sixth. The value carries which was meant, so a later mention of a sixth is either a contradiction to be weighed or an ordinary update.

**A count can be refined into individuals.** When one member becomes salient, and it usually does when something happens to it, the count is superseded by window-closing and individuated claims take over. The refinement is an ordinary supersession rather than a rewrite, so nothing that referred to the count is orphaned.

This is instance-level cardinality: a fact about one thing at one time. It is distinct from the class-level cardinality declared on a [relation definition](relations.md), which constrains every instance and is enforced by a critic. Both exist, and they are different mechanisms. See [`lineage.md`](lineage.md) for how the distinction descends from OWL's qualified cardinality restrictions.

Measures with units, such as a word count or an elapsed duration, are the neighbouring case and use the same slot. [The corpus study](research/2026-08-03/modelling-study.md) found both written into prose with their units, uninterpretable to any query.

A count over participants is for the **unindividuated** case only. Where the participants are known people they are role-edges on an [Event](events-and-roles.md), one per participant, each an independently addressable Statement with its own teller and its own audience. Collapsing known participants into a count discards exactly the property the Event node exists to provide. "About thirty people showed up" is a count; "Wren, Rowan, and Quill lifted the piano" is three `agent` edges. A count whose members later become salient individuates by the ordinary refinement above.

The same rule decides how a list is recorded. Nine named authors are individuated by the speaker in the act of listing them, so they are nine Statements rather than one opaque list, and the list form is reserved for the case where nobody has individuated anything, where it is the same object as a count over a kind. A list that begins vague and later becomes specific refines exactly as a count does.

Collective and distributive readings are not distinguished in either shape. Whether they lifted it together or separately lives in the gloss.

This is a **declined** distinction, not an unknown one. Conceptual graphs mark it in the notation, separating a collective plural from a distributive one, and the lattice-theoretic treatment of plurals behind that notation is mature. We are choosing not to pay for it, and [`research/2026-08-03/counting-and-quantity.md`](research/2026-08-03/counting-and-quantity.md) records where to go if plurals ever become load-bearing.

## The frame

Every Statement declares which referential layer it is made in. The layer is a small closed set:

- **`actual`**: the claim is about the entity as it exists. A bot's model version, a person's employer, a room's topic.
- **`persona`**: the claim is about a character the entity presents. A persona agent's opinions, its stated history, its manner.
- **`source`**: the claim is about the material a persona is drawn from. The historical figure behind a character, the corpus behind a voice.

```
s6  (person/quill, runs_on, model/opus-4.8)          frame actual
s7  (person/quill, admires, doctrine/single_chamber)  frame persona
s8  (person/ferrer, executed_in, 1794)                frame source
```

Without the frame, `s8` written against `person/quill` is well-typed, passes every critic, and is false. Domain and range checks cannot catch it because the types are correct; the error is in which layer the predicate applies to. The study found this failure live, in a corpus where 39% of entries sat on a single persona agent, a quarter of them touching the historical layer and several mixing two layers inside one entry.

The frame is load-bearing in three places. A read defaults to `actual` and must opt into the others, so a question about what a bot runs on never returns what its character believes. A `source` claim never propagates to the entity presenting the persona. And a frame mismatch between subject and relation is a checkable condition, so the critic bank has something to check.

The frame is not a hedge and not a credence. A `persona` claim can be perfectly certain; it is simply certain about a character.

What the frame does **not** fix is a wrong subject. It marks which layer a claim is made in, on a subject already chosen, so a claim about the person *behind* a persona that was filed onto the persona itself is not repaired by any of the three values: the claim is not about the character, and `source` points at the material the character draws on rather than at the principal presenting it. The corpus contains that case, a detail about the operator's household recorded against the persona agent, publicly.

### Redirection to a principal

The fix is not a fourth layer. A value meaning "in the principal's layer" leaves the persona as the triple's subject and asks every reader to re-target, so a question about whose cat it is still has to know to traverse. A second subject coordinate is worse: two subjects is the scope ambiguity the one-level qualifier discipline exists to prevent.

Instead the doctrine already running elsewhere applies: **the frame says which layer, the substrate says which handle.**

`presents` is a seed [relation](relations.md) from a principal to a persona. `principal` joins the frame's closed set as a **redirect marker rather than a layer**: it means re-target this claim at the subject's principal. A hard critic resolves it at write time, reading the `presents` edge, rewriting the subject, and storing the frame as `actual`. What lands is an ordinary claim about the person, with a provenance qualifier recording that it arrived by way of the persona. Nothing downstream ever sees the marker, so the stored frame stays three-valued and every read defaults as before.

Four properties make it safe.

**It is declared, never inferred.** An extractor may not propose redirection, because a misfire files a claim about a bot onto a human, and that failure direction is severe. Only a writer who understood the conversation sets it.

**An unknown principal is a teachable error, not a guess.** If no `presents` edge resolves, the write is refused with the question that would fix it, and a persistent failure reaches a person like any other schema gap.

**It is revocable.** The redirect records as a derivation whose assumption stamp names the `presents` edge, so withdrawing that edge voids the redirected claims on the next fold. This is [the severance fold-filter](identity.md), reused rather than reinvented.

**It makes the subject guard bind the right person.** That is the live failure exactly: a household detail about a real person sat publicly on a bot's memory, where no guard about that person applied to it. Under redirection the claim is about the person, so the guard that protects them is the one that runs.

The agent-facing cost is one more value in a small enum, and it is the easiest of them to answer, because it maps onto a question a participant in the conversation always knows: is this about the character, or about the human behind it? With it, the closed set covers the persona relationship completely: `persona` for the character, `principal` for the person presenting it, `source` for the material the character draws on.

It is also not new. Cyc solved this generally with microtheories, asserting in a fiction context that a character is a fourth-grader while asserting in the real-world context that the same character is a cartoon. The frame is a deliberate simplification of that idea: closed where microtheories are open, three-valued where they are a lattice, and checkable by a critic where a general context logic is not. See [`lineage.md`](lineage.md).

## The gloss

A Statement points at the utterance it was drawn from. It does not contain it.

This distinction is the one the modelling study forced. A single sentence routinely yields many claims: one observed biography entry carried nationality, residence, four employers, two projects, and a name. Those are eight Statements and one utterance. Splitting the sentence eight ways would manufacture eight phrases nobody said, so the gloss is shared:

```
g1  utterance, turn:01J7…
    "Australian programmer living in Sweden, worked at Northwind and
     three others, designed the file format, real name withheld"

s9  (person/rowan, nationality, country/au)      gloss g1
s10 (person/rowan, resides_in, country/se)       gloss g1
s11 (person/rowan, worked_at, org/northwind)     gloss g1
…
```

The gloss is a **second trace, not a fallback**. It is indexed in its own right and retrieved alongside the structure rather than consulted when structure fails. The two carry different things and answer different questions: the structure supports precision, and the narrative supports recall. Where a question turns on sequencing, change over time, or synthesis across occasions, the gloss is what carries the answer, and where a question is a single lookup it adds nothing. See [the two traces](two-traces.md).

Sharing a gloss has a consequence worth stating: **visibility is per-Statement, not per-utterance**. The biography above yields seven public Statements and one private-to-teller, from one sentence. A compound utterance cannot carry a single transmission principle, and the store does not ask it to.

Some content survives only as a gloss. Metaphor, analogy, and reframing have no claim to extract, and any structural decomposition destroys what was said. A Statement over such an utterance carries a weak claim about its subject and leans on the gloss for everything else. This is a designed outcome rather than a shortfall.

## Provenance

Provenance qualifiers sit exactly one level deep. A qualifier never carries its own qualifier, because a qualifier modifying a qualifier makes scope ambiguous and the ambiguity is not worth what it buys.

```
s12 (person/wren, keeps_pet, animal/pepper)
    told_by   person/wren
    told_in   turn:01J7…
    expressed hedged
    observed  2026-07-14
    recorded  2026-07-16
```

`expressed` records how firmly the telling was put: hedged, plain, or emphatic. It qualifies the act of telling, not the claim, and it is deliberately not a credence. Two people each saying "probably" are two tellers who both declined to commit, which is a different state from two people asserting flatly, and a model that keeps only the count cannot tell them apart. Keeping it here rather than as a nested attitude matters because hedging is constant: paying for a Statement-in-an-object-slot every time somebody says "I think" would be absurd, where a qualifier on the telling costs one field. See [belief](belief.md) for what it does and does not move.

`told_by` names a teller. A Statement with several tellers is a fact a set of people stand behind, which is what an endorsement is: there is no separate attestation object, and each teller's endorsement carries its own transmission principle and its own retraction authority. The last teller's retraction ends the Statement's life; an earlier one's does not.

The observed-against-recorded pair is a genuine axis, not bookkeeping. It is what lets a document authored years ago and ingested today record both truthfully, and it is what relieves the pressure to date a claim by the day it was heard. A claim whose utterance anchors no time leaves its validity open and still sorts correctly, because the occasion of learning is held by the [episode](memory-typology.md), not smuggled into the claim.

### Who else heard it

Provenance names the teller. Who else was there belongs to the [gloss](two-traces.md) rather than to each Statement, because it is a property of the occasion: one sentence spoken in a room of four yields eight Statements and one account of who heard it.

That account is **two sets**, because one field cannot serve both readers. The **disclosure set** is who demonstrably took part, and it is the only one the audience evaluator reads. The **exposure set** is who the utterance reached, and it is read only by the dependence test. See [privacy and provenance](privacy-and-provenance.md) for why the narrow one licenses and the wide one only suppresses.

```
g1  utterance, turn:01J7…
    told_by    person/wren
    disclosure [person/rowan]
    exposure   [person/rowan, person/quill]
```

Two mechanisms read these, and neither is computable without them.

A [transmission principle](privacy-and-provenance.md) is evaluated against the present audience *less* the disclosure set. Something said in front of four people is not a confidence held from any of the four, and withholding it from someone who was standing there is not discretion; it is a conspicuous silence in front of a person who knows better.

[Dependence between attestations](belief.md) is partly determined by the exposure set. Two tellers who were both present when a third said something are not two pieces of evidence, and in a shared channel that is the ordinary case rather than the exception.

The disclosure set widens an audience, which no other field in the model does, so it is built from demonstrated participation rather than from channel membership. See [privacy and provenance](privacy-and-provenance.md).

Two rules about the agent's own place in this follow, and both exist to stop the agent inflating its own evidence.

**The agent is a witness to everything told to it, and never an independent teller of it.** A claim the agent re-records in its own words is the same claim it was told, so the re-recording adds an occasion and not a teller. Without this, a single sentence read back into the store counts as corroboration from two sources, which the live corpus makes a large problem rather than a theoretical one: a substantial fraction of recorded content is agent-told, much of it restating what a participant had just said.

**What the agent says is an utterance like any other.** Its outbound turns produce glosses whose witnesses are their recipients, which is what makes a relay chain visible. If one person tells the agent something, the agent relays it, and the recipient later tells it back, the second teller is genuinely distinct and their evidence is entirely derived from the first. That is dependence through the agent rather than through a shared occasion, and it is only detectable because the relay left a record. See [belief](belief.md).

## Validity

A claim holds over an interval, which may be open at either end.

```
s13 (person/rowan, worked_at, org/northwind)  valid [2019-03, 2021-06)
s14 (person/quill, runs_on, model/opus-4.8)   valid [2026-07-16, …)
```

Supersession closes a window; it never deletes. Learning that someone has changed employer closes the old interval and opens a new one, and both remain readable. This is what makes "where did they work in 2020" answerable at all, and it is why a time-bounded fact stops having to be prose.

A closure records **why** it happened, as a small closed set: superseded by a later claim, corrected by one, expired against a stated horizon, retired as no longer worth carrying, or withdrawn by its teller. Without it, a claim that was corrected and a claim that simply stopped holding read identically afterwards, which is the same conflation the retraction distinction below exists to prevent, one level down. The values are the decisions [the staleness ladder](time.md) already enumerates, and an explicit unknown is available so declining is recorded rather than guessed.

Two supersession axes stay distinct. Closing a validity window says the claim stopped being true. Retracting says it was never true. Conflating them loses the difference between a person changing jobs and the store having been wrong about their job.

Dates, durations, quantities, and recurrences are typed values throughout, with strings only at the input boundary. See [time](time.md).

## Credence

Credence is derived from evidence, never from a model stating a number. It moves on the count and independence of the tellers who stand behind a claim, weighted by how reliable each has been.

The agent sees a coarse ordinal with its evidence attached, because that is the granularity the distinction actually supports:

```
s15 (person/quill, persona_of, person/ferrer)
    credence  confirmed · two independent tellers
```

A claim recorded as a hedge does not need the hedge in its text. "Likely drawn from Ferrer" and a later flat assertion of the same relation are one Statement told twice, not two entries whose relationship is invisible. How firmly each telling was put rides the `expressed` qualifier above; the credence moves only if a second independent teller arrives, because one person growing surer is not corroboration. See [belief](belief.md).

Two tellers who are repeating each other are not two pieces of evidence. Dependence between attestations is a provenance determination, and dependent evidence produces no confidence gain. See [belief](belief.md).

## Transmission principle

Each Statement carries the condition under which it may travel, as data rather than as a fixed enum: in confidence, attributed to its teller, public, reciprocal, or restricted to a named purpose.

The evaluator resolves a principle against the present audience and the log's history, cheaply and fail-closed. A principle is a predicate, not a query, and the surface stays a small vocabulary the agent can reason about. See [privacy and provenance](privacy-and-provenance.md).

## Derivation

A derived Statement, one produced by consolidation, distillation, or inference rather than by an utterance, records what it came from:

```
s16 (person/rowan, collaborates_with, person/wren)
    derived_from  [s1, s4, e1]
    activity      link-inference
    agent         model:…, template:…@v3
    criterion     co-participation in two or more events
    assumes       [merge#7]
```

Provenance is computed with the conclusion rather than attached afterwards, and it records the evidence and the criterion, not merely which model and template ran. The assumption stamp lists the revocable assumptions the derivation treated as holding, which is what makes retraction propagate: withdrawing `merge#7` voids everything stamped with it and re-derives from what remains.

A derived Statement's transmission principle is **the intersection of its premises'**, computed with the conclusion like everything else in the record. This holds wherever a derivation happens, on a turn or in [a pass](off-turn.md), and it is what stops an inference from being the aggregate leak the [distillation boundary](privacy-and-provenance.md) exists to prevent: a conclusion that could only have been reached from a confidence is that confidence, restated.

## Equality

Two Statements are the same Statement when their claim, frame, validity interval, **and assertedness** agree. This is a structural test, not a similarity threshold.

Assertedness is in the key because leaving it out collapses a quotation into a belief. A proposition quoted as the object of one person's attitude and the same proposition later asserted flatly by someone else share a claim, a frame, and an interval, and they are not the same Statement: one is a fact the store holds and the other is a fact about what somebody said. Collapsing them either makes the store believe what it only quoted, or swallows a real assertion into a node that is never independently retrievable.

Promotion is therefore explicit. When a quoted proposition is later asserted, the assertion is its own Statement with its own tellers, credence, and transmission principle, and the two are related rather than merged. The quoted one remains what it always was: readable only through the attitude that carries it, and ended by the retraction of that attitude rather than by any authority of its own.

The consequence is that a re-mention resolves to the existing Statement rather than appending a near-copy, and that the commonest form of duplication stops needing a similarity threshold to catch. In the observed corpus, sixteen entries were exact textual duplicates of another entry and many more were rewordings of one claim; all of them collapse structurally.

Deduplicating claims does **not** deduplicate occasions. Two Statements that resolve to one may still have been learned on two occasions, from two tellers, in two utterances, and all of that is retained: the tellers accumulate on the Statement, and each occasion keeps its own gloss. The distinction is load-bearing, because the redundancy across occasions is what supports cross-checking a claim against its own record, and collapsing it would discard the thing that makes multi-occasion recall work.

## What a Statement is not

Five kinds of content are deliberately outside this model.

**Directives.** Instructions about how to behave in a context, and the agent's own charter, are configuration rather than memory. They have no teller, no truth value, no credence, no validity interval, and no audience. The charter lives in [the self slot](memory-typology.md) and a directive in [its own kind](memory-typology.md), scoped and versioned, and neither is ever mistaken for a fact. The observed corpus held twenty-two such entries filed as ordinary content, which is a category error this model declines to inherit.

**Formal content.** A proof, a formula, or a fragment of code has internal structure the store does not model. It is held as an opaque literal under a typed relation, which is honest: the structure adds nothing, queries nothing, and checks nothing, and pretending otherwise would be decoration.

**Figurative content.** Metaphor and analogy are carried by the gloss, as described above. There is no claim to extract, and extraction would destroy the content.

**Dispositions and generics.** A tendency, a habit, or a recurring dynamic between two people is not a claim that holds over an interval; it is a claim about how things usually go. Someone who tends to talk at length, or a pair whose exchanges reliably spark each other's side projects, has no representation here: the subject may be a dyad rather than a handle, the quantification is habitual rather than temporal, and a rate stated as a bound is neither a count over a kind nor a recurrence. These degrade to a thin claim leaning on the gloss. The corpus contains several, and one of them is the fourth entry of the [flagship four-copy case](events-and-roles.md), so this is a known gap rather than a rare one. A habitual modality alongside the frame is the obvious extension and is deliberately not taken here.

**Third-party deontics.** What someone else has forbidden or permitted the agent, told by them, is a Statement and not [configuration](memory-typology.md): it has a teller, a validity interval, an audience, and a truth value, all of which a directive lacks. What flattens is the deontic force. The object is an activity description, which the store holds only as an opaque literal, and the modality ends up in the relation name. This is a recorded cost rather than a solved problem, and filing such a claim as a directive would put another person's constraint into the agent's own charter, which is the worse error.

Naming these five is part of the model. A representation that accommodates everything constrains nothing, and the value of the Statement is precisely in what it refuses to hold.
