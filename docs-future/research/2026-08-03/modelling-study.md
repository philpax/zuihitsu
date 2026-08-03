# Modelling study: the Statement model against a live corpus

A falsification exercise, run 2026-08-03 against the running instance's knowledge base before the design chapters were written. The question is narrow and empirical: **can the proposed Statement model express what this system actually recorded?**

The answer is a qualified yes. Most of the corpus models cleanly, and several shapes model *better* than they do today. But three shapes cannot be expressed at all, one of them accounts for a large fraction of the corpus, and one widely-held assumption about the model turns out to be wrong in a way that matters for the two-traces design. The findings are stated as they came out, including where they contradict the design that motivated the study.

## Method

The instance was stopped, its data directory backed up, and the event log read through a read-only connection. No write path was touched.

The corpus: 2,361 events, of which 198 are content entries across 45 memories, with 92 links over 17 registered relations. Every entry was read. Sampling was for shape diversity rather than randomness, and the shape taxonomy below emerged from the reading rather than being imposed on it.

**Genericisation.** Every example is rewritten with invented placeholders before appearing here, per the fixtures rule in `CONTRIBUTING.md`. The cast used throughout:

| Placeholder | Role |
|---|---|
| `self` | the instance |
| `person/rowan` | the instance's creator and operator |
| `person/wren` | another person, `rowan`'s partner, who operates a persona agent |
| `person/quill` | `wren`'s persona agent, a third-party bot the instance talks to |
| `person/quill@chat` | that agent's platform stub |
| Silvano Ferrer | an invented 18th-century pamphleteer, the historical figure `quill`'s persona is drawn from |
| Pepper | `wren`'s cat |
| `topic/persona_kit` | the framework `quill` runs on |
| `topic/letter_town` | a slow-correspondence community of agents |

Shapes are reproduced faithfully; content is not. Where a count is given it is from the real corpus; where a sentence is quoted it is an invented sentence of the same shape.

## The corpus at a glance

| Measure | Count |
|---|---|
| Content entries | 198 |
| Distinct entry texts | 182 |
| Entries that exactly duplicate another entry | 16 |
| Entries on the single largest memory (a persona agent) | 77 (39%) |
| Entries carrying a propositional-attitude verb (*argues, views, warns, describes, claims*) | 35 |
| Entries with three or more clauses or an internal semicolon | 20 |
| Entries carrying a hedge (*likely, possibly, tends to*) | 8 |
| Entries carrying temporal or plan language in prose (*as of, currently, plans to*) | 17 |
| Entries that are operating instructions rather than assertions | 22 |
| Visibility: public / attributed / private-to-teller | 157 / 40 / 1 |
| Teller: participant / agent / bootstrap | 120 / 77 / 1 |

Two numbers set up most of what follows. **Thirty-nine percent of the corpus sits on one memory**, a third-party persona agent, and **twenty-two entries are not assertions at all** but instructions to the agent about how to behave in a given context.

## Notation

Worked examples use this shorthand. It is a reading aid, not a proposed serialisation.

```
s1  (person/rowan, worked_at, org/northwind)
    interval  [2019-03, 2021-06)
    told_by   person/wren, turn:01J7…
    posture   public
    credence  confirmed · 2 independent tellers
    gloss     g4
```

```
e1  event/create
    agent     person/wren
    theme     person/quill
    source    topic/instance_architecture
    time      [2026-07-14, 2026-07-16)
```

## Bucket 1: expresses cleanly

Roughly 60% of the corpus, and several of these are improvements on what is recorded today rather than merely equivalent.

### Simple attributes

The commonest shape. `"Official animal is the lyrebird"`, `"Full name: Silvano Ferrer"`, `"is left-handed"`, `"has a cat called Pepper"`.

```
s2  (person/quill, official_animal, animal/lyrebird)
s3  (person/wren, keeps_pet, animal/pepper)
```

Nothing is lost. The gain is that `same (subject, relation, object)?` becomes a structural test, which matters because the corpus contains the same attribute recorded two and three times in slightly different words.

### Platform identity attributes

`"Chat username: rowan"`, `"Chat display name: Rowan"`, `"Chat ID: 100000000000000001"`. These recur as exact duplicates because each re-mint re-appends them: one text appears ten times, another four.

```
s4  (person/rowan@chat, platform_handle, "rowan")
    told_by  connector:chat
```

All sixteen exact-duplicate entries in the corpus collapse under structural equality. This is the duplicate-window critic doing real, measurable work on real data.

### Relations with validity intervals

Today a relation is a bare edge and anything time-bounded degrades to prose. The corpus has the degraded form: employment history, current operational status, and superseded plans all live inside sentences.

```
s5  (person/rowan, worked_at, org/northwind)   interval [2019-03, 2021-06)
s6  (person/quill, runs_on, model/opus-4.8)    interval [2026-07-16, …)
```

Supersession by window-closing replaces the current pattern of writing a new sentence and hoping the old one ages out.

### The multi-participant event

The flagship failure from the survey, present in the corpus exactly as described. One happening was recorded four times: once on each of the three participants' memories and once on a topic memory abstracting the pair's dynamic. The four texts are subject-appropriate rephrasings of one fact.

```
e1  event/create
    agent    person/wren
    theme    person/quill
    source   topic/instance_architecture
    time     [2026-07-14, 2026-07-16)   ("in two days")
```

Four entries become one Event with four role-edges. Each participant's memory reaches it by traversal rather than by holding a copy. The second and third filings resolve to `e1` as structural no-ops, so the flush burst has nothing to rephrase.

This case models cleanly and is the strongest single argument for the Event shape. One qualification is carried into bucket 2: the original prose also recorded that the event *sparked* a month-long project, and that consequence has nowhere to go.

### Provenance currently smuggled into prose

Several entries carry their own provenance inside the sentence: `"Told by wren that treating a cat as a superior is counter-revolutionary"`, `"Described by quill as his exact inverse"`, `"Agent observation: …"`. The teller is in the text because the text is the only place it fits, even though a `told_by` field exists.

```
s7  (person/quill, describes_as_inverse_of, person/juniper)
    told_by  person/quill
    posture  attributed
```

Clean win: the prefix disappears into a field, and the resulting Statement is comparable with others about the same subject.

### Hedges become credence

Eight entries hedge in prose. The corpus even contains a full revision arc: a claim first recorded as `"Likely based on <historical figure>"` and later, after corroboration, as a flat assertion of the same fact, with both entries alive simultaneously and nothing linking them.

```
s8  (person/quill, persona_of, person/ferrer)
    credence  suspected → confirmed   (2 tellers, independent)
```

One Statement whose credence moves, rather than two entries whose relationship is invisible.

### Consolidation artifacts stop being necessary

Three consolidations in the corpus produce a longer sentence concatenating two shorter ones, joined by a semicolon or a relative clause, with the relationship to the sources recoverable only through `produced_by`. Under the Statement model the two facts were always two Statements and there is nothing to consolidate. The maintenance pass that produces these is, on this evidence, a workaround for the prose representation rather than a feature.

## Bucket 2: expresses at a cost worth naming

### Compound entries fragment, and the gloss stops being one-to-one

Twenty entries carry three or more clauses. The worst case in the corpus is a single biography entry carrying eight distinct claims: nationality, current residence, four past employers, two named projects, a role, and a real name.

That entry becomes roughly eight Statements. Two costs follow, and the second is the important one.

The first is write amplification: one utterance now produces eight objects, eight critic passes, and eight rows.

The second is that **the gloss is not one-to-one with the Statement**. The design assumed each Statement carries its own prose gloss beside its structure. Here, eight Statements derive from one sentence, and there is no honest way to split that sentence eight ways; each fragment's "gloss" would be a synthetic phrase the speaker never said. The correct shape is that a **gloss belongs to the utterance, and many Statements point at it**, which is a different cardinality from the one the design assumed and changes what the second trace actually is.

This is not a defect in the Statement model. It is a correction to the two-traces design, and it arrived from the data rather than from theory.

The corpus also supplies the reason this matters for privacy: that same biography entry was recorded once as a single public entry including the person's real name, and the real name was later split out into its own entry marked private-to-teller. **A compound entry cannot carry mixed visibility, and the instance discovered this the hard way.** Fragmenting into Statements fixes it, because posture is per-Statement.

### Propositional attitudes need a Statement as an object

Thirty-five entries assert that someone *argues, views, warns, describes, rebuts, contrasts, concedes*, or *analyses* something, and the something is usually itself a complex proposition rather than an entity:

> `quill` rebuts the "system crash" reading of the revolution, arguing it was a rewrite of a legacy system that produced permanent changes including civil equality and the metric system.

The subject is a person, the relation is an attitude, and the object is a whole claim with its own internal structure. Modelling it as `(person/quill, argues, "…")` puts an unparsed sentence in the object slot, which reinstates facts-are-sentences one level down.

The honest form requires a Statement to be able to occupy an object slot:

```
s9   (revolution/1789, was, rewrite_of_legacy_system)
s10  (person/quill, argues, s9)
     posture  public
```

This is a real extension, and it is **not** covered by the Wikidata one-level-deep qualifier discipline, which constrains qualifiers rather than objects. It also raises a question the design has not answered: `s9` is not asserted by the system, only quoted by `s10`, so it must not be retrievable as a fact about the revolution. That is the same quotation-versus-assertion boundary the design already draws for inter-agent claims, applied one level in.

Cost: bounded nesting, and a rule that a nested Statement is never independently visible.

### Events have no relations to other events

The flagship event "sparked a month-long project". Elsewhere the corpus records a technical failure and, separately, the reaction to it. The universal role set places participants inside an event; it says nothing about how one event stands to another.

Causation, consequence, and precedence between events all currently live in prose. The fix is small (event-to-event relations, which the relation model already supports) but it must be stated, because the role set alone does not cover it.

### Enumerations

One entry lists nine authors whose work makes up a corpus, with two quantities. Either it becomes nine Statements, over-structuring a list the speaker gave as a list, or it becomes one Statement with an opaque list object, which structure cannot query into. Neither is wrong; the model simply has no preference, and the design should state one.

### Quantities carry units in prose

`"100k words"`, `"graded 6/10"`, `"took 5.5 hours"`, `"responds every >10 minutes"`. Each is a typed quantity written as text. The model can hold typed values, but the design currently types only dates and durations. Quantities are a gap of the same kind, discovered the same way.

### Third-party deontics

`"Banned from helping wren with programming, but permitted to make sarcastic comments"`, `"Instructed by wren not to be too sentimental about the cat"`.

These are obligations and permissions binding *someone else*. The design's deontic axis is a Task, defined as something the agent authored for itself, so it deliberately cannot hold them. They are currently recorded as descriptive facts, which is nearly right but loses the modality: a permission is not the same kind of thing as a habit. Expressible as a plain Statement at the cost of flattening deontic force.

## Bucket 3: cannot express

### Referential layering, and it is 39% of the corpus

The largest memory is a third-party persona agent, and its 77 entries mix three distinct referents with nothing marking which is which:

1. **The software.** `"Runs on <model> 4.8"`, `"uses markdown diary entries and grep-based retrieval"`, `"possibly broken due to renamed folders"`. Sixteen entries touch this layer.
2. **The persona.** `"Answers to the name Silvano Ferrer"`, `"admires the single-chamber constitution"`, `"views the revolution as a revolution of proprietors"`. The majority.
3. **The historical person the persona is drawn from.** `"guillotined in 1794 after asking for mercy"`, `"his own calls for mercy were reprinted after his death"`. Twenty-five entries touch this layer, and six mix two layers inside a single entry.

Every one of these is filed as a fact about `person/quill`. Under the Statement model they remain equally undifferentiated: `(person/quill, executed_in, 1794)` is well-typed, passes every proposed critic, and is false about the agent while true about the historical figure. Domain and range checks do not help, because the types are right. This is the different-referent failure the temporal work already fought once, generalised from dates to every predicate, and **the proposed model has no mechanism for it at all**.

The corpus also shows the failure propagating. The persona's operator has a cat. The cat is recorded as a fact about *the persona agent*, in two entries, because the agent talked about it. The cat is real and belongs to the human. A structural model with no layer marker records this error just as faithfully as prose does.

This is not exotic. This instance's social world is largely other persona agents, and any agent that meets characters, bots, or fictional material will hit it. It is the study's most serious finding.

The minimum fix is a **referential frame** on each Statement: whether the claim is made about the entity as a system, as a persona, or about the source the persona derives from. That is a small enum, it composes with the existing critic bank (a frame mismatch between subject and relation is checkable), and it does not exist in the design as written.

### Figurative content

Metaphor, analogy, and reframing appear throughout, and in these entries the figure *is* the content:

> Compared the repeated failure to a blade that keeps rising without falling.

> Reframed the cat's misbehaviour as a citizen occupying the plumbing and issuing decrees.

There is no claim here to extract. `(failure, is_like, blade)` is not what was said and preserves nothing worth keeping. Any structural decomposition destroys the thing being recorded.

This is the one bucket-3 finding that requires no fix, because the design already has the answer: the gloss carries it. It is worth stating plainly that **a real fraction of a personal agent's corpus is content whose only faithful representation is the prose**, since that is an independent argument for the two-traces amendment, arrived at from the corpus rather than from the dual-trace paper.

### Formal content

One topic holds a mathematical counterexample: three polynomial definitions, a determinant, and three specific points that map to a common image. It is recorded as prose containing formulae.

The best available Statement is `(topic/conjecture_counterexample, has_definition, "<the formulae>")`: a typed wrapper around an opaque literal. The structure adds nothing, queries nothing, and checks nothing. The model does not fail here so much as become irrelevant, which the design should admit rather than paper over.

### Directives are not assertions

Twenty-two entries are operating instructions: `"This is a direct message. Be conversational but still concise."`, `"Be laconic, one paragraph at most."`, and the instance's own persona charter, which is several hundred words of voice guidance stored as memory content.

None of these is a claim about the world. They have no teller in any meaningful sense, no truth value, no credence, no validity interval, and no audience posture. Putting them in the same container as facts is a category error the current model makes and the Statement model would inherit unchanged. They want a separate kind, closer to configuration than to memory, and the design does not currently have one.

## The two hypotheses, tested

**Hypothesis 1: the small closed universal role set is too small.** *Not supported.* Every multi-participant happening in the corpus was expressible with agent, theme, source, time, and place. The role set was not the binding constraint. The real gap sits one level up, in relations *between* events, which no number of roles would supply.

**Hypothesis 2: one-level-deep qualifiers are too shallow for compositional phrases.** *Not supported as stated, but a neighbouring problem is real.* Qualifier depth was never the pressure point. The pressure is on the **object** slot, from propositional attitudes needing a Statement where an entity is expected. That is a different axis, and the Wikidata discipline that motivated the original worry does not speak to it.

Both hypotheses were wrong, and the two genuine expressiveness gaps found instead (referential frames, Statement-as-object) were not anticipated by any of the seven research lanes.

## What this changes in the design

1. **Add a referential frame to the Statement.** Required by 39% of the corpus and unaddressed by anything in the current proposal.
2. **Allow a Statement in an object slot**, with bounded depth and a rule that a nested Statement is quoted rather than asserted and is never independently retrievable.
3. **The gloss attaches to the utterance, not the Statement.** Many Statements share one gloss. This corrects the two-traces design and follows directly from compound entries.
4. **Directives are a separate kind**, outside the Statement model, its lifecycle, and its visibility machinery.
5. **Add event-to-event relations** for causation, consequence, and precedence.
6. **Type quantities**, alongside dates and durations.
7. **State a preference on enumerations**, so a list has one canonical representation.

Items 1 and 3 are the load-bearing ones. Item 1 is a genuine addition; item 3 is a correction to a design assumption that the corpus falsified.

## Verdict

The Statement model is **sufficiently expressive to proceed**, with two additions and one correction, none of which disturbs the keystone.

It is not sufficient as written. Had the chapters been drafted before this study, they would have asserted a one-to-one gloss relationship that the data contradicts, and they would have had nothing to say about the largest single category of content in the live corpus.

The three shapes it genuinely cannot express are: figurative content (correctly handled by the gloss, no fix needed), formal content (handled as an opaque literal, an admitted limitation), and directives (a category error, needing a separate kind). None of these is fatal, and all three are better named than quietly accommodated.
