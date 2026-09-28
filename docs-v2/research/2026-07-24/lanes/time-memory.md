# Lane: time, and memory typology

Research lane for the zuihitsu ontology redesign. Covers (1) bitemporal database
theory and its gaps against zuihitsu's model, (2) the schedule-vs-description
conflation (failure class 4), (3) typed temporal values at an LLM interface (#103),
(4) memory typology from cognitive architectures (#58/#59/#74), and (5) a
principled retrieval/decay model to replace cosine-threshold hygiene (failure
class 9). Every load-bearing claim is cited; uncertainty is flagged inline.

---

## 1. Bitemporal (and tri-temporal) database theory vs zuihitsu's asserted/occurred split

### The canonical two axes

The temporal-database tradition (Snodgrass, Jensen, TSQL2) fixes two orthogonal
time dimensions:

- **Valid time** — when a fact is true *in the modelled reality*.
- **Transaction time** — when a fact is *current in the database* (recorded).

A relation carrying both is *bitemporal*; each tuple has a subset of valid times,
and each valid-time chronon has a subset of transaction times.[^tsql2model][^tsql2edc]
This is exactly zuihitsu's `occurred_at` (valid time) / `asserted_at` (transaction
time) split, confirmed by `docs/time.md` and by the Zep/Graphiti prior art it
cites.

SQL:2011 standardised this with two named, *interval*-shaped time dimensions:
**application-time period tables** (valid time, user-maintained, via a
`PERIOD FOR` on user columns) and **system-versioned tables** (transaction time,
automatically maintained by the DBMS).[^sql2011wiki][^sql2011ulb] The load-bearing
detail is that SQL:2011's time dimensions are **periods (intervals), not
instants** — every row carries a `[start, end)` for each temporal dimension.

### Tri-temporal: the decision-time axis zuihitsu lacks

A third axis appears in the literature: **decision time** — when the decision
that committed the system to a fact was *made*, as distinct from when the fact
became true (valid) or when it was recorded (transaction).[^tritemporal] The
worked example: an employee record where the *hiring-approval* date (decision) is
separate from the *employment start* (valid) and the *row insertion* (transaction).
Decision time captures delayed effects, retroactive changes, and revisions —
"we decided on Monday that the raise is effective retroactively from January, and
recorded it Wednesday."

**Relevance to zuihitsu.** zuihitsu has a strong analogue of decision time that
it does not name: the difference between *when a teller stated something* and
*when the agent recorded it*. Today `asserted_at` collapses "the teller said it"
and "the log recorded it" into one timestamp — fine while they coincide, but
episodic conversation search (#74) and delayed ingestion (#44 bulk documents,
where a document authored in 2019 is ingested in 2026) break that coincidence.
Zep/Graphiti already split these into **four** timestamps per edge: `valid from`,
`valid to`, `observed` (when the source *stated* it), and `recorded` (when the
system *ingested* it).[^zep] The `observed`/`recorded` split *is* the decision/
transaction distinction. zuihitsu's single `asserted_at` is under-resolved here.
`docs/time.md` §"Now" already reconstructs per-turn wall-clock from `recorded_at`,
so the raw data exists; it is just not lifted onto the fact as a first-class
"stated-at vs ingested-at" pair.

### The missing piece #1: validity *intervals on facts and relations*

This is the sharpest gap and it maps directly onto **failure class 3** (relations
are bare edges). SQL:2011, TSQL2, and every interval-based temporal knowledge
graph attach a **validity interval** to the *fact/edge itself*: a temporal fact
is `(subject, predicate, object, τ)` where τ is the interval during which the
relation held.[^tkg-medium] The canonical example is exactly the one in the
zuihitsu failure log: `(Boris Johnson, isPrimeMinisterOf, UK, [2019, 2022])`.[^tkg-medium]
Intervals can be **left-open** (start unknown), **right-open** (end unknown, i.e.
still current), or **closed**.[^tkg-medium]

zuihitsu's links are `{from, to, relation, ...}` with **no validity interval**, so
"worked at X 2019–2021" cannot be a relation — it degrades into prose in a
ContentEntry, which is failure class 1 (facts are sentences). The fix from the
temporal-DB canon: relations carry their own `TemporalRef`-shaped valid interval,
independent of the `asserted_at`/`observed` transaction axis. This is a
*bitemporal edge*, and it is well-trodden: "Bitemporal Property Graphs" formalises
edges bearing both valid and transaction time.[^bitemporal-pg]

### The missing piece #2: fact versioning (a fact true then false)

In temporal KGs, when new information contradicts an existing fact the system
**closes the old fact's validity window** (sets `valid to`) and records the new
fact — invalidation, not deletion.[^zep][^tkg-medium] The superseded fact stays
queryable for historical/point-in-time reads. zuihitsu has *supersession
tombstones* on entries, which is the same idea on the transaction axis, but it
lacks the **valid-time** version: "Dave lived in Sydney (valid 2015–2020), then
Melbourne (valid 2020–)". Today both are separate prose entries with separate
`occurred_at`s and no explicit "this closes that" edge; the "current residence"
query must be re-derived from prose + recency rather than read off a closed/open
interval. A redesign should distinguish:
- **Transaction-time supersession** (existing tombstones): "I was wrong, scrub the
  earlier assertion" — the earlier assertion was *never* true.
- **Valid-time versioning** (missing): "it *was* true, then it stopped being true"
  — close the interval, keep the history.

Conflating these is a known anti-pattern; the temporal-DB literature keeps them on
orthogonal axes precisely because "I made a mistake" and "the world changed" are
different operations with different audit semantics.[^sql2011ulb]

### The missing piece #3: Allen's interval algebra for qualitative anchoring

zuihitsu already has `BeforeAfter { dir, anchor }` — a qualitative temporal
relation anchored to another memory. This is a *two-relation subset* (before,
after) of **Allen's interval algebra**, which defines the 13 mutually exclusive,
exhaustive, qualitative relations between two intervals: `before/after`,
`meets/met-by`, `overlaps/overlapped-by`, `starts/started-by`, `during/contains`,
`finishes/finished-by`, and `equals`.[^allen] Allen's algebra is the principled,
complete vocabulary for *qualitative* temporal placement when exact dates are
unknown — exactly zuihitsu's use case ("after Dave's wedding," "during the
pandemic," "while he was at that job"). Two observations:

1. The `during`/`contains`/`overlaps` relations are what you need to place a fact
   *inside* a relation's validity interval ("she mentioned it *during* her time at
   X") — impossible today because neither relations-with-intervals nor the richer
   Allen relations exist.
2. Allen's algebra supports **constraint propagation / composition** (transitivity
   tables: if A before B and B before C, then A before C), which lets a temporal
   graph *answer ordering questions* it was never told directly. zuihitsu's
   `BeforeAfter` resolves to bounds by a one-time denormalisation (`docs/time.md`
   §Storage), never composes, so it cannot answer "did X happen before Y?" unless
   both are pinned to instants. Adopting even a small closed subset of Allen
   relations with a composition table would make the "temporal graph alongside the
   relationship graph" (which `docs/time.md` already gestures at) actually
   *inferential*.

**Uncertainty flag:** full 13-relation Allen reasoning with path-consistency is
NP-hard in the general case (the full interval algebra), though tractable
subclasses exist (e.g. the pointisable / ORD-Horn subclass). I did not verify the
exact tractable-subclass boundary against a primary source in this pass; treat
"adopt a tractable subset" as the safe recommendation, not "adopt all 13 with full
propagation."

---

## 2. The schedule-vs-description conflation (failure class 4)

**The failure:** a single `occurred_at` field means both "when the referent
happens" (descriptive temporal scope) and "when the agent should act" (deontic /
task time). `FREQ=DAILY` stamped on a fact *describing another bot's cron job*
made the agent wake for it. `docs/time.md` even admits the overload: "A deadline
mildly overloads `occurred_at`, which we accept deliberately."

### How mature systems separate these: iCalendar is the reference design

iCalendar (RFC 5545) is the load-bearing prior art because it separates the two
concerns into **distinct component types**:

- **VEVENT** — a thing that *occurs* on the calendar. Has `DTSTART`/`DTEND`: the
  descriptive occurrence.[^ical-todo]
- **VTODO** — an *action item / task*. Uses `DUE` (not `DTEND`) and carries
  task-specific state like `PERCENT-COMPLETE` and `STATUS`.[^ical-todo] A VTODO is
  a *deontic* object: something the agent is meant to do.
- **VALARM** — a *trigger to act*, nested inside a VEVENT or VTODO. It has a
  `TRIGGER` (when to fire, expressible as a duration *relative to* the parent's
  `DTSTART`/`DTEND`/`DUE`) and an `ACTION` (what to do: DISPLAY, EMAIL,
  …).[^ical-trigger][^ical-alarm]

The three-way split is precisely the distinction zuihitsu collapses:

| iCalendar | Concept | zuihitsu today |
|---|---|---|
| VEVENT `DTSTART` | *when the referent occurs* (descriptive) | `occurred_at` |
| VTODO `DUE` | *when a task is due* (deontic) | `occurred_at` + `#due` tag (overloaded) |
| VALARM `TRIGGER`/`ACTION` | *when/whether to act, and what action* | `occurred_at` + wake-up scheduler (implicit) |

The critical iCalendar insight: **the trigger is separate from the occurrence, and
the trigger is defined *relative to* the occurrence** (`RELATED=START`, an offset
duration).[^ical-trigger] A VEVENT with no VALARM is a pure description — it *never
fires*. zuihitsu has no way to say "this is a description with no alarm"; any dated
memory can arm the wake-up scheduler.

### Corroborating models

- **BPM / workflow** and **real-time BDI agents** annotate *intentions/plans* with
  deadlines, durations, and priorities *at the execution layer*, kept separate from
  the descriptive temporal facts in the belief base — a desire's deadline is "the
  relative time instant when the goal is expected to be achieved," a property of
  the *intention*, not of a world-fact.[^bdi] The separation of "belief about when
  something is true" from "intention with a deadline" is architectural in BDI.
- **VALARM's `ACTION`** makes explicit that a trigger carries an *action type* —
  there is no such thing as "fire" without "fire *to do what*." zuihitsu's fired
  wake-up infers the action (relay to the target participant) from the memory's
  subject/teller (`docs/time.md` §Agent-initiated speech), which is a heuristic
  standing in for an absent explicit action field.

### The principled split for zuihitsu

Separate three fields that today are one:

1. **Descriptive occurrence** (`occurred_at`, keep): the valid-time placement of the
   referent. Answers "when did/does this happen in the world." A fact *describing*
   another bot's cron job legitimately has `Recurring(FREQ=DAILY)` here — that is
   true of the world.
2. **Deontic due/target** (new, e.g. `act_at` or a first-class *Task* object):
   present *only* when the agent itself is meant to do something. A description
   has none. This is the VEVENT-vs-VTODO line.
3. **Trigger** (new, derived or explicit): when to surface, expressed *relative to*
   the deontic due (VALARM `TRIGGER` semantics), plus what action to take (VALARM
   `ACTION`). The wake-up scheduler should arm on the *trigger*, never on the bare
   descriptive occurrence.

The one-line diagnosis of failure class 4: **the wake-up scheduler currently arms
on the descriptive axis because there is no deontic axis to arm on.** Introducing a
VTODO/VALARM-shaped deontic layer means a dated *description* (the other bot's
cron) can never fire, because only a Task with a Trigger fires — and a Task is
something the agent authored *for itself*, never something it merely recorded about
the world. This also cleanly resolves `docs/time.md`'s admitted overload of `#due`.

---

## 3. Typed temporal values at an LLM interface (#103)

The design pressure in #103: the agent surface is "halfway to typed dates" — date
objects that are secretly `{ day = "YYYY-MM-DD" }` string tables, and duration
*strings* (`"6 months"`) parsed to anchor-free milliseconds, forcing a month to be
a fixed 30 days. The prior art strongly favours full typing.

### Civil vs absolute is the load-bearing distinction

The Temporal API (TC39, the modern JS date redesign, and jiff's direct model)
draws the distinction zuihitsu needs:

- **Instant** — an absolute point on the timeline (nanoseconds since epoch), no
  calendar, no zone.[^temporal-mdn]
- **PlainDate / PlainDateTime (civil / "plain")** — a wall-clock date with *no*
  timezone: "March 10, 2pm" without committing to which March 10.[^temporal-mdn]
- **ZonedDateTime** — instant + zone + calendar, the bridge between the two.[^temporal-mdn]
- **Duration** — a span with per-unit values (years…nanoseconds); *not*
  calendar-aware on its own — "durations are just quantities."[^temporal-mdn]

zuihitsu's `TemporalRef` already honours civil-vs-absolute (`Day` civil vs `Instant`
absolute), and `docs/time.md` already leans on it (Day → noon-representative bounds).
The gap #103 names is **durations**: an anchor-free duration *cannot* represent a
calendar month, because a month has no fixed millisecond length. This is exactly why
Temporal separates `Duration` (the quantity) from the **arithmetic that anchors it**
(`plainDate.add(Duration)` resolves the month against a reference date — 31 Jan + 1
month = 28/29 Feb). zuihitsu's `:add_months` metatable method already does calendar-
correct anchoring, but `calendar.upcoming("6 months")` does not, because the window
duration never meets an anchor. The fix is the Temporal model: a **typed Span**
threaded together with the query's *now*, so a month resolves against a real date.

### RRULE pathologies to guard at the typed boundary

If recurrences are exposed as typed values, the RFC 5545 RRULE edge cases must be
handled, not passed through raw:

- **Invalid BYxxx values are silently ignored**, not coerced: `FREQ=MONTHLY;
  BYMONTHDAY=31` *skips* months without 31 days (Feb, Apr, Jun, Sep, Nov) rather
  than rolling to the last day.[^rrule-kanzaki][^rrule-dateutil] A monthly rule
  from Dec 31 yields Dec 31, Jan 31, Mar 31, May 31 — a user who meant "end of every
  month" gets a broken schedule.
- **Impossible instances (Feb 30) must be ignored, never coerced**[^rrule-kanzaki]
  — which matches zuihitsu's existing `Day` handling (impossible civil date →
  empty bounds, never rolls into March; `docs/time.md` §Storage).
- The RFC's recommended fix for "last day of month" is `BYSETPOS=-1` /
  `BYMONTHDAY=-1` (negative index), not a fixed day number.[^rrule-dateutil]

zuihitsu already *rejects* free-phrasing recurrences ("every Monday" that is not a
valid rrule; `docs/time.md`). The typed-value redesign should go further and expose
a **constructor vocabulary** for the common safe patterns (`calendar.every("week",
"tuesday")`, `calendar.monthly_last_day()`), so the model names an operation rather
than hand-writing an RRULE string that can encode the BYMONTHDAY=31 trap. This is
the same "name the operation, don't compute the value" philosophy `docs/time.md`
already applies to dates, extended to recurrences and durations.

### Recommendation for #103

Take the *first-class values* option, not merely "retype the internals":
- Date, Duration/Span, and Recurrence as typed userdata backed by
  `jiff::civil::Date` / `jiff::Span` end to end, strings only at the input boundary.
- Preserve the two load-bearing affordances #103 flags: the value must still (a)
  stand in directly as an `occurred_at`, and (b) stringify to its ISO form under
  interpolation. Temporal/jiff both make this natural (a civil date has a canonical
  ISO string).
- Durations become anchor-aware by construction: a `Span` resolves against `now`
  (or any anchor date) inside calendar queries, killing the fixed-30-day month.
- Expose recurrence *constructors* that cannot encode the RRULE pathologies, rather
  than a raw RRULE string field.

---

## 4. Memory typology from cognitive architectures

The consistent finding across the mature cognitive architectures — and the one that
modern LLM-agent memory work has re-converged on — is a **four-way typology** that
zuihitsu currently collapses into one kind (the ContentEntry) plus ad-hoc side
channels.

### The canonical four kinds (Tulving → Soar/ACT-R → CoALA)

- **Working memory** — the actively-maintained current context.[^soar-intro][^coala]
- **Semantic memory** — general, de-contextualised facts about the world.[^soar-intro][^coala]
- **Episodic memory** — snapshots of *experience*, time-indexed; "what happened."[^soar-intro][^coala]
- **Procedural memory** — skills / how-to; in Soar, production rules; in CoALA, the
  agent's own code and the LLM weights.[^soar-intro][^coala]

**Soar** implements all four explicitly: procedural memory as production rules;
long-term declarative memory split into *semantic* (general facts) and *episodic*
(automatic snapshots of the top-state working memory, one stored at the end of each
decision cycle); retrieval is cue-driven.[^soar-intro][^soar-epmem] Soar's episodic
store is **automatic and architectural** — the agent does not choose to record an
episode, the architecture snapshots working memory every decision, and retrieval is
a cue-based nearest-neighbour over episodes.[^soar-epmem]

**CoALA** (Sumers, Yao, Narasimhan, Griffiths, TMLR 2023) is the direct bridge to
LLM agents: it classifies a language agent's memory into working / episodic /
semantic / procedural, with procedural memory being "code/LLM," and structures
actions into internal (memory) vs external (world).[^coala] The 2025–26 agent-memory
surveys report the ecosystem has "converged on a remarkably consistent three-tier
taxonomy — episodic, semantic, procedural — that mirrors decades of cognitive
science."[^agentmem-survey]

The concrete LLM exemplars:
- **Generative Agents** (Park et al. 2023): observation stream (episodic) + reflection
  (abstraction into semantic) + importance-weighted retrieval.[^genagents-survey][^genagents]
- **Voyager** (Wang et al. 2023): the clearest *procedural* memory — a skill library
  of *executable code*, each skill indexed by the embedding of its natural-language
  description, retrieved by cue and composed into new behaviours.[^voyager]
- **MemGPT/Letta**: OS-style hierarchical memory (working "RAM" main context vs
  archival "disk"), i.e. the working-vs-long-term split as virtual memory.[^agentmem-survey]

### Mapping zuihitsu's open issues onto the typology

This is the payoff. The three "nowhere principled to live" issues are each a
*different memory kind* the current single-ContentEntry ontology has no slot for:

| Issue | What it wants | Memory kind | Prior art |
|---|---|---|---|
| **#58** saved Luau procedures | store & invoke reusable code | **Procedural** | Voyager skill library[^voyager]; CoALA procedural (code)[^coala] |
| **#74** search past conversations / tool results | recall raw experience when semantic search misses | **Episodic** | Soar auto-snapshot episodes[^soar-epmem]; Generative Agents observation stream[^genagents] |
| **#59** transient private scratchpad | working notes across turns, not committed | **Working** (persistent) | MemGPT main context[^agentmem-survey]; CoALA working memory[^coala] |
| ContentEntry (today) | curated durable facts | **Semantic** | Soar/ACT-R declarative semantic[^soar-intro] |

zuihitsu's ContentEntry is a **semantic** memory (curated, de-contextualised prose
facts with visibility and provenance). The redesign should make the *kind* a
first-class type distinction, because **each kind has different lifecycle, decay,
retrieval, and authority rules** — that is the entire point of the typology, not
mere labelling:

- **Semantic (ContentEntry, existing):** durable; volatility-modulated ranking decay
  (existing); visibility-gated; distilled into descriptions; written by tellers,
  curated by maintenance passes. Authority: Platform (turns) + Agent (maintenance).
- **Episodic (#74):** *automatic*, not curated — the event log's conversation turns
  and tool results already *are* an episodic store; #74 is really "expose an
  episodic retrieval index over the existing log." Lifecycle: never edited (it is
  raw experience), never distilled into descriptions, aggressively recency-decayed
  in retrieval (old episodes rarely wanted), and — critically — **read-only and
  provenance-heavy**: an episode is "what was said," not "what is true," so it must
  surface with staleness/uncuratedness markers (the issue's own "usual caveats about
  staleness apply"). It is the *fallback* tier: consulted only when semantic search
  misses. This matches Soar's design where episodic is architectural/automatic and
  semantic is deliberate.[^soar-epmem]
- **Procedural (#58):** executable Luau, indexed by a natural-language description
  embedding (Voyager's exact design[^voyager]). Lifecycle: **decay by invocation
  recency/frequency, not by calendar age** (#58 Q5 asks exactly this — a procedure
  unused for a year but still correct is not "stale" the way a fact is; ACT-R's
  base-level activation, frequency+recency of *use*, is the right decay model here —
  see §5). Retrieval: on-demand cue-based (query-plan embedding), not auto-loaded, to
  bound prompt cost. Authority: Agent-authored (ties to the #20 reflection pass as
  the natural producer). Invocation runs in the same frozen sandbox under the same
  `max_steps`/`block_timeout` budget (#58 Q3).
- **Working / scratchpad (#59):** persistent-but-transient, private to the agent,
  **outside the visibility model entirely** (it is the agent's own head). Lifecycle:
  the staging area — reflection promotes items to semantic memory or discards them
  (#59's own design). Decay: cleared on consolidation, not ranked. **Storage tension
  (#59 Q2):** the event log is the source of truth, but scratchpad churn is exactly
  the kind of transient state that should *not* bloat replay. Recommendation: keep it
  in the log (determinism is a fixed point) but as a *compactable* channel — a
  `ScratchpadWritten`/`ScratchpadCleared` pair whose net effect after consolidation is
  a single promote-or-discard, analogous to how superseded blocks are folded. This
  preserves "system is a pure function of the log" while letting the working set stay
  bounded. **Uncertainty flag:** whether the log or a side-table is right here is a
  genuine open call that trades replay-purity against log size; I lean log-with-
  compaction but flag it as the weakest recommendation in this lane.

### What the type distinction *buys* (the concrete argument)

The failure log's class 1 ("facts are sentences") is really "every memory is the
*same* kind of sentence, so every downstream mechanism re-derives structure." A
typed memory ontology fixes a sibling problem: **not every memory should obey the
same lifecycle.** Today a procedure, a scratch note, a raw conversation turn, and a
curated fact would all be ContentEntries subject to the same visibility predicate,
same volatility decay, same dedup/consolidation at cosine 0.85/0.95 — which is wrong
for all three of the non-semantic kinds. A procedure deduped against a fact by cosine
similarity is nonsense; a scratch note distilled into a public description is a
privacy leak; a raw episode consolidated as if it were a curated fact launders
"someone said X" into "X is true." The type distinction is what lets each kind carry
its *own* decay, retrieval, distillation, and authority rules — which is the whole
reason cognitive architectures separate them.[^coala][^soar-intro]

---

## 5. Base-level / spreading activation vs cosine-threshold hygiene (failure class 9)

**The failure:** dedup at cosine 0.95, consolidation at 0.85 — constants *in one
embedder's geometry*. An embedder change silently invalidates all of them, because
absolute cosine values are encoder-specific.

### The empirical problem is real and documented

Absolute cosine-similarity values are **not comparable across embedding models**:
different encoders show "systematic differences in absolute cosine similarity
values" under a fixed threshold, even when the *relative* similarity structure is
preserved.[^cosine-drift] Mixing vectors from two model versions in one index makes
similarity "less meaningful across generations" because the embedding spaces have
different geometry even at equal dimensionality.[^cosine-drift] And a *fixed*
threshold decays as the data distribution shifts — "old similarity thresholds stop
being properly calibrated"; adaptive schemes (e.g. Class-Typical-Matching) re-fit
the threshold as the cosine distribution moves.[^embed-drift] So zuihitsu's
constants are, as the failure log says, load-bearing magic numbers pinned to one
model's space.

### The two principled alternatives

**(a) Rank/distribution-relative thresholds, not absolute constants.** Instead of
"dedup if cosine > 0.95," use a threshold *calibrated from the current embedder's
own similarity distribution* — e.g. a percentile of same-memory vs cross-memory
similarity, or a precision/recall-curve-fit F1-optimal threshold benchmarked against
labelled pairs.[^embed-drift] This makes the *decision* embedder-invariant even
though the *cosine number* is not, because the threshold moves with the geometry. It
is the minimal, lowest-risk fix and directly addresses "an embedder change silently
invalidates all."

**(b) ACT-R base-level activation as a decay/recency model that is *not* embedder
geometry at all.** ACT-R's declarative memory gives each chunk a **base-level
activation** from the frequency and recency of its use, following the power law of
forgetting:

> `B_i = ln( Σ_j t_j^(-d) )`

where `t_j` is time since the j-th access of chunk i, and `d` is the decay rate
(community default `d = 0.5`).[^actr-base] Total retrieval activation sums base-level
+ **spreading activation** (associative boost from currently-active context) +
partial-match penalty + noise; the most-active matching chunk wins.[^actr-base]

The key property for failure class 9: **base-level activation depends only on
access timestamps, not on embedding geometry at all** — it is a pure function of the
event log's own `seq`/time (which zuihitsu already has, and which is deterministic
under replay). Recency and frequency of *use* are embedder-independent. This is a
strictly better foundation for zuihitsu's *volatility/staleness decay* (§Recency and
volatility in `docs/time.md`), which is currently an ad-hoc `exp(−Δt/τ)` with
hand-set τ = 90/365/3650 days. ACT-R's power-law form is the *principled* version of
the same shape, with decades of empirical fit,[^actr-base] and — critically — it
*naturally* models multiple accesses (each `touch`/retrieval adds a term), which
zuihitsu's single-Δt decay does not. zuihitsu already marks memories "touched" on
calendar reads (`docs/time.md`); those touches are exactly the access events ACT-R's
`B_i` sums over.

**Spreading activation** additionally offers a graph-native, embedder-independent
*retrieval* signal: activation flows from the currently-in-context memories along
relation edges to associated memories — "contextual relevance given the current
focus."[^actr-base][^soar-intro] Since zuihitsu is already a knowledge *graph*, this
is nearly free and complements cosine similarity with a *structural* signal that
does not move when the embedder changes. Soar likewise combines "spreading activation
and base-level activation based on recency and frequency" for its semantic-memory
retrieval.[^soar-intro]

### Recommendation for failure class 9

Two-part, layered:
1. **Retrieval/decay:** replace the ad-hoc `exp(−Δt/τ)` recency boost and the
   volatility-decay with an **ACT-R-style base-level activation** computed from the
   log's own access timestamps (frequency + recency, power-law), plus a
   **spreading-activation** term over the relation graph. Both are embedder-invariant
   and deterministic under replay — a strong fit for the append-only-log fixed point.
   Cosine similarity stays as *one* term (relevance), not the sole ranking axis.
2. **Hygiene (dedup/consolidation):** keep embeddings for candidate generation but
   make the *decision threshold* distribution-relative (percentile/F1-calibrated
   against the current embedder), and recompute it when `EmbeddingModelChanged`
   fires — turning the silent-invalidation failure into an automatic recalibration.
   The absolute constants 0.95/0.85 become *derived* quantities, not config magic.

**Uncertainty flag:** ACT-R base-level activation is well-validated for *human*
memory recall latency/probability; its transfer to "which agent memory to surface"
is by analogy, not proof. The strong, defensible claim is narrow: *frequency+recency
of access is an embedder-independent decay signal, and a distribution-relative
threshold survives embedder change* — both directly attack failure class 9. The
broader "adopt the full ACT-R activation equation" is a reasonable design bet, not a
verified win, and should be tuned empirically (the eval harness is the place).

---

## Implications for zuihitsu

### Recommended temporal model

1. **Add validity intervals to relations (and to facts).** A link/relation carries
   its own valid-time interval (`TemporalRef`-shaped, left/right-open supported), so
   "worked at X 2019–2021" is a *relation*, not prose. This directly fixes **failure
   class 3** and removes a major driver of **failure class 1**. Prior art:
   SQL:2011 application-time periods, interval-based temporal KGs, bitemporal property
   graphs.[^sql2011ulb][^tkg-medium][^bitemporal-pg]

2. **Split supersession into two axes.** Keep transaction-time tombstones ("I was
   wrong") *and* add valid-time versioning ("it was true, then stopped") — close the
   interval, keep the history, per the temporal-KG invalidation pattern.[^zep][^tkg-medium]

3. **Resolve the stated-vs-recorded (decision-time) axis.** Lift the existing
   `recorded_at` distinction onto the fact as an `observed`/`recorded` pair (Zep's
   4-timestamp model[^zep]), which #44 (delayed bulk ingestion) and #74 (episodic
   recall) both need.

4. **Fix the schedule/description conflation (failure class 4)** with an
   iCalendar-shaped three-layer split: descriptive `occurred_at` (VEVENT), a deontic
   Task/`due` layer (VTODO) present only when the agent must act, and a Trigger
   (VALARM: relative offset + action) that the wake-up scheduler arms on. A pure
   *description* (another bot's cron) has no Task and no Trigger, so it can never
   fire.[^ical-todo][^ical-trigger]

5. **Adopt a tractable Allen-interval subset** for qualitative anchoring, generalising
   `BeforeAfter` to before/after/during/overlaps/meets with a composition table, so
   the temporal graph can *infer* orderings.[^allen] (Flagged: bound to a tractable
   subclass, not the full NP-hard algebra.)

6. **Make dates/durations/recurrences first-class typed values (#103):** civil-vs-
   absolute per Temporal/jiff, anchor-aware `Span` durations (killing the fixed-30-day
   month), and recurrence *constructors* that cannot encode the `BYMONTHDAY=31`
   pathology.[^temporal-mdn][^rrule-kanzaki]

### Recommended memory typology

Promote memory *kind* to a first-class type, replacing the single ContentEntry with
four kinds carrying distinct lifecycle/decay/retrieval/authority rules
(Tulving/Soar/ACT-R/CoALA[^coala][^soar-intro]):

- **Semantic** (curated facts = today's ContentEntry): durable, volatility-decayed,
  visibility-gated, distilled into descriptions.
- **Episodic** (#74): the conversation/tool-result log exposed as an automatic,
  read-only, recency-decayed, provenance-marked *fallback* retrieval tier — never
  distilled, never treated as curated truth.[^soar-epmem][^genagents]
- **Procedural** (#58): executable Luau indexed by description embedding (Voyager),
  decayed by *invocation* frequency/recency (ACT-R base-level), retrieved on-demand,
  produced by the #20 reflection pass.[^voyager][^actr-base]
- **Working/scratchpad** (#59): persistent-transient, private, outside the visibility
  model, consolidated (promote-or-discard) by reflection; stored as a compactable log
  channel to preserve replay-purity without bloat *(weakest-confidence recommendation)*.

The payoff against **failure class 1**: not every memory should obey the same
lifecycle, and forcing four kinds through one visibility predicate + one cosine-
dedup + one volatility decay is wrong for three of them (a procedure deduped against
a fact; a scratch note distilled into a public description; a raw episode laundered
into asserted truth).

### Recommended retrieval/decay model (failure class 9)

- Replace ad-hoc `exp(−Δt/τ)` recency and volatility decay with **ACT-R base-level
  activation** (frequency + recency of access, power law, over the log's own
  timestamps) plus **spreading activation** over the relation graph — both
  embedder-invariant and deterministic under replay.[^actr-base][^soar-intro]
- Make dedup/consolidation thresholds **distribution-relative** (percentile/F1-fit
  against the current embedder), recomputed on `EmbeddingModelChanged`, so 0.95/0.85
  stop being magic constants and embedder change triggers recalibration rather than
  silent invalidation.[^cosine-drift][^embed-drift]

### Failure-class / issue coverage map

- **Class 1 (facts are sentences):** typed memory kinds + relations-with-intervals
  move structure out of prose.
- **Class 3 (bare edges):** validity intervals on relations.
- **Class 4 (schedule/description):** iCalendar VEVENT/VTODO/VALARM split.
- **Class 9 (embedder-geometry thresholds):** ACT-R activation + distribution-relative
  hygiene thresholds.
- **#103:** first-class typed dates/durations/recurrences.
- **#58 / #74 / #59:** procedural / episodic / working memory kinds.
- **#44:** decision-time (observed vs recorded) axis for delayed ingestion.

---

## Sources

[^tsql2model]: Jensen, Snodgrass, Soo, "The TSQL2 Data Model." https://people.cs.aau.dk/~csj/Thesis/pdf/chapter12.pdf
[^tsql2edc]: Snodgrass, "Temporal Databases." https://www2.cs.arizona.edu/~rts/pubs/EDC.pdf
[^sql2011wiki]: "SQL:2011," Wikipedia. https://en.wikipedia.org/wiki/SQL:2011
[^sql2011ulb]: Kulkarni & Michels, "Temporal features in SQL:2011." https://cs.ulb.ac.be/public/_media/teaching/infoh415/tempfeaturessql2011.pdf
[^tritemporal]: "Temporal database," Wikipedia (valid/transaction/decision time). https://en.wikipedia.org/wiki/Temporal_database
[^allen]: "Allen's Interval Algebra" (13 relations), CRAN ArchaeoPhases vignette. https://cran.r-project.org/web/packages/ArchaeoPhases/vignettes/allen.html
[^zep]: "What Is a Temporal Knowledge Graph?" Zep (Graphiti bitemporal: valid from/to, observed, recorded; invalidation by closing valid-to). https://www.getzep.com/ai-agents/temporal-knowledge-graph/
[^tkg-medium]: "Temporal Knowledge Graphs," Self Study Notes (Medium): temporal fact (s,p,o,τ); left/right-open intervals; Boris Johnson example. https://medium.com/self-study-notes/temporal-knowledge-graphs-5b032671e0c7
[^bitemporal-pg]: "Bitemporal Property Graphs: Dealing with Both Valid and Transaction Time," ADBIS. https://link.springer.com/chapter/10.1007/978-3-032-05281-0_15
[^ical-todo]: RFC 5545 §3.6.2 To-Do Component (VTODO: DUE, PERCENT-COMPLETE). https://icalendar.org/iCalendar-RFC-5545/3-6-2-to-do-component.html
[^ical-trigger]: RFC 5545 §3.8.6.3 Trigger (TRIGGER relative to DTSTART/DTEND/DUE via RELATED). https://icalendar.org/iCalendar-RFC-5545/3-8-6-3-trigger.html
[^ical-alarm]: RFC 5545 §3.6.6 Alarm Component (VALARM: ACTION + TRIGGER). https://icalendar.org/iCalendar-RFC-5545/3-6-6-alarm-component.html
[^rrule-kanzaki]: iCalendar RRULE spec (invalid BYxxx ignored; impossible instances ignored not coerced). https://www.kanzaki.com/docs/ical/rrule.html
[^rrule-dateutil]: python-dateutil rrule docs (BYMONTHDAY=31 gaps; BYSETPOS=-1 for end-of-month). https://dateutil.readthedocs.io/en/stable/rrule.html
[^temporal-mdn]: "JavaScript Temporal is coming," MDN (Instant vs PlainDate/PlainDateTime vs ZonedDateTime; Duration not calendar-aware). https://developer.mozilla.org/en-US/blog/javascript-temporal-is-coming/
[^soar-intro]: Laird, "Introduction to the Soar Cognitive Architecture" (arXiv:2205.03854) — working/procedural/semantic/episodic; spreading + base-level activation. https://arxiv.org/pdf/2205.03854
[^soar-epmem]: Soar Manual, Episodic Memory (automatic snapshot per decision; cue-based nearest-neighbour retrieval). https://soar.eecs.umich.edu/soar_manual/07_EpisodicMemory/
[^actr-base]: ACT-R base-level activation equation B_i = ln(Σ t_j^-d), d=0.5 default; total activation = base + spreading + mismatch + noise. Petrov, "Computationally Efficient Approximation of the Base-Level Learning Equation." http://act-r.psy.cmu.edu/wordpress/wp-content/uploads/2012/12/652petrovAbstract.pdf
[^coala]: Sumers, Yao, Narasimhan, Griffiths, "Cognitive Architectures for Language Agents" (CoALA), TMLR / arXiv:2309.02427 — working/episodic/semantic/procedural(code+LLM). https://arxiv.org/abs/2309.02427
[^voyager]: Wang et al., "Voyager: An Open-Ended Embodied Agent with LLMs" (arXiv:2305.16291) — skill library of executable code indexed by description embedding, retrieved by cue. https://arxiv.org/abs/2305.16291
[^genagents]: Park et al., "Generative Agents" (arXiv:2304.03442) — retrieval = recency (γ=0.995/hr exp decay) + importance + relevance. https://ar5iv.labs.arxiv.org/html/2304.03442
[^genagents-survey]: Ruder newsletter summary of Generative Agents memory stream (observation/reflection/retrieval). https://newsletter.ruder.io/p/generative-agents-forums-for-foundation
[^agentmem-survey]: "Memory for Autonomous LLM Agents: Mechanisms, Evaluation, and Emerging Frontiers" (arXiv:2603.07670) — three-tier episodic/semantic/procedural convergence; MemGPT OS-style hierarchy. https://arxiv.org/html/2603.07670v1
[^bdi]: "Real-Time BDI Agents: a model and its implementation" (arXiv:2205.00979) — desires annotated with deadline/priority at the execution layer. https://arxiv.org/pdf/2205.00979
[^cosine-drift]: "Cosine Similarity Shift: Theory & Methods," EmergentMind; and "The Math Behind Good Embeddings… Why Your Vectors Drift" — absolute cosine values encoder-specific, cross-model geometry incomparable. https://www.emergentmind.com/topics/cosine-similarity-shift
[^embed-drift]: "5 methods to detect drift in ML embeddings," Evidently AI; "How to Measure Drift in ML Embeddings," Towards Data Science — fixed thresholds decay as distribution shifts; adaptive re-calibration. https://www.evidentlyai.com/blog/embedding-drift-detection
