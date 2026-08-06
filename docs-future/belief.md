# Belief

Every [Statement](statements.md) carries a credence. The credence is derived from counting evidence, and never from a model stating a number.

## Model-stated confidence

Verbalised confidence is systematically overconfident, saturates to a handful of coarse values regardless of the underlying uncertainty, and shifts with how the question is asked. A number obtained that way looks like a measurement and is closer to a stylistic tic. Storing it would launder a wording artefact into a quantity that downstream mechanisms then treat as evidence.

Where a judgement genuinely requires the model, it is quantised to a coarse ordinal: suspected, likely, confirmed. That is roughly the granularity a model can sustain, and it is honest about being a judgement rather than a measurement.

## Credence comes from tellers

The inputs are the number of tellers who stand behind a claim, their independence, and how reliable each has been.

```
s14 (person/quill, persona_of, person/ferrer)
    told_by   person/wren, person/rowan
    credence  confirmed · two independent tellers
```

A claim first recorded as a hedge and later asserted flatly is one Statement with two tellings, not two entries whose relationship is invisible. The current system produces the latter: [the corpus study](research/2026-08-03/modelling-study.md) found a claim recorded as "likely" and, later, the same claim asserted flatly, both alive at once with nothing connecting them and no way to tell that the second was the first, firmed up.

## A hedge is not a credence

The two are routinely conflated and they are different objects. Credence is the store's, derived from how many independent tellers stand behind a claim. A hedge is the teller's, and it belongs to the act of telling, which is why it rides as [the `expressed` qualifier](statements.md) on the telling rather than moving anything.

The consequence contradicts a natural reading of the previous section, so it is stated directly. When the same teller says "probably X" and later says "X", the credence does not move. There is still one teller and no corroboration, so nothing about the evidence has changed. What changed is how firmly that person put it, and that is recorded where it happened.

This is not a technicality. The showcase case in the live corpus is exactly this shape: one teller, hedged, then the same teller, flat, with no second source anywhere in the arc. A design that moved credence there would be inferring corroboration from emphasis, which is the verbalised-confidence failure wearing a human face rather than a model's. The same rule kills the neighbouring error: two independent tellers each saying "probably" are two pieces of evidence for a claim neither committed to, and their `expressed` qualifiers are what keep that visible instead of rounding it to `confirmed · two independent tellers`.

What a firming-up earns is a surface that reads honestly: two tellings, the second unhedged, from someone who was unsure and now is not. That is more informative than a credence tick, and it is what an agent deciding whether to act on a claim requires.

The representation separates strength of belief from amount of evidence, which is the distinction a single probability collapses. "One unreliable person said this once" and "it is genuinely uncertain whether this is true" are different states and must not compare equal.

## Dependent evidence produces no gain

Two tellers repeating each other are not two pieces of evidence.

This is the same soundness problem that makes attribute overlap useless for [identity](identity.md), and it has the same answer: dependence is a provenance determination, not a judgement. If two attestations trace to a common source, or one teller was present when the other learned it, they are dependent, and dependent evidence adds nothing.

Both conditions are read from the record rather than inferred. The derivation graph carries the first. The gloss's exposure set carries the second, which is the wider of [the two witness sets](privacy-and-provenance.md) precisely because over-counting exposure only suppresses corroboration.

A third path runs through the agent itself, and it needs the same treatment. The agent is a witness to everything told to it and never an independent teller of what it was told, so re-recording a claim in its own words adds an occasion rather than a source. Because the agent's own utterances are glosses whose witnesses are their recipients, a relay is visible: someone who learned a fact from the agent and later tells it back is a distinct teller whose evidence traces to the original. Dependence through the agent is the commonest kind in a store the agent is constantly reading back to people, and it is detectable only because the relay left a record.

In a shared channel this is the ordinary case rather than a corner one. Everyone present hears everything, so three people repeating what a fourth said in the room is one piece of evidence, and a credence that counted three would be badly wrong in the direction that matters. Sociality is what makes corroboration meaningful and, in the same stroke, what makes dependence detection load-bearing.

The rule is deliberately conservative. Detecting dependence is the load-bearing part. The arithmetic applied afterwards is a no-op in the dependent case, which is trivially sound. Nothing here relies on a contested fusion operator, and [`confidence.md`](confidence.md) records why that restraint is deliberate.

Trust discounting is the other operator that does real work: an attestation from a partially reliable teller contributes proportionally less, with the shortfall becoming uncertainty rather than disbelief. Being told something by someone unreliable is not evidence against it.

## Revision is not prioritised

New information does not automatically win.

A claim from a low-credibility teller does not overturn a well-corroborated one. It is recorded, it accrues its own credence, and if it accumulates support the balance shifts. A teller is not always right, and a store that assumes the most recent assertion is the true one is a store that can be walked anywhere by whoever speaks last.

Contradiction is therefore a state, not an incident. Two Statements that cannot both hold coexist, each with its own credence and evidence, and the read surface shows both with their support. This replaces the current model, where an arbitration pass writes a prose note about the conflict and the conflict persists as a flag.

Where contradiction resolves, it resolves by one side accumulating evidence or by a validity interval closing, both of which are ordinary operations. Nothing needs to be deleted for the store to stop believing something.

## The agent-facing surface

The surface stays ordinal and anchored to its evidence:

> three people told me this, one of whom is usually unreliable

rather than:

> confidence 0.72

The opinion lives in the substrate. What surfaces is a coarse ordinal with the evidence attached, because that is what supports a decision about whether to act on a claim or go and check it. A number the agent cannot interrogate invites exactly the false precision that verbalised confidence already suffers from.

## The belief is absolute; the account of it is not

A claim's credence is computed once, over every teller, and does not vary by room. What varies is the evidence account rendered beside it, which is filtered to the tellers the present audience may learn of.

The alternative fails on both horns. Rendering the full count reveals that an undisclosable endorser exists, which is a confidence leaking in aggregate. Recomputing credence per room makes the store's belief room-dependent, so a derivation computed in one conversation rests on a different credence than the same derivation in another, and the read paths diverge in exactly the way computing visibility once in the substrate exists to prevent.

Separating the two keeps both properties: one belief, many accounts of it. Where the only thing distinguishing one room's account from another's is the existence of an endorser who cannot be named, the ordinal surfaces alone with no account at all, which is the [zero-residue](privacy-and-provenance.md) standard applied here rather than a special case. The agent sounds vaguer than it strictly needs to on occasion, which is the correct direction to fail.
