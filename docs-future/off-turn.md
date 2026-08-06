# Off-turn work

The store changes when nobody is talking. Consolidation, re-derivation after a severance, episode composition, structuring retries, decay, wake-ups, exploration, and whatever the agent decides to do on its own initiative all run outside a turn.

None of it is a separate write path. Off-turn work inherits every commitment the on-turn path carries, and two of those bite harder here, because there is no participant present to notice a mistake.

## A pass is an ordinary writer

Every pass crosses the same typed [seam](the-seam.md), meets the same critics, and can be rejected. There is no maintenance bypass, no privileged verb, and no pass-only authority.

Four rules follow, and each closes a way a pass could otherwise launder something.

**A pass writes as the agent.** It is never a teller on someone's behalf, and its output is a derived [Statement](statements.md) carrying `derived_from`, its activity, its criterion, and its assumption stamp. "How do you know?" answers for pass-written content exactly as it answers for anything else, which is the difference between curation and accretion.

**A pass may not widen an audience.** A synthesis carries no more than the intersection of what it drew on, a description is restricted further to public content because it cannot name a teller, and no pass extends a [disclosure set](privacy-and-provenance.md). An audience is widened by evidence arriving, never by tidying.

**A pass may not cross the [episodic wall](two-traces.md).** An episode is not a premise, and no amount of off-turn deliberation promotes a reconstruction into a fact.

**A pass may not touch the [self slot](memory-typology.md).** The one piece of state that conditions every turn is out of reach of the machinery that decides what to drop.

## The work list replaces the sweep

A pass that sweeps the store re-reads everything on every run, so its cost per fact is constant at best and rising in practice. That is the graveyard lesson in [`lineage.md`](lineage.md), and it decides the shape of off-turn work: passes drain queues that write-time marks fill.

The marks already exist, or are cheap consequences of things that do:

| | |
|---|---|
| **owing recomputation** | a derivation whose premises gained support since it ran |
| **support weakened** | a derivation whose premises lost credence, through unreliability or discovered dependence |
| **voided** | a derivation stamped with a withdrawn merge, awaiting re-derivation |
| **window closed** | a Statement whose validity interval ended, awaiting whatever succeeds it |
| **gloss-only** | an utterance committed without structure after a structuring failure, retried once |
| **contested** | a contradiction pair coexisting, awaiting evidence or a person |
| **candidate** | a merge or a resolution below its threshold, awaiting corroboration |
| **pending structure** | a span of an [ingested document](memory-typology.md) committed to the source layer, kept off every read surface until its job completes |
| **unreflected** | a working note awaiting the promote-or-discard pass |
| **unsummarised** | a closed session that met the bar for an episode and has not been composed |

A tick that finds empty queues costs a queue read. A tick that finds work does exactly that work, and the cost is proportional to what changed rather than to what is stored.

Whole-store sweeps do not disappear, but they stop being the routine path. The [drift](the-seam.md) machinery, canaries and re-derivation audits, is inherently whole-store, and it is a diagnostic run rarely rather than curation run constantly. Confusing the two is how a maintenance budget becomes a scaling problem.

## The heartbeat drains, it does not think

The timer is a scheduler, not a deliberator. What it runs splits cleanly by whether a model is involved.

**Mechanical work** calls nothing: closing windows, propagating a void, resolving aliases, updating access-recency ranking, expiring a candidate. It is deterministic, cheap, and can run on any tick.

**Judgement work** calls a model: synthesis, weighing a contradiction, composing an episode, retrying a structuring that failed. Each is a record-at-call-time activity whose prompt and response land in the log, so every one of these is permanent log volume as well as a model call. It is metered per tick rather than drained to empty.

Wake-ups ride the same timer and are not maintenance. A [trigger](time.md) is a commitment the agent made, and a queue of tidying must never starve one. Triggers are drained first, and a tick that runs out of budget drops maintenance rather than deferring a commitment.

## Consolidation gets smaller

Most of what consolidation does today is recovering structure that was never captured, and capturing it at write time takes the work away rather than moving it.

**Structural deduplication disappears.** Same claim, same frame, same validity interval is one Statement at write time. There is nothing for a pass to detect afterwards, and the similarity threshold that currently decides it stops existing.

**Cross-audience merging disappears.** Today a narrower entry is retired into a wider one because a fact recorded by two tellers is two entries. Under this model it is one Statement with two tellers, each carrying their own transmission principle and their own retraction authority, so the tier that reconciles them has nothing to reconcile.

**Near-miss resolution remains**, and is the genuine residue: the overlapping-but-not-agreeing [Event](events-and-roles.md) participant sets, alias-equivalent relations, and claims a critic flagged as candidates rather than resolving. These are judgements, they are queued as candidates, and an unresolved one reaches a person rather than being decided by a threshold.

**Distillation remains**, under the rule in [privacy and provenance](privacy-and-provenance.md): the intersection in general, public-only for a description, which cannot attribute what it summarises.

## Exploration

Every queue above is fed by a mark, and a mark is left when something changes. A store driven only by marks is structurally incapable of noticing that two things it has known separately for months are connected, because nothing changed to say so.

**Exploration** is the one pass with no mark behind it. It samples a pair of memories, asks what connects them, and keeps a note if the answer is worth anything.

Four constraints make it affordable and safe, and each of them is a rule the design already has rather than a new one.

**It runs on what is left over.** Exploration consumes the judgement budget remaining after the queues drain, never competing with them and never a reason to raise the budget.

This is the design's **one declared exception** to the falling-cost commitment, and it is worth being exact about what is excepted. Every other mechanism does work proportional to what changed, so its cost per fact falls as the store grows. Exploration does not: its cost is unrelated to what changed, and the space it samples grows with the store, which is the shape [the graveyard lesson](lineage.md) warns about. What bounds it is a fixed budget rather than a falling rate, so total cost stays flat while yield per unit of it declines. That is a weaker guarantee, taken deliberately, for the one mechanism that buys something no mark-driven pass can. It is metered by construction, switched off first under pressure, and [registered](confidence.md) as possibly not worth running at all.

**It samples structurally, not randomly.** Random pairing is the crude form of the idea, and a typed graph can do better: two memories with no path between them but neighbouring in embedding space, two Events sharing one participant and nothing else, a claim whose relation has no instances in a neighbourhood full of them. These are queries rather than guesses, and they are available precisely because [a fact stopped being a sentence](overview.md).

**Its output is a working note, never an utterance and never a settled claim.** An exploration writes into [the scratchpad](memory-typology.md), so it inherits promotion by reflection, a taint set, and the audience arithmetic that goes with them. It never speaks: what it produces is something the agent might later have a reason to say, not a reason to say it.

**It cannot promote itself.** A daydreamed link has exactly one signal behind it, the model that proposed it, and [agreement before promotion](the-seam.md) requires two independent ones. So an exploration's output stays a candidate until ordinary evidence arrives to corroborate it, and if none ever does it decays with the rest of the scratchpad. This is stricter than scoring the idea for novelty and keeping the good ones, and it is the correct strictness: a plausible connection between two true facts is exactly what a language model produces when there is no connection at all.

The privacy consequence needs stating, because the pairing sampler is the first mechanism here that deliberately reaches across memories with unrelated audiences. The most interesting pair is often the cross-person one, and that is also the disclosure hazard: pairing a confidence with a public fact yields a note that encodes the confidence. The [intersection rule](statements.md) covers it, the taint set carries it into promotion, and the sampler is not exempt from either. An exploration that pairs across audiences produces a note no wider than the narrower of them.

## Agent-initiated work

The hardest case, and the one this design constrains rather than settles.

An off-turn message is not a reply. The agent is choosing the audience rather than being handed one, which inverts the usual evaluation: instead of asking what may be said to the people present, it asks who this may be said to at all. The evaluation is fail-closed over the audience it proposes, and a message it cannot justify to that audience is not sent, rather than being sent in a softened form.

Three hard limits:

- **No off-turn disclosure across an identity boundary.** That path requires challenge-response, which requires a live counterpart. A pass cannot clear it.
- **An off-turn message is an ordinary occasion.** It produces a gloss, its witnesses are whoever it actually reached, and anything the agent asserts in it is a Statement told by the agent.
- **Initiative is exception-triggered, never ambient.** A wake-up fired, a commitment came due, a queue item needs a person. An agent that speaks because a timer fired and it had spare capacity is a drift generator with a schedule.

What is deliberately not settled here is what *deserves* initiation: the salience judgement that decides a thought is worth someone's attention. That question is older than this design and is recorded as open in [`confidence.md`](confidence.md). The constraints above hold whatever answer it eventually gets.

## What this does not change

The fold stays model-free. Every off-turn model call is a record-time activity, written to the log with its response, and replay consumes the record without calling anything. A pass that changed what the fold computes, rather than appending events the fold reads, would break the one commitment the whole design rests on.
