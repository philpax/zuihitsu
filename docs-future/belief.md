# Belief

Every [Statement](statements.md) carries a credence. It is derived from counting evidence, and never from a model stating a number.

## Why not ask the model

Verbalised confidence is systematically overconfident, saturates to a handful of coarse values regardless of the underlying uncertainty, and shifts with how the question is asked. A number obtained that way looks like a measurement and is closer to a stylistic tic. Storing it would launder a wording artifact into a quantity that downstream mechanisms then treat as evidence.

Where a judgement genuinely requires the model, it is quantised to a coarse ordinal: **suspected**, **likely**, **confirmed**. That is roughly the granularity a model can actually sustain, and it is honest about being a judgement rather than a measurement.

## Credence comes from tellers

The inputs are the number of tellers who stand behind a claim, their independence, and how reliable each has been.

```
s14 (person/quill, persona_of, person/ferrer)
    told_by   person/wren, person/rowan
    credence  confirmed · two independent tellers
```

A claim first recorded as a hedge and later corroborated is **one Statement whose credence moved**, not two entries whose relationship is invisible. The current system produces the latter: [the corpus study](research/2026-08-03/modelling-study.md) found a claim recorded as "likely" and, later, the same claim asserted flatly, both alive at once with nothing connecting them and no way to tell that the second was the first, confirmed.

The representation separates strength of belief from amount of evidence, which is the distinction a single probability collapses. "One unreliable person said this once" and "it is genuinely uncertain whether this is true" are different states and must not compare equal.

## Dependent evidence produces no gain

Two tellers repeating each other are not two pieces of evidence.

This is the same soundness problem that makes attribute overlap useless for [identity](identity.md), and it has the same answer: dependence is a **provenance determination**, not a judgement. If two attestations trace to a common source, or one teller was present when the other learned it, they are dependent, and dependent evidence adds nothing.

The rule is deliberately conservative. Detecting dependence is the load-bearing part; the arithmetic applied afterwards is a no-op in the dependent case, which is trivially sound. Nothing here relies on a contested fusion operator, and [`confidence.md`](confidence.md) records why that restraint is deliberate.

Trust discounting is the other operator that does real work: an attestation from a partially reliable teller contributes proportionally less, with the shortfall becoming uncertainty rather than disbelief. Being told something by someone unreliable is not evidence against it.

## Revision is not prioritised

New information does not automatically win.

A claim from a low-credibility teller does not overturn a well-corroborated one. It is recorded, it accrues its own credence, and if it accumulates support the balance shifts. A teller is not always right, and a store that assumes the most recent assertion is the true one is a store that can be walked anywhere by whoever speaks last.

Contradiction is therefore a **state**, not an incident. Two Statements that cannot both hold coexist, each with its own credence and evidence, and the read surface shows both with their support. This replaces the current model, where an arbitration pass writes a prose note about the conflict and the conflict persists as a flag.

Where contradiction resolves, it resolves by one side accumulating evidence or by a validity interval closing, both of which are ordinary operations. Nothing needs to be deleted for the store to stop believing something.

## What the agent sees

The surface stays ordinal and anchored to its evidence:

> three people told me this, one of whom is usually unreliable

rather than:

> confidence 0.72

The opinion lives in the substrate. What surfaces is a coarse ordinal with the evidence attached, because that is what supports a decision about whether to act on a claim or go and check it. A number the agent cannot interrogate invites exactly the false precision that verbalised confidence already suffers from.
