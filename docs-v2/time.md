# Time

Time belongs to different objects for different reasons. The [object model](statements.md) owns object identity and lifecycle. This chapter defines temporal value types and the policies that interpret them.

- An Assertion carries validity: when its Proposition applies in the modelled world.
- An Occasion carries observed time for the external input event, from connector or operator evidence, and recorded time for its entry into the log.
- A testimony Attestation carries the time of the telling within its Occasion when finer precision is available.
- A [Perception](artefacts-and-perceptions.md) carries the time its Activity observed, which may differ from the Artefact's creation or sharing time, and its recorded time.
- Event occurrence is an attribute Assertion about an [Event](events-and-roles.md). Event identity owns no occurrence field.
- A Task carries one or more trigger conditions.

These times are not interchangeable.

## Typed temporal values

Temporal values are typed from genesis. Strings exist only at input and rendering boundaries. Every value records its precision, uncertainty, and timezone ownership rather than manufacturing exactness.

| Type | Semantics |
|---|---|
| `CivilDate` | A calendar date with its calendar and no implied instant. |
| `LocalDateTime` | A civil date and time whose timezone is unknown or owned by a named person, place, connector, or policy. |
| `ZonedDateTime` | A civil date and time with an IANA zone and calendar. It resolves to an instant under a named timezone database version. |
| `Instant` | An absolute timeline point. |
| `Interval` | Two bounds, each open, inclusive, exclusive, unknown, or Event-bounded, with precision and uncertainty. |
| `Duration` | A fixed elapsed quantity, where that interpretation is valid. |
| `CalendarSpan` | Years, months, weeks, or days, resolved against an anchor and a calendar. |
| `EventBound` | `before`, `after`, or `during` a named Event, resolved from that Event's occurrence. |
| `RecurrenceIntent` | A typed recurrence with explicit ambiguity and exceptional-date policies. |

Precision covers at least year, month, day, minute, second, and exact instant. Uncertainty is an explicit bound, a qualitative estimate, or unknown. Timezone ownership distinguishes "09:00 in Rowan's current zone" from "09:00 Europe/Stockholm" and records the policy that resolves the former. A later timezone or database change produces a new projection. It never alters the original value.

Civil and absolute time stay distinct. A birthday is a civil date, not an instant. Calendar spans are anchor-aware: adding one month to 31 January invokes the recorded month policy rather than becoming 30 days. These rules follow established temporal-database and date-time-library practice ([research lane](research/2026-07-24/lanes/time-memory.md#1-bitemporal-and-tri-temporal-database-theory-vs-zuihitsus-assertedoccurred-split); [typed-value research](research/2026-07-24/lanes/time-memory.md#3-typed-temporal-values-at-an-llm-interface-103)). Typed quantities also appear in the corpus as prose, in the position dates once occupied.

## Validity, observation, and recording

An Assertion's validity is an interval over its Proposition. It answers when the asserted content applies. Repeated disjoint periods are separate Assertions over the same Proposition. A changed interpretation of validity appends a correction or supersession transition. It never edits the source Occasion or Attestation.

Delayed ingestion can therefore represent a document authored in 2019, shared in 2025, perceived in 2026, and recorded later, without treating any of those times as validity. When an utterance does not establish validity, the validity stays open or unknown. The system never dates an Assertion by the day it was heard.

Temporal databases support separating validity from transaction time. The additional observed-time distinctions and their assignment to these objects are this design's synthesis ([research lane](research/2026-07-24/lanes/time-memory.md#the-canonical-two-axes)). Validity intervals closed by later window-closing are convergent across temporal databases and every surveyed production graph.

## Event-bounded validity

Not all time has a date. A validity bound may name an Event instead of a value:

- `before(E)`: the validity ends no later than the start of E's occurrence;
- `after(E)`: the validity starts no earlier than the end of E's occurrence;
- `during(E)`: the validity lies within E's occurrence.

"Lived in Leeds before the move" is an Assertion whose validity end is `before(event/move)`. The bound resolves at read time from E's audience-safe occurrence. Until that occurrence is known, the bound renders as the Event reference and is treated as unknown for temporal filtering. When the occurrence becomes known, or is later corrected, the resolved bound follows it without any write to the bounded Assertion. When E is suppressed for the audience, the bound renders as unknown, so a hidden Event never leaks through a date.

Richer qualitative reasoning is deferred: `overlaps`, `meets`, `equals`, bounds relative to other Assertions, and composition across chains of relations. The maximal tractable subclass of Allen's interval algebra is a settled result, and whether a smaller subset suffices is open ([research lane](research/2026-07-24/lanes/time-memory.md#the-missing-piece-3-allens-interval-algebra-for-qualitative-anchoring); [verification](research/2026-07-24/verification/part-b.md#verdict-table)). Adding it later is additive: a new bound variant and new projections, with no change to stored validity. Unknown or ambiguous anchoring stays explicit rather than being replaced by an invented timestamp.

## Occurrence and Task

Occurrence and action are mechanically separate from genesis.

An Event occurrence Assertion describes when a happening occurs, and it never fires. A [Task](statements.md#task) is an agent-authored action intent with one or more trigger conditions. The genesis condition kinds are a point in time (a temporal value that resolves to an instant) and a `RecurrenceIntent`. Further condition kinds are additive.

The genesis action vocabulary has one action: `wake_turn(conversation, note)`. When a trigger condition is due, the scheduler starts an ordinary agent turn in the named conversation and renders the note into that turn's context. A due trigger is therefore an initiating event at genesis, and reminders work. A reply to the conversation's audience is an outbound [Occasion](statements.md#occasion) and passes the [pre-delivery check](privacy-and-provenance.md#pre-delivery-check) like any other. No inbound message starts the turn, so it cannot use challenge-response in the moment: its audience resolves from clearances already recorded. Further actions are additive definitions. Initiation without a Task is a separate capability, deferred as [proactive initiation](off-turn.md#deferred-proactive-initiation).

A `wake_turn` note can carry restricted content, so it is checked twice:

- At creation, the Task critic rejects a `wake_turn` whose conversation does not exist, whose note is empty, or whose note or arguments carry a restriction that does not admit the target conversation's current audience.
- At firing, only content that clears the conversation's audience at that moment is rendered. A note that no longer clears, for example because a member joined, is replaced by a non-disclosing marker, and the failure is recorded for the operator. The turn still starts.

Activation requires the authority the Task names, checked by the Task critic in the [verified write](verified-write.md#hard-critics). A trigger condition fires only while its Task is `active`. Each firing is recorded per condition, keyed by the Task, the condition, and the scheduled instant, so a repeated firing of the same instant is idempotent. A one-shot Task completes when its action completes. A recurring Task stays active until it is cancelled or its recurrence ends. A Task never targets an Assertion or Event as its trigger. The [off-turn](off-turn.md#scheduling) scheduler drains due trigger conditions before maintenance.

The current system's overloaded occurrence field produced the observed failure: a third party's recurring routine woke the agent. iCalendar's mature separation of VEVENT, VTODO, and VALARM establishes the solution shape. No surveyed peer has this failure, so the problem may be local; the fix is sound regardless, and the exact successor records are a local design decision ([current failure](../docs/ontology-failures/2026-07-23.md#schedule-and-description-conflate-in-the-temporal-model); [research lane](research/2026-07-24/lanes/time-memory.md#2-the-schedule-vs-description-conflation-failure-class-4); [verification](research/2026-07-24/verification/part-b.md#verdict-table)). The separation exists at genesis because, added later, it would leave old descriptive recurrence able to fire.

## Modality

Time does not encode modality. The [object model](statements.md#modality) owns the modality axis and its genesis values. Their temporal readings are:

- A `planned` Event does not become `actual` because its date passes. It keeps rendering as planned until an Assertion reports otherwise.
- Event cancellation is a new Assertion with `cancelled` modality and its own provenance. The planned Assertion is not mutated.
- Task cancellation is a Task lifecycle transition and is never inferred from Proposition modality. Third-party `deontic` content never causes action.
- `habitual` is a disposition or recurring pattern, not a claim about every instant, and it never fires.

The initial policy treats non-actual modalities opaquely except for rendering and scheduling guards. Habitual and deontic inference is deferred.

## Recurrence intent

A recurrence is built from typed intent, never accepted as an unchecked RRULE string. The value records frequency, interval, anchors, bounds, timezone ownership, calendar, and explicit policies for exceptional dates:

- `last_day`: the final civil day of each target month;
- `skip`: omit periods in which the requested civil value does not exist;
- `clamp`: use the final valid civil value in the period;
- `business_adjust(calendar, direction)`: move according to a named, versioned business calendar;
- an explicit gap and fold policy for daylight-saving transitions.

"Last day of every month" is valid intent and compiles to `last_day`. "Monthly on the 31st" is rejected as ambiguous unless the writer chooses `skip`, `clamp`, or `last_day`. Impossible civil dates are never silently rolled. A business adjustment without a calendar version is invalid, and business-calendar adjustment as a capability is deferred. These policies keep ordinary intent while guarding the RRULE pathologies the research identifies ([research lane](research/2026-07-24/lanes/time-memory.md#rrule-pathologies-to-guard-at-the-typed-boundary)).

## Correction and world change

A temporal correction changes an interpretation, not the source. Suppose an Occasion contains "we shipped on Tuesday" and an Assertion resolves validity or occurrence to the wrong Tuesday. The correction appends evidence for the corrected value, a supersession transition on the old Assertion, and a new Assertion with the corrected value, supported by a [derivation Attestation](statements.md#attestation) over the temporal parser Activity and the source locator. The Occasion, the utterance span, the testimony Attestation, and the old Assertion remain. When no corrected value is supported, withdrawal without substitution is valid. The current system's cue and withdrawal work shows why evidence is requested and why unsupported substitution is unsafe ([current-system evidence](research/2026-08-06/current-system-fixes.md#span-justification)).

A world change is different. If someone changes employer, the old Assertion's validity is closed with a cause and a new Assertion is added. If the old employer was recorded incorrectly, the old Assertion is superseded or retracted. Both are append-only; their transition reasons differ.

## Staleness and volatility

An open validity interval does not imply permanent truth. Volatility is a registered property of relation or Assertion policy, with expected review horizons. Passing a horizon never mutates the source or silently closes its interval. It creates a candidate Activity that may append a bounded replacement or a closure with evidence, mark the Assertion stale under a named policy, queue re-verification, or leave the Assertion unchanged when evidence is insufficient. Policy changes rebuild the stale projection. They never make historical records mean something new. Volatility automation is deferred, together with its `validity_boundary_due` queue mark. At genesis, passing a horizon changes nothing.

## Genesis boundary

Required at genesis: typed values with precision, uncertainty, and timezone ownership; validity on Assertions, including Event bounds; source observation and recording times; occurrence as Event-attribute Assertions; the modality axis; and Tasks with trigger conditions and the `wake_turn` action, separate from occurrence.

Initial policy: conservative rendering of actual, planned, and cancelled; no firing from descriptive occurrence; explicit safe recurrence constructors; and source-preserving correction.

Deferred ([evolution](program/evolution.md#deferred-capabilities)): richer qualitative temporal inference, business-calendar adjustment, volatility automation, habitual and deontic inference, and autonomous recurrence interpretation. Their raw inputs exist from genesis, so enabling one changes projections and automation, not persisted meaning.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `dated-description-no-fire` | A participant describes another system's nightly backup job. | A habitual occurrence Assertion; no Task; nothing enters the scheduler. |
| `birth-date-no-fire` | A participant states their birth date. | An Event occurrence with a `CivilDate`; nothing fires on the anniversary. |
| `task-multi-condition` | The agent creates one Task with a point-in-time condition and a weekly recurrence. | Each condition fires on its own schedule, with one firing record per condition and instant. |
| `task-cancelled-no-fire` | A Task is cancelled before its due instant. | No firing record exists for the condition, and no turn starts. |
| `trigger-wakes-turn` | Rowan asks for a reminder on Friday at 09:00; the agent creates an active Task with `wake_turn(rowan-dm, "remind Rowan about the dentist")`. | At 09:00 the firing is recorded and an ordinary turn starts in that conversation with the note in its context manifest. The reply is delivered to Rowan as an outbound Occasion after the pre-delivery check. |
| `wake-turn-note-audience-at-creation` | In Quinn's DM, the agent creates a `wake_turn` for a group channel whose note repeats Quinn's confidence. | The Task critic rejects the Task with a teachable error. No Task is recorded as active. |
| `wake-turn-note-no-longer-clears` | A `wake_turn` note cleared its channel at creation; before it fires, a new member joins the channel. | The turn starts with a non-disclosing marker in place of the note. The note's content is absent from the manifest, and a failure record for the operator names the Task. |
| `task-duplicate-firing` | The scheduler attempts the same condition and instant twice. | One firing is recorded. |
| `planned-not-actual` | A planned Event's date passes with no report. | Reads return the Assertion with `planned` modality; no `actual` Assertion is created. |
| `event-cancellation` | A participant says the planned trip is off. | A new `cancelled` Assertion; the planned Assertion is unchanged; no Task transition is inferred. |
| `event-bounded-validity` | "I lived in Leeds before the move", with the move's date unknown, then later stated. | The validity end renders as "before the move", then resolves to the stated date with no write to the Leeds Assertion. |
| `event-bound-hidden-event` | A bound names an Event suppressed for the current audience. | The bound renders as unknown; the Event is not revealed. |
| `delayed-ingestion-times` | A 2019 document is shared in 2025 and perceived in 2026. | Occasion, Perception, and recorded times are distinct; none becomes the Assertion's validity. |
| `undated-utterance` | A fact is stated with no temporal cue. | Validity is open or unknown, not the Occasion date. |
| `timezone-owner` | "09:00 my time" from a participant whose zone later changes. | The stored value names the owner; the projection changes, the value does not. |
| `month-end-span` | One calendar month is added to 31 January. | The recorded month policy decides; the result is not 30 days later. |
| `monthly-31st-ambiguous` | "Monthly on the 31st" with no policy, then "last day of every month". | The first is rejected with a teachable error offering `skip`, `clamp`, or `last_day`; the second compiles to `last_day`. |
| `business-adjust-needs-calendar` | A business-day adjustment with no calendar version. | Rejected. |
| `temporal-correction` | "We shipped on Tuesday" was resolved to the wrong Tuesday. | Supersession and a replacement Assertion with evidence; the source bytes and old IDs remain. |
| `world-change-vs-correction` | A job change, then a separately discovered mistaken employer. | The first closes validity with a cause; the second supersedes or retracts. |
| `volatility-horizon` | An Assertion passes its review horizon. | At genesis nothing is appended. The Assertion's validity stays open, and its source is unchanged. |
