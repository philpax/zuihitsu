# Fixes landed in the current system, 2026-08-06

Evidence gathered by the running system rather than by a research lane. On 2026-08-06 a wave of agent-behaviour work merged into the current system ([PR #122](https://github.com/philpax/zuihitsu/pull/122)), closing #113, #125, and #126. It is recorded here because three of its findings bear on the design in [`../../`](../../): one supplies a mechanism the design lacked, one supplies a rule, and one supplies an instance of a failure the design claims to close.

The figures come from the eval harness, not from the event log, so they describe scenario runs rather than the live instance. Where a count is suite-wide it aggregates every scenario in the run.

## Span justification

The temporal extraction now emits a `cue` beside every occurrence: the words, copied from the statement the occurrence keys, that the time was read from. The cue is matched against that statement's own text, folded so case, whitespace, and edge punctuation do not matter, and an occurrence whose cue is absent from the statement is dropped.

Paired with a current-day guard that suppresses a resolution landing on today when the statement names no time of its own, the two cut dated occurrences suite-wide from 188 to 112, roughly 40%, with no date-dependent gate regressing.

Three properties are worth carrying forward.

It asks the model for evidence rather than for a judgement. A fabricated date needs a fabricated quote to carry it, and whether a quote appears in a given text is decidable without a model.

It applies to every resolution rather than to a suspicious subset, so a date read from a neighbouring statement or from nowhere is caught on the same rule as a date read from the clock.

It is not airtight. A cue can be quoted faithfully and still not denote a time, which is why the day-shaped heuristics still run behind it. Those heuristics are explicitly a heuristic over text rather than a property of it, and the current system accepts false suppressions in both directions to get them.

## Withdrawal without substitution

An occurrence the agent wrote at append time was previously never re-examined, only never clobbered. Across 405 runs, 334 appends carried an authored date against 112 the extraction resolved, and only the latter passed any check. A date lifted from a namesake therefore stood unchallenged whenever the agent wrote it directly.

The extraction pass now sees timed entries written in the same turn and may report one as misdated, at which point the occurrence is withdrawn and the entry returns to untimed. Withdrawal is the only available outcome. Substitution stays forbidden, because clobbering an authored date is the failure that made authored dates sacrosanct, and because the two errors do not cost the same: withdrawing a wrong date disarms a wake-up, where substituting one arms a different wrong one.

## Redaction decided per read path

Three visibility leaks were found in the same wave, each by a different route:

- A by-id entry handback returned a withheld entry with its `withheld` marker hard-coded false.
- An ambiguous-entry-prefix error listed candidate entries by their text.
- A visible link row carried an occurrence read off a withheld entry.

The third is the informative one. No text was involved, so any fix framed as redacting text would have missed it. The current system's conclusion is that a date is disclosure on its own terms: "something on the 16th" is a fact about a person the reader was not cleared for, even with its words stripped. The underlying cause is that redaction is decided independently at each read path with nothing requiring a path to ask, which is now filed as #127 behind #123.

## Drift invisible under green runs

A scenario fell from 1.00 to 0.20 and read as a regression from the branch under test. Three n=20 arms, at HEAD, at one commit reverted, and at the code from before the branch began, all landed near 0.65 with p = 0.74 between them, against a historical rate of 0.92. The cause was a namespace addition eight days earlier, whose interaction with a scenario fixture made correct modelling score as a miss.

It went unnoticed because the first run after the change came up 5/5, which the prior rate explains without difficulty. The response was per-criterion drift detection that pools trailing runs rather than judging each alone, and brackets a flagged criterion to a commit range through the `git_sha` on each history line. Rises are flagged as readily as falls, because a criterion that suddenly cannot fail has usually stopped exercising the path it names.
