# Relations

A relation definition is a versioned ontology object under a stable definition ID. A relation instance is represented by a Proposition and situated Assertion; [statements](statements.md) owns their identity, provenance, validity, audience, and lifecycle. This chapter owns relation-definition constraints and schema evolution.

The case for domain/range constraints and deprecate-and-alias is convergent rather than speculative: the research review found reversed extraction errors and independent production systems that normalised drifting edge vocabularies ([research report](research/2026-07-24/report.md#33-relations-attribute-bearing-interval-scoped-schema-evolving), [issue 7 survey](research/2026-07-24/lanes/survey-issue7.md)). The exact historical-version and governance rules below are permanence-driven design decisions, prompted by the current immutable-schema failure ([ontology failures](../docs/ontology-failures/2026-07-23.md#relation-schemas-are-immutable-and-vocabulary-drifts)) and unresolved migration behaviour ([limitations](../docs/limitations.md)).

## Definition records

Every version declares, where applicable:

- stable relation ID and monotonically ordered definition version;
- human description and canonical display name;
- inverse or symmetry rule;
- cardinality or functional/exclusive constraints;
- subject domain and object range;
- applicable referential frames;
- allowed modalities and value shapes;
- lifecycle status: active, deprecated, or aliased;
- activation authority and provenance.

Domain, range, frame, and value constraints make reversed or mistyped proposals mechanically rejectable. They do not establish truth. A well-typed Assertion may still be false, ambiguous, or poorly grounded.

```text
relation/worked_at @ v3
  inverse: relation/employed
  cardinality: many-to-many
  domain: kind/person
  range: kind/organisation
  frames: system
```

Validity belongs to the Assertion, not to the relation definition. “Worked at Northwind from 2019 to 2021” therefore uses one Proposition over `worked_at` and a temporally situated Assertion. A correction appends an Assertion transition under the lifecycle in [statements](statements.md); it does not mutate the relation instance or source Occasion.

## All ontology definitions are versioned

The same registration discipline applies to relation definitions, entity kinds, universal and typed Event roles, Event types, referential frames, modalities, transmission principles, and critic definitions. Each has a stable ID and immutable versions. A new version may clarify presentation or tighten future acceptance, but it cannot repurpose the ID.

Every accepted Assertion records the definition versions against which it was accepted. Replay interprets and validates it under those versions. Changing a domain, range, frame applicability, role filler constraint, or critic does not silently rejudge historical Assertions. A current projection may separately report that an old Assertion would fail current policy, but that is a versioned derived result, not a rewrite of history.

If a change would alter the meaning of an existing definition rather than extend or correct its future use, mint a new stable ID and relate the definitions explicitly. Historical input never receives fields or meanings it did not carry.

## Deprecation and aliasing

Deprecation stops a definition being preferred for new writes but preserves its interpretation. Aliasing records that reads may expand one relation to another canonical relation under a named alias-policy version:

```text
relation/event_at    deprecated; alias_to relation/located_at
relation/happened_at deprecated; alias_to relation/located_at
relation/located_at  active
```

Alias resolution is non-destructive. Historical Propositions retain their original relation ID and definition version; expanded query results report both the stored and resolved IDs. Activation rejects alias cycles, self-aliases, incompatible endpoint types, and aliases whose semantic narrowing would make expansion unsound. Transitive expansion is allowed only through an acyclic, validated chain.

An alias is not a claim that two definitions were always identical. Where old usage was mixed or narrower, keep the old definition deprecated and use an explicit mapping Derivation or manual review rather than an alias.

## Context does not become hidden schema

A relation Assertion may cite source text and may retain unstructured context through its source locator or an explicitly non-queryable annotation. This is an escape hatch for nuance, not a second relation language. Critics and queries cannot infer structural semantics from that text. If repeated context becomes query-relevant, it requires a governed definition proposal.

## Schema governance

The conversational agent may propose a missing relation, kind, role, Event type, frame, modality, transmission principle, or critic definition. A proposal records examples, intended constraints, parent definitions, and the proposing Occasion or Activity. It remains inactive and cannot make a `claim` valid.

Activation, deprecation, aliasing, and version publication are governed operator or delegated schema-authority operations. They run structural checks, including alias-cycle detection, compatibility analysis, fixture replay, and collision checks against active names. The operation records who authorised it and the ontology head it extends.

This boundary prevents `claim` from carrying hidden ontology language. A write either uses active definition versions or remains a source-only/proposal result. The agent cannot smuggle a one-off predicate into a relation name or activate its own proposal as a side effect of recording content.

## Seed vocabulary and extension

Genesis includes only definitions required by permanent mechanics and the first policy: identity, participation, composition, placement, origin, operatorship, presentation, and the universal Event-role parents. Social and environmental concepts can be proposed as experience demands, but they follow the same governed path.

The seed is minimal to avoid encoding designer assumptions as world knowledge. Minimal does not mean mutable or informal: all identity-bearing definition slots and their version semantics exist at genesis, including modalities and transmission principles whose richer policies remain disabled.

## Event endpoints

Relations may accept [Events](events-and-roles.md) as subject or object. Causation, consequence, and precedence use ordinary Propositions and Assertions with Event-compatible domain/range definitions. Event co-reference never rewrites these endpoints; a query through a composite Event view is a resolution-environment projection and reports the source Event IDs.

## Required fixtures

- A relation accepted under `v1` remains readable under `v1` after `v2` narrows its range; a current-policy diagnostic may flag it but cannot retract it.
- A four-name vocabulary drift is collapsed by acyclic aliases without rewriting stored Propositions, and query output identifies alias expansion.
- An attempted alias cycle and an alias between incompatible endpoint types are rejected before activation.
- The agent proposes a missing `approved_by` definition during a conversation. The source Occasion remains durable, but no structural Assertion using that relation becomes accepted until governance activates a version.
- Deprecating a typed Event subrole leaves historical role Assertions and generic parent-role traversal intact.
