# Time

Time belongs to different objects for different reasons. The canonical [assertion model](statements.md) owns object identity and lifecycle; this chapter defines temporal value types and the policies that interpret them.

- An [Assertion](statements.md) carries validity: when its Proposition applies in the modelled world.
- An Occasion carries observed time for the external social or input event and recorded time for ingestion.
- An Attestation carries the time of the telling within its Occasion when finer precision is available, plus recorded time if it differs.
- A [Perception](artefacts-and-perceptions.md) carries observation and recorded times for the model or tool Activity that produced it.
- Event-attribute Assertions carry occurrence values: when a happening occurs. An Event read projects an audience-safe occurrence; Event identity owns no occurrence field.
- An agent-authored Task carries due or target time.
- A Trigger carries the condition and action that may fire relative to a Task.

Validity, observation, recording, occurrence, due time, and firing condition are not interchangeable.

## Typed temporal values

Temporal values are typed from genesis. Strings exist only at input and rendering boundaries. Every value records its precision, uncertainty, and timezone ownership rather than manufacturing exactness.

| Type | Required semantics |
|---|---|
| `CivilDate` | Calendar date with calendar and no implied instant. |
| `LocalDateTime` | Civil date and time whose timezone is unknown or owned by a named person, place, connector, or policy. |
| `ZonedDateTime` | Civil date and time plus IANA zone and calendar; resolves to an instant under a named timezone database version. |
| `Instant` | Absolute timeline point. |
| `Interval` | Bounds that may be open, inclusive, or exclusive, each with precision and uncertainty. |
| `Duration` | Fixed elapsed quantity where that interpretation is valid. |
| `CalendarSpan` | Years, months, weeks, or days resolved against an anchor and calendar. |
| `QualitativeInterval` | A relation such as before, after, during, overlaps, meets, or equals against another temporal object. |
| `RecurrenceIntent` | A typed recurrence plus explicit ambiguity and exceptional-date policies. |

Precision includes at least year, month, day, minute, second, and exact instant. Uncertainty may be an explicit bound, qualitative estimate, or unknown. Timezone ownership distinguishes “09:00 in Rowan's current zone” from “09:00 Europe/Stockholm” and records the policy used to resolve the former. A later timezone or database change produces a new projection; it does not alter the original value.

Civil and absolute time must remain distinct. A birthday is a civil date, not an instant. Calendar spans are anchor-aware: adding one month to January 31 invokes the recorded month policy rather than becoming 30 days. These rules follow established temporal-database and date-time-library practice ([research lane](research/2026-07-24/lanes/time-memory.md#1-bitemporal-and-tri-temporal-database-theory-vs-zuihitsus-assertedoccurred-split); [typed-value research](research/2026-07-24/lanes/time-memory.md#3-typed-temporal-values-at-an-llm-interface-103)).

## Validity, observation, and recording

An Assertion's validity is an interval or qualitative temporal constraint over the Proposition. It answers when the asserted content applies. Repeated disjoint periods are separate Assertions over the same Proposition. A changed interpretation of validity appends a correction or supersession transition; it does not edit the source Occasion or Attestation.

Observed and recorded time belong to source and observation records:

- Occasion observed time says when the external input event happened according to connector or operator evidence.
- Attestation observed time may identify a span within the Occasion.
- Perception observed time says when the tool or model made its observation, which may differ from the Artefact's creation or sharing time.
- recorded time says when each record entered the log.

Delayed ingestion can therefore represent a document authored in 2019, shared in 2025, perceived in 2026, and recorded later without treating any one time as the Assertion's validity. If an utterance does not establish validity, the Assertion remains temporally open or unknown; the system does not date it by the day it was heard.

Temporal databases support validity and transaction-time separation. The additional observed/source distinctions and their assignment to these objects are this design's synthesis ([research lane](research/2026-07-24/lanes/time-memory.md#the-canonical-two-axes)).

## Occurrence, Task, and Trigger

Occurrence, Task, and Trigger are mechanically separate in the required genesis substrate.

An Event occurrence Assertion describes when a happening occurs. It never fires by itself. An agent-authored Task is a minted immutable instruction source containing the specified action, actor, arguments, authority, audience, source, and optional due or target time. Its transitions fold from `proposed` to `active`, `completed`, `cancelled`, `superseded`, or `erased`. A Trigger is a separately minted immutable condition/action binding owned by one Task. Its transitions fold from `inactive` to `armed`, `fired`, `cancelled`, `superseded`, or `erased`. Only an `armed` Trigger whose Task is `active` may invoke the bound action. A Trigger cannot target an Assertion or Event directly.

A description of another system's nightly job can therefore contain an actual habitual or recurring occurrence without creating a Task. A historical birth date can be an Event occurrence without a Trigger. Only an authorized Task with a live Trigger enters the scheduler.

The current system's overloaded occurrence field produced the observed failure: a third party's recurring routine woke the agent. iCalendar's mature VEVENT/VTODO/VALARM separation establishes the solution shape, although the exact successor objects are a local design decision ([current failure](../docs/ontology-failures/2026-07-23.md#schedule-and-description-conflate-in-the-temporal-model); [research lane](research/2026-07-24/lanes/time-memory.md#2-the-schedule-vs-description-conflation-failure-class-4); [verification](research/2026-07-24/verification/part-b.md#verdict-table)). Trigger separation must exist before structured Assertions can become authoritative; adding it later would leave old descriptive recurrence capable of acquiring new firing semantics.

## Modality

Time does not encode modality. [Proposition identity](statements.md) includes a registered modality axis from genesis. At minimum it distinguishes:

- `actual`: asserted to hold in the modelled world;
- `planned`: intended future occurrence, without asserting completion;
- `hypothetical`: considered or conditional content;
- `habitual`: a disposition or recurring pattern, not a claim about every instant;
- `deontic`: an obligation or permission;
- `cancelled`: a plan or scheduled Event whose cancellation is asserted.

`cancelled` is a registered Proposition modality and remains mechanically distinguishable from `planned` and `actual`. A planned Event does not become actual merely because its date passes. Event cancellation creates a new Assertion with `cancelled` modality and provenance; it does not mutate the planned Assertion. Task and Trigger cancellation use their own lifecycle transitions and are not inferred from proposition modality. Third-party deontic content remains an Assertion; only an authorised agent-authored Task with a live Trigger can cause action.

Initial inference treats non-actual modalities opaquely except for rendering and scheduling guards. Richer qualitative and non-actual temporal reasoning is represented by separately registered `activation_gate` capabilities, principally `capability:qualitative-temporal-inference` and `capability:habitual-deontic-inference`; the initial `stage:9` policy remains opaque except for rendering and scheduling guards. Reserving and populating the modality identity slot is required at genesis because it cannot be reconstructed reliably from old prose later.

## Qualitative anchoring

Not all time has a date. Assertions and Events may be constrained relative to other temporal objects using a registered tractable subset of interval relations: before, after, during, overlaps, meets, and equals. Composition is deterministic under the definition version recorded for the relation set.

The full interval algebra is not adopted. The maximal tractable subclass is established, but whether this smaller subset is sufficient remains an implementation choice ([research lane](research/2026-07-24/lanes/time-memory.md#the-missing-piece-3-allens-interval-algebra-for-qualitative-anchoring); [verification](research/2026-07-24/verification/part-b.md#verdict-table)). Unknown or ambiguous anchoring remains explicit rather than being replaced with an invented timestamp.

## Recurrence intent

A recurrence is constructed from typed intent rather than accepted as an unchecked RRULE string. The value records frequency, interval, anchors, bounds, timezone ownership, calendar, and explicit policies for exceptional dates.

Supported policy choices include:

- `last_day`: choose the final civil day of each target month;
- `skip`: omit periods in which the requested civil value does not exist;
- `clamp`: use the final valid civil value in the period;
- `business_adjust(calendar, direction)`: move according to a named, versioned business calendar and direction;
- explicit gap/fold policy for daylight-saving transitions.

“Last day of every month” is valid intent and compiles to `last_day`; it is not banned because some months lack a 31st. “Monthly on the 31st” is rejected as ambiguous unless the writer chooses `skip`, `clamp`, or `last_day`. Impossible civil dates are never silently rolled. A business adjustment without a calendar version is invalid.

These policies preserve ordinary intent while addressing the RRULE pathologies identified in the temporal research ([research lane](research/2026-07-24/lanes/time-memory.md#rrule-pathologies-to-guard-at-the-typed-boundary)).

## Correction and refinement

A temporal correction changes an interpretation, not the source. Suppose an Occasion contains “we shipped on Tuesday,” and an Assertion initially resolves validity or Event occurrence to the wrong Tuesday. The correction appends:

1. evidence for the corrected temporal value;
2. a replacement or supersession transition from the old Assertion or occurrence interpretation;
3. a new Assertion or Event attribute using the corrected value;
4. a Derivation naming the temporal parser, policy, ontology, unified ResolutionEnvironment, and source locator.

The Occasion, utterance span, Attestation, and old record remain unchanged. If no corrected value is supported, withdrawal without substitution is valid. The current system's cue and withdrawal work shows why evidence should be requested and why unsupported substitution is unsafe ([current-system evidence](research/2026-08-06/current-system-fixes.md#span-justification)).

A world change is different from a correction. If someone actually changes employer, close the old Assertion's validity and add a new Assertion. If the old employer was recorded incorrectly, supersede or retract the old Assertion. Both are append-only, but their transition reasons differ.

## Staleness and volatility

An open validity interval does not imply permanent truth. Volatility is a registered property of relation or Assertion policy, with expected review horizons and domain-specific evidence requirements. Passing a horizon does not mutate the source or silently close its interval. It creates a candidate Activity that may:

- append a derived replacement with bounded validity when evidence supports a historical reading;
- append a closure or supersession transition with that evidence;
- annotate the current Assertion as stale under a named policy;
- queue a re-verification request;
- leave the Assertion unchanged when evidence is insufficient.

Temporalisation is therefore a derived operation with complete lineage. It never edits the original Assertion or participant words. Policy changes rebuild the stale projection and may generate new proposals; they do not make historical records mean something new.

## Genesis and policy boundary

Required at genesis: typed values with precision, uncertainty, timezone ownership, validity on Assertions, source observation and recording times, occurrence represented by Event-attribute Assertions, modality identity, and mechanically separate Task and Trigger records.

Initial policy: conservative actual/planned/cancelled rendering, no firing from descriptive occurrence, explicit safe recurrence constructors, and source-preserving correction.

Activation-gate capabilities: `capability:qualitative-temporal-inference`, `capability:business-calendar-adjustment`, `capability:volatility-automation`, `capability:habitual-deontic-inference`, and `capability:autonomous-recurrence-interpretation`. Their required raw fields exist from `stage:9`, so disabling a capability changes projections and automation, not persisted meaning.
