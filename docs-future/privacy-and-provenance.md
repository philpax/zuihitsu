# Privacy and provenance

Privacy is not a tag on a fact. It is a condition on how a fact may flow, carried as data on each [Statement](statements.md) and evaluated against who is present and what the log records.

## Transmission principles

Each Statement carries the condition under which it may travel. The conditions are a small registered vocabulary, each compiling to a deterministic predicate over the present audience and the log's history:

| | |
|---|---|
| `in_confidence` | only back to the teller |
| `attributed` | may be repeated, always with its teller named |
| `public` | may be repeated freely, and may be distilled |
| `reciprocal` | may be shared with someone who has shared comparably |
| `with_consent(e)` | permitted once a named consent event is in the log |
| `purpose(p)` | permitted only in service of a named purpose |
| `except(S)` | permitted except to a named set |

Making these data rather than a fixed enum buys three things a four-value enum cannot. A confidence can cross an inter-agent boundary, because a principle travels with the claim where a local enum value means nothing to a different system. The subject guard becomes a **derived** negative norm, one instance of "never to this set", rather than a special case bolted on. And a reminder or calendar flow gets a principled account of why it may surface something, through `purpose`, rather than an exception.

The evaluator stays cheap, deterministic, and fail-closed. A principle is a predicate resolved against the present set, never a query over a knowledge base, and an unresolvable condition denies. Audiences are predicates over who is present rather than enumerated sets, because enumerating audiences over a growing population is combinatorial and the enumeration is never right for long.

## Zero residue

An uncleared confidence must leave no trace. Not a softened version, not a hint, not a conspicuous gap: an observer must not be able to distinguish a surface where the fact was withheld from one where it was never known.

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
