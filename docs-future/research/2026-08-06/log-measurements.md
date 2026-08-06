# Log measurements, 2026-08-06

Figures taken from the running instance's own event log, read read-only. They are cited throughout the design and were, until this note, recorded nowhere: several arrived through review passes and lived only in commit messages, which is not an audit trail.

## Method

Every figure comes from `zuihitsu debug events`, which reads the log without taking the write lock. Three commands were used: `--summary` for the session and type counts, `--type <PayloadType>` for a payload class, and `--seq N` for an individual event. No write-exception subcommand was run and the agent was not stopped.

The counts below are of the log as it stood on 2026-08-06. They will drift as the instance runs, and nothing in the design should depend on their exact values: what they support is the *shape* of the argument in each case, and where a design claim needs a precise figure it is stated as approximate.

## The log

| | |
|---|---|
| Events | 2,361 |
| Content entries | 198 |
| Lua blocks executed | 132 |
| Model calls recorded | 417 |
| Sessions | 27 |
| Conversations | 7, of which 1 is multi-party |
| Distinct human tellers | 3 |

## Payload composition

Recorded model calls are **95.9%** of payload bytes: 31.4 MB of 32.7 MB, at a mean of 75 KB per call over 417 calls. Content entries are 0.27% of payload.

This is the denominator behind three separate cost arguments in the design: that a scratchpad note is negligible beside the model call that wrote it, that a read event is negligible against the same, and that structuring adds a call per write block to a log already dominated by calls. All three are ratio arguments, so they hold as long as the composition does.

Independently corroborated: the on-disk `events.sqlite` is 33.8 MB, consistent with a 32.7 MB payload.

## Model-call latency

Over the same 417 calls: **p50 6.8 s, p90 30.8 s, p99 73.7 s, max 94.9 s.**

Cited by [`../../evolution.md`](../../evolution.md) stage 0c as the baseline any extraction latency is judged against, and by [`../../memory-typology.md`](../../memory-typology.md) in the arithmetic showing a long document cannot be ingested one span per call.

## Visibility and telling

**Posture across the 198 entries: 157 public, 40 attributed, 1 private-to-teller.**

This is the measurement that corrected the episodic composition rule from public-only to the intersection. Attributed content is a fifth of the corpus and is repeatable-with-attribution rather than withheld, so a public-only rule excluded a fifth of the corpus, concentrated in the two richest sessions, to protect a single entry.

**Teller across the same entries: 120 participant-told, 77 agent-told, 1 seed.**

The 77 include the agent restating in its own words what a participant had just said. Under teller-counting alone those read as a second, independent source, which is what forced the rule that the agent is a witness to what it is told and never an independent teller of it.

**No claim in the corpus is asserted by two distinct human tellers.** This is why the credence shape is deferred rather than blocking: with teller counts in zero or one, there is nothing for a fusion operator to do.

## What a block touches

Memories touched per executed block: **mean 1.69, median 1, p90 3, max 11.** Blocks per turn: mean 1.35, median 1, max 7. Union of memories touched across a whole turn: mean 2.03, median 2, max 11.

Ambient recall fired 147 times, surfacing 0 to 3 memories each, with 135 of the 147 surfacing between 1 and 3.

Read as a bound on taint-set size this is a **proxy and not a measurement of the thing itself**: the running system has no scratchpad, so these are block reads rather than note consultations. What they support is that the sets are unlikely to be large, not that the mechanism is affordable.

Brief sizes over the same period ran from 2,240 to 8,106 characters, which is the basis for excluding brief content from a taint set: it is kilobytes of memory content entering context with no read event behind it.

## Derived structure

| | |
|---|---|
| `LinksInferred` | 133 |
| `LinkCreated` | 92 |
| Merges ever proposed | 2 |
| `EntriesConsolidated` | 27 |
| `BeliefArbitrated` | 13 |
| `EntryAttested` | 9 |

The first three support the claim that severance re-derivation is small at present scale: the entire derived population is in the low hundreds, and no merge has ever been withdrawn.

The consolidation and arbitration events are more useful than their counts suggest. Each names the entries the running system itself judged to be about one claim, which makes them a **labelled re-mention set the instance produced by accident**, and the cheapest available gold data for stage 0c's convergence measurement.

## Duplication

Sixteen entries are exact textual duplicates of another entry; fourteen of those are connector-minted boilerplate. Twelve distinct (memory, text) pairs account for 21 duplicate copies in total.

The design cites this for what structural equality collapses. The caveat matters more than the figure: because most exact duplicates are boilerplate, this set does *not* test whether an extractor converges on the same triple from differently-worded restatements, which is the assumption structural equality actually rests on.

## Metalinguistic content

**Four of the 198 entries** are claims about a specific past utterance: grading a line the agent produced, conceding one phrase of it and disputing another, acknowledging a particular turn of phrase, and characterising how a document described someone. One of the four carries three such claims in a single entry.

Two percent is small. It is cited because the shape has no representation at all without a gloss reference in the object slot, not because it is common.

## An estimate, not a measurement

The read-event volume figure in [`../../query-surface.md`](../../query-surface.md) is derived rather than observed, and should be read as arithmetic on the numbers above: roughly 143 read-shaped calls across 132 blocks plus 147 ambient recalls is about 300 read events over the log's life, at a couple of hundred bytes each, so under 0.2% of payload and roughly a tenth added to the event count. The running system appends no read events, so there is nothing to measure; the claim is that the decision to append them is affordable, and it rests on the payload composition above.
