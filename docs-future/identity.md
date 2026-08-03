# Identity

A person met on two platforms is two stubs until something says otherwise. What says otherwise is a **merge**, and a merge is a revocable, credence-bearing assumption rather than a permanent union.

All of this lives below a wall. The agent sees one resolved handle per person and never chooses between siblings, never tests two handles for equality, and never learns that the machinery exists.

## Stubs are permanent, merges are not

A platform stub is minted by its connector, keyed on whatever that platform treats as stable, and it never goes away. Merging does not consume it, rename it, or move its content.

```
merge#7
  members    [person/quill@chat, person/quill@forum]
  credence   likely · relational corroboration, two tellers
  asserted   2026-07-20
```

A merge is a first-class object with its own credence, which strengthens under corroboration and weakens under divergence. It is not a boolean, and it is not transitive closure over an equivalence relation.

Transitivity is what makes the current model dangerous. A union-find class is a closure structure, so a single wrong merge corrupts everything reachable from it, and the corruption is indistinguishable from correct structure afterwards. The published record on hard equivalence at scale is unambiguous: the transitive closure of half a billion equivalence assertions manufactured false identities, collapsing 177,000 distinct entities into single classes. Unrestricted substitution is far stronger than what the link is actually used for.

## Same for recall is not same for disclosure

A merge licenses two different things, and they are separable.

**Recall** unifies history: asking about a person returns what they said on both platforms, and the agent reasons about one person. This rides the tentative merge and is safe to be imperfect, because severance genuinely undoes it.

**Disclosure** crosses a boundary in the world: saying something to this account that was told by that one. It is not undoable, and it does not ride the same credence. It requires either evidence past a far stricter bar or a completed challenge-response, and until then the agent may know the connection without acting on it.

Separating these is what lets identity resolution be aggressive where the cost of being wrong is a re-fold, and conservative where the cost is a confidence spoken to the wrong person.

## Evidence comes from structure, not overlap

The obvious signal, two profiles that know the same facts, is unsound. Knowledge can be recited: anyone who has read a conversation can repeat what was in it, so counting shared facts as independent evidence double-counts a single act of copying. Treating recited attributes as corroboration is the mechanism by which a patient impersonator gets merged into someone else's identity.

The signal that resists this is **relational**: two stubs independently linked to the same third parties, by different tellers, through participation, acquaintance, and placement. Forging that requires insinuating oneself into the network rather than reading a transcript, which raises the cost substantially.

It does not close the gap. A sufficiently patient attacker builds real relationships and eventually forges real relational evidence. This is why irreversible disclosure sits behind challenge-response rather than behind a credence threshold, however high.

## Severance is a fold filter

Every derived Statement carries an **assumption stamp**: the small set of revocable assumptions its derivation treated as holding, which in practice is zero or one merge.

```
s15 (person/rowan, collaborates_with, person/wren)
    assumes  [merge#7]
```

Withdrawing a merge appends a severance event. It rewrites nothing. On the next fold, every Statement stamped with the withdrawn merge is voided, and the world reads as though the merge never happened. This is computed rather than reconstructed: there is no forensic pass trying to work out what a merge touched, because each derivation recorded it at the time.

Re-derivation from the now-separated stubs is a **separate, record-time maintenance pass**. It drives models and embedders and its outputs land as new events. The fold itself stays deterministic and model-free, which keeps the replay commitment intact.

The stamp stays minimal by design. Only revocable assumptions are stamped, never every premise, which is what keeps the bookkeeping from growing combinatorially the way a full justification network does.

## The wall

Identity resolution happens in the substrate, before the agent sees anything.

The agent receives one resolved handle for the person it is talking to and writes to what it is given. It does not choose between `person/quill`, `person/quill@chat`, and `person/quill@forum`, because only one of those is ever visible to it. It does not test whether two handles denote the same person, because it never holds two.

This is not a convenience. The current system's identity machinery leaks into behaviour: after a confirmed merge, the agent failed to relay a sibling stub's history back to its own teller in seven of ten runs. The visibility and retrieval machinery held; the agent's model of the merged identity is what faltered. Given a surface with one handle and unified reads, there is nothing left to falter at, because there is no second thing to reason about.

The same wall covers class membership, designated primaries, and merge credence. These are architectural metadata. The agent cannot read them, cannot write them, and cannot second-guess them.

## What the agent can still do

Merging is not something the agent is forbidden from participating in. It can observe that two people seem to be the same and say so, and that observation is evidence like any other, weighted by the fact that the agent is a fallible teller. What it cannot do is perform the merge, inspect the class, or route around the resolution it was handed.

An operator asserts a merge directly, and that assertion is within the operator's authority by definition. The guards that constrain agent-proposed merges do not constrain it.
