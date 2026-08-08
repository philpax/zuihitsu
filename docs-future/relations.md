# Relations

A relation is a first-class object with a definition, a lifecycle, and constraints. An instance of a relation is a [Statement](statements.md), which means it carries everything a Statement carries: provenance, credence, validity, audience, and frame.

## A relation instance is not a bare edge

Today an edge carries a name, a posture, and nothing else, so anything time-bounded or qualified degrades into prose and re-enters the facts-are-sentences failure. Under this model:

```
s12 (person/rowan, worked_at, org/northwind)
    valid     [2019-03, 2021-06)
    told_by   person/rowan
    credence  confirmed
    frame     actual
```

"Worked at Northwind from 2019 to 2021" is a relation, not a sentence about a relation. Supersession closes the window rather than deleting the edge, so the employment history remains readable and "where did they work in 2020" is answerable.

## Definitions declare domain and range

Registering a relation requires its inverse, its cardinality, and the types its endpoints accept.

```
worked_at
  inverse      employed
  cardinality  many-to-many
  domain       person
  range        organisation
  description  Employment, bounded by the validity interval.
```

Domain and range are what make a reversed or mistyped edge a rejectable write rather than a silently stored one. Reversed relationships are a documented, common extraction failure, and the current system's link graph shows it plainly: it contains edges asserting that a person was created by the agent alongside the correct inverse, that a room operates a person, and that an event participates in a person. Every one of those is well-formed as a bare edge and nonsense against a declared range.

The [frame](statements.md) participates too. A relation may declare which referential layers it applies in, so a claim about a historical source cannot attach to the software presenting the persona.

## Vocabulary evolves by deprecation and aliasing

A relation definition is not frozen at first use. It carries a lifecycle:

```
event_at        deprecated, aliased_to: located_at
happened_at     deprecated, aliased_to: located_at
located_at      active
```

Reads resolve aliases transitively, so a query for `located_at` returns everything written under any of its aliases, and the drift collapses at read time without rewriting history. The append-only log is untouched: an alias is a forward event like any other.

This exists because coinage drift is not a hypothetical. The current system has the same semantic relation coined four different ways across sessions, with nothing structural to prevent a fifth, and a registered relation that cannot be amended, only abandoned. Two independent production systems reached the same fix shape from the same problem: a closed vocabulary, a single canonical form, and a deterministic migration.

Deprecation is not deletion. A deprecated relation keeps working, keeps its history, and keeps resolving. It simply stops being the form a new write should use, and the critic bank says so as a teachable error.

## The free-text channel

Every relation instance may carry a free-text `context` field.

The field is what keeps the closed vocabulary honest. Without it, the pressure to express a nuance the vocabulary lacks goes into the relation name, which is how a vocabulary drifts into hundreds of one-off strings. One surveyed production system found 78% of its edges were single-use free-text relations before it normalised them.

With the field, the nuance goes somewhere that is explicitly unstructured, and the relation stays canonical. The `context` field is never queried structurally and never resolved. It is read by a person or rendered to the agent, and nothing depends on its shape.

## The seed vocabulary stays minimal

The relations present at genesis are the structural universals the system itself depends on: identity, participation, composition, placement, origin, operatorship, presentation, and acquaintance.

Presentation is `presents`, from a principal to a persona they run. It is seeded rather than coined because the [frame's redirect](statements.md) resolves against it at write time, and a structural mechanism cannot wait on the agent to invent the relation it depends on. It sits beside operatorship, which is its nearest neighbour and equally structural.

Social and environmental semantics are the agent's to coin at runtime. That has not changed, and it should not: an ontology preloaded with what the designers thought mattered is an ontology that constrains what the agent can notice. What has changed is that coinage now happens against a schema that can catch misuse, and that a mis-coined relation is repairable rather than permanent.

## Event endpoints

Relations hold between entities, and also between [Events](events-and-roles.md). Causation, consequence, and precedence use the ordinary machinery with an Event at one or both ends. Their definitions declare event types as domain and range in the same way, so a causal claim between two incompatible event kinds is checkable.
