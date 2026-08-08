# Time

The design keeps three axes apart. Conflating any two of them is what makes an agent wake up for someone else's calendar.

- Occurrence: when the thing a claim describes happens in the world.
- Task: something the agent itself is meant to do, with a due time.
- Trigger: what actually fires, expressed relative to a Task.

A description has an occurrence and nothing else. Only a Trigger fires, and a Trigger only ever hangs off a Task the agent authored for itself. A fact about another system's nightly job, or the birth year of a historical figure, has an occurrence, no Task, no Trigger, and therefore no path to waking anything.

The current system has one occurrence field carrying two meanings, and the predicted failure is the observed one: a daily recurrence extracted from a fact describing a third party's routine, which then woke the agent at every boot until the occurrence was withdrawn by hand. The scheduler arms on the descriptive axis because there is no other axis to arm on.

## Bitemporal, plus decision time

Each [Statement](statements.md) carries three temporal coordinates, and they answer different questions.

| | |
|---|---|
| `valid` | the interval over which the claim holds in the world |
| `observed` | when the claim was made or the thing was noticed |
| `recorded` | when the store learned it |

Separating `observed` from `recorded` is what makes delayed ingestion coherent: a document written years ago and read today records both, and neither is a lie.

It also relieves a specific pressure. A claim whose utterance anchors no time leaves `valid` open, and the store still knows when it was learned, because the occasion is held by the [episode](memory-typology.md). The current system, lacking that, pushes the model to date claims by the day it heard them, and it does exactly that: an undated past event stamped with the day it was mentioned, then relayed the next day as though it had happened yesterday, with the correction costing two turns of the person's time.

## Typed values

Dates, durations, quantities, and recurrences are first-class typed values end to end. Strings appear only at the input boundary.

The distinction between civil and absolute time is honoured rather than flattened. A day is a civil date; a moment is an instant. Treating a birthday as an instant produces the timezone bugs that make an anniversary land on the wrong day for half the world.

Durations are anchor-aware. A month resolves against the date it is measured from, so "three months from January 31" has one correct answer rather than a fixed 90-day approximation.

Recurrences are built through constructors rather than parsed from strings. The constructors cannot express the pathological cases, so a monthly recurrence anchored on the 31st is unrepresentable rather than silently skipping February. What cannot be constructed cannot be stored.

The corpus study found quantities in the same position dates once were: written into prose with their units, uninterpretable to any query. Word counts, elapsed hours, and ratings all appear as text. They are typed for the same reason dates are.

## Qualitative anchoring

Not everything has a date, and forcing one manufactures precision.

Real recorded language anchors relatively far more often than absolutely: something happened before something else, during a period, a few weeks after a thing that itself has no date. The store holds these as interval relations rather than discarding them or inventing timestamps.

The relations available are a tractable subset: before, after, during, overlaps, meets, and equals, with a composition table so orderings can be inferred transitively. Full interval reasoning is intractable, and the maximal tractable subclass containing all the basic relations is a settled result rather than a design choice.

This matters for more than tidiness. The dual-trace evidence attributes its largest gain to temporal reasoning, and the mechanism it reports is narrative anchors of exactly this kind: a detail placing one thing relative to another, with no date anywhere. A model that only accepts absolute times cannot record what people actually say.

## Correcting a date

Changing when something happened is a first-class operation on the occurrence, not a rewrite of the claim.

The current system offers only full replacement, and the cost is visible in the log: asked to fix one date, the agent looped over entries guessing at the stored format, then rewrote an entire entry's text to move the date, discarding and re-authoring content it had no reason to touch.

Under this model the occurrence is a field on a Statement whose text is a separate concern. Correcting it touches the occurrence. The [gloss](two-traces.md) is untouched, because the person's words did not change: only the store's reading of them did.

## Staleness and volatility

A claim with an open validity interval is not automatically true forever.

Volatility is a property of the claim's kind rather than a flag someone remembers to set. Where a claim is inherently transient, its interval carries an expected horizon, and passing that horizon makes it a candidate for revisiting rather than a fact to keep asserting.

A retroactive pass over aged claims chooses from a ladder. Temporalise where the claim is sound but untimed, closing its window at the date it can be shown to have held. Annotate where it is plausibly still current but unverified. Re-verify by queueing it for the next contact with someone who would know. Retire where it has no temporal reading at all.

Temporalising is the common case and the one that resolves cleanly. Most aged claims are not false, they are undated: an activity someone was doing, a plan someone had. Closing the window turns a claim that reads as current into dated history, which is permanent and no longer decays.
