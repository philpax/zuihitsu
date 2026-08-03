# Events and roles

A happening is one node. The participants hang off it as role-edges, and each edge is an independently addressable [Statement](statements.md).

```
e1  event/create
    agent   person/wren
    theme   person/quill
    source  topic/instance_architecture
    time    [2026-07-14, 2026-07-16)
```

The alternative, which the current system is stuck with, is that a fact binds to exactly one subject, so a happening with three participants shatters into three subject-appropriate rephrasings. That failure is not hypothetical: [the corpus study](research/2026-08-03/modelling-study.md) found one happening recorded four times in a single session, once on each participant and once on a topic abstracting the pair. Under this model those four writes resolve to one Event, and the second and third arrivals are structural no-ops with nothing to rephrase.

## The role set is small and closed

Roles come from a fixed universal set: `agent`, `theme`, `instrument`, `source`, `recipient`, `time`, `place`, and a small handful more. The set is closed, and extending it is a change to the model rather than something the agent does at runtime.

Everything else is an **attribute on the Event**, not another role. An event's outcome, its manner, its mood, its cost: all attributes.

The reason for the restraint is that role assignment beyond agent and theme is genuinely hard. Expert annotators disagree on the tail, numbered role inventories are documented as inconsistent above the first two positions, and frame-specific inventories running to the low thousands are unteachable to a writer working one utterance at a time. A small universal set is learnable; a large one produces confident, inconsistent labelling that later reads as structure.

The corpus study tested this directly and found the role set was **not** the binding constraint: every multi-participant happening observed was expressible with agent, theme, source, and time. The pressure the study did find is one level up, and is handled below.

When the right role is genuinely unclear, the writer says so rather than guessing, and the [gloss](two-traces.md) preserves the surface form. A hedged role is recoverable; a confidently wrong one is not.

## Events relate to other events

Roles place participants inside an event. They say nothing about how one event stands to another, and real recorded content needs that constantly: one thing sparks another, follows another, or is a consequence of another.

Event-to-event relations carry this, using the ordinary [relation](relations.md) machinery:

```
e1  event/create      (as above)
e2  event/project_run
    agent  person/wren
    time   [2026-07-16, 2026-08-14)

s20 (e1, sparked, e2)
```

This was the one gap the corpus study found in the event model, and it is small: the relation layer already supports it, and the fix is to register the relations and permit an Event as an endpoint. Without it, causation and consequence fall back into prose, which is exactly the failure the Event node exists to end.

## Attributes and roles are both Statements

Each role-edge and each attribute is a Statement in its own right, which means each carries its own provenance, its own credence, and its own transmission principle.

This composes with the audience model in a way a single prose sentence cannot. One participant may have told the agent who did the thing, another when it happened, and a third something they said in confidence about why. Those are three Statements against one Event, with three tellers and three audience conditions. Rendering the event to an audience assembles only the edges that audience may see, so the same Event reads differently to different people without any duplication.

The same property is what makes an event correctable. Learning that the instrument was misattributed changes one edge. Under prose it would mean rewriting a whole sentence and losing everything else the sentence carried.

## Resolving a re-mention

When a write arrives describing a happening the store already holds, it resolves to the existing Event rather than creating a second one. The test is structural: same event type, same agent and theme, overlapping time. A match adds any new edges the arriving description carried and records the new teller against the existing Statements.

What this does **not** do is discard the occasion. The re-mention keeps its own gloss and its own turn reference, so the Event accumulates the occasions on which it was discussed while holding one copy of what happened. Deduplicating the claim and preserving the episode are different operations, and the boundary between them is stated in [the two traces](two-traces.md).

The failure mode to watch is a resolution that is too eager: two genuinely distinct happenings of the same type between the same people, collapsed because they overlap in time. The critic bank treats a resolution as a rejectable proposal like any other write, and an ambiguous match is a teachable error rather than a silent merge.
