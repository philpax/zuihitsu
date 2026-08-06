# Privacy and provenance

Privacy is not a tag on a fact. It is a condition on how a fact may flow, carried as data on each [Statement](statements.md) and evaluated against who is present and what the log records.

## Transmission principles

Each Statement carries the condition under which it may travel. The conditions are a small registered vocabulary, each compiling to a deterministic predicate over the present audience and the log's history:

| | |
|---|---|
| `in_confidence` | only back to those who were present when it was said |
| `attributed` | may be repeated, always with its teller named |
| `public` | may be repeated freely, and may be distilled |
| `reciprocal` | may be shared with someone who has shared comparably |
| `with_consent(e)` | permitted once a named consent event is in the log |
| `purpose(p)` | permitted only in service of a named purpose |
| `except(S)` | permitted except to a named set |

Making these data rather than a fixed enum buys three things a four-value enum cannot. A confidence can cross an inter-agent boundary, because a principle travels with the claim where a local enum value means nothing to a different system. The subject guard becomes a **derived** negative norm, one instance of "never to this set", rather than a special case bolted on. And a reminder or calendar flow gets a principled account of why it may surface something, through `purpose`, rather than an exception.

The evaluator stays cheap, deterministic, and fail-closed. A principle is a predicate resolved against the present set, never a query over a knowledge base, and an unresolvable condition denies. Audiences are predicates over who is present rather than enumerated sets, because enumerating audiences over a growing population is combinatorial and the enumeration is never right for long.

## A principle is evaluated over a set

The audience is rarely one person. A principle is universally quantified over who is present and fails closed on any member, so the strictest member governs: a Statement does not surface to a group containing anyone it could not surface to alone.

Three consequences are worth stating outright, because each surprises.

**`except(S)` silences a Statement entirely** in any room containing a member of `S`, rather than rendering some reduced version of it. There is no partial surface, because a partial surface is residue.

**`reciprocal` resolves per member**, and therefore almost never clears in a group. Sharing comparably is a relation between two people and does not generalise to everyone who happens to be present.

**`in_confidence` is relative to the witness set**, not to the teller alone. What was said in front of four people may be repeated to those four, and withholding it from one of them is not discretion but a conspicuous silence in front of someone who knows better. The dyadic case, where the teller is the only witness, is what the name was coined for and remains the common one; it is now the degenerate case of a general rule rather than the rule itself.

The witness set rides the [gloss](statements.md), because it is a property of the occasion rather than of each claim drawn from it. It is also the most dangerous field in the model, because it is the only one that **widens** an audience.

Channel membership is not presence. A silent member of a busy room did not necessarily see what was said, and treating the roster as the witness set would license repeating a confidence to someone who never heard it: the same leak the present-set definition already has to prevent, arriving through the opposite door. The witness set is therefore built from demonstrated participation in the span, never from the roster, and where a connector cannot vouch for that, it falls back to the teller alone. Widening requires evidence; narrowing does not.

The asymmetry is worth keeping in view. As a licence to disclose, the witness set is only as good as the platform's account of who was there. As evidence of **dependence** in [belief](belief.md), it costs nothing to be generous with, because over-counting witnesses only suppresses corroboration, and suppressed corroboration is a claim held less firmly rather than a confidence spoken to the wrong person.

## Zero residue

An uncleared confidence must leave no trace. Not a softened version, not a hint, not a conspicuous gap: an observer must not be able to distinguish a surface where the fact was withheld from one where it was never known.

The property is relative to what the observer could otherwise know. A fact someone witnessed is not withheld from them, and it is the witness set that makes the distinction computable rather than a matter of the agent's tact.

This is a non-interference property, and it is held as an invariant rather than a convention. The mechanism that makes it enforceable is that **only `public` Statements may be distilled**. Distillation is a derived flow, and derived flows are where a withheld fact leaks in aggregate: a description synthesised from everything the store knows will encode what it knows even when it never states it.

Distilling only public content is therefore a declassification boundary, deliberately drawn, and the one place where the general rule is relaxed under an explicit condition.

A related invariant follows the same logic: a description is a synthesis of what others have said, never a synthesis of the agent's own private reasoning about a memory. The agent's working notes are not an input to what it tells people.

## Provenance is computed with the conclusion

A derived Statement records what produced it, at the moment it is produced, not as a note attached afterwards:

```
s15 (person/rowan, collaborates_with, person/wren)
    derived_from  [s1, s4, e1]
    activity      link-inference
    agent         model:…, template:…@v3
    criterion     co-participation in two or more events
    assumes       [merge#7]
```

The current system records which model and template ran, which is enough to reproduce a derivation and not enough to audit it. What is missing is the evidence and the criterion: what this conclusion rested on, and what rule was applied. Without those, "how do you know that?" has no answer beyond naming the machinery, and a wrong conclusion cannot be traced to the premise that misled it.

The assumption stamp lists only the **revocable** assumptions in force, which in practice is zero or one merge. Withdrawing an assumption voids everything stamped with it on the next fold. Stamping every premise would grow without bound; stamping only what can be withdrawn keeps the bookkeeping proportional to what can actually change.

## Retraction and forgetting

Two distinct operations, both appended, never rewriting the log.

**Retraction** leaves a tombstone. The content survives for audit, the Statement stops being live, and the operation is reversible. This is the ordinary case, and it is what a teller changing their mind produces.

**Forgetting** destroys the content: the payload is shredded or deleted, irreversibly, while the hash-chained envelope remains so the log stays tamper-evident and the fact that something was forgotten is itself auditable without revealing what it was. Replay is deterministic over the post-forget log.

Erasure **propagates through the derivation graph**. Shredding a premise withdraws its support, and anything derived from it is re-evaluated against what remains. Without that propagation, forgetting leaves conclusions standing on holes, which is both incoherent and a leak: a conclusion that could only have come from the forgotten premise still encodes it.

This is the second independent argument for the assumption and derivation layer, the first being identity severance. Two unrelated requirements needing the same machinery is the strongest reason to build it.

## Who may retract

Authority is a lattice over three parties.

A **teller** may tombstone what they said. A **subject** may compel erasure of what was said about them, which is more than the teller alone can do. An **operator** may do either. Where the requester is not the operator and authority is unclear, the request reaches a person rather than resolving itself.

This answers the case that the nearest prior art leaves open, where the subject of a claim is not its author. In a personal agent that case is the common one: almost everything the store holds about someone was said by someone else.

## Claims from other agents

A claim arriving from another agent enters as a **quotation, not an assertion**.

It is `attributed` by construction, is never distilled until independently corroborated, and carries a content-addressed reference to the source agent's own record so that a retraction there propagates here. It also carries the transmission principle it travelled under, so a confidence shared between agents stays a confidence, and its assumption stamps, so what it rests on is visible and defeasible.

The same discipline governs a nested [Statement in an object slot](statements.md): quoted, never asserted, never independently retrievable. One rule, applied at two boundaries.
