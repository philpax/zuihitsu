# Relations

A relation definition is a versioned ontology object under a stable definition ID. A relation instance is a Proposition situated in an Assertion. [The object model](statements.md) owns their identity, provenance, validity, audience, and lifecycle. This chapter owns definition constraints, how the agent coins relations, and schema evolution.

The case for domain and range constraints and for deprecate-and-alias is convergent, not speculative. The research review found reversed extraction errors, and independent production systems normalised drifting edge vocabularies ([research report](research/2026-07-24/report.md#33-relations-attribute-bearing-interval-scoped-schema-evolving); [issue 7 survey](research/2026-07-24/lanes/survey-issue7.md)). The current system's immutable relation schema and unresolved migration behaviour prompted the versioning rules ([ontology failures](../docs/ontology-failures/2026-07-23.md#relation-schemas-are-immutable-and-vocabulary-drifts); [limitations](../docs/limitations.md)).

## Definition records

Every definition version declares, where applicable:

- the stable relation ID and a monotonically ordered version;
- a description and a canonical display name;
- an inverse or symmetry rule;
- a cardinality or functional or exclusive constraint;
- the subject domain and object range;
- the applicable referential frames;
- the allowed modalities and value shapes;
- at least one example, written over `example/` placeholder handles;
- a status: `provisional`, `established`, `deprecated`, or `aliased`;
- the authority and the Activity or Occasion that registered it.

Domain, range, frame, and value constraints make reversed or mistyped writes mechanically rejectable. They do not establish truth: a well-typed Assertion may still be false or poorly grounded.

```text
relation/worked_at @ v3
  inverse: relation/employed
  cardinality: many-to-many
  domain: kind/person
  range: kind/organisation
  frames: system
```

Validity belongs to the Assertion, not to the definition. "Worked at Northwind from 2019 to 2021" is one Proposition over `worked_at` in a temporally situated Assertion.

## All ontology definitions are versioned

The same registration discipline applies to relations, entity kinds, universal and typed Event roles, Event types, referential frames, modalities, transmission principles, and critic definitions. Each has a stable ID and immutable versions. A new version may clarify presentation or tighten future acceptance. It never repurposes the ID.

A Proposition key holds definition IDs only, never versions. Every accepted Assertion records the definition versions it was accepted against, and replay validates it under those versions. Changing a domain, range, frame applicability, role filler constraint, or critic does not rejudge historical Assertions. A current projection may report that an old Assertion would fail current policy. That report is a versioned derived result, not a rewrite.

A change that alters the meaning of a definition, rather than extending or correcting its future use, mints a new stable ID and relates the two definitions explicitly. Historical input never acquires fields or meanings it did not carry.

## Governance

Relations are the agent's to coin. The seed carries only the structural universals the system relies on, and social and environmental semantics grow at runtime, as the current [seed ontology principle](../CONTRIBUTING.md#the-seed-ontology) states.

Every other definition kind stays operator-governed: entity kinds, roles and subroles, Event types, frames, modalities, transmission principles, and critics. The agent may propose one of these with examples, intended constraints, and parent definitions. The proposal stays inactive until the operator publishes a version, and it cannot make a write valid. The operator's publication runs structural checks, including collision checks against active names and scenario replay, and records the authority and the frontier it extends.

## Coining under critics

Agent coinage risks vocabulary drift. The peer survey found one production system in which 78% of edges were one-off free-text relations before normalisation, and both structurally serious peers converged on a closed, migratable vocabulary ([issue 7 survey](research/2026-07-24/lanes/survey-issue7.md)). Coinage under critics is a design synthesis that keeps open coinage and targets that drift with five layers. It is unvalidated until the Evidence milestone in [evolution](program/evolution.md) measures it on the real corpus.

1. Coining requires a description, a domain, a range, and an example over `example/` placeholder handles ([definition audience](#definition-audience)). A write naming an unregistered relation gets a teachable error asking for these. The description is embedded, and the embedding model version is recorded with the definition.
2. A new relation whose name or description embeds near an existing definition with compatible endpoints is rejected with a teachable error naming that definition. The agent either uses the existing relation or restates the coinage with an explicit account of how its relation differs. The distance threshold is policy-defined. Only definitions visible to the writing context's audience are compared, so the error never names a restricted coinage.
3. At the point of writing, the context renders the relations in use for each entity kind involved, so the agent reuses before it coins. The list is computed per entity kind, not per entity, from Assertions visible to the writing context's audience.
4. A new relation is `provisional` until Assertions using it are attested across a policy-defined number of distinct Occasions, after which it becomes `established`. A provisional relation is usable immediately. A background job may alias a provisional relation into an established one without the operator, subject to the alias checks below. Aliasing or versioning an established relation requires the operator.
5. A declared functional or exclusive cardinality is recorded at coinage but feeds [mechanical contradiction](belief.md#contest-and-contradiction) detection only after the operator confirms it. Until then the relation behaves as many-to-many for contradiction purposes.

The first two layers and the definition-audience check are hard critics in the [verified write](verified-write.md#hard-critics). The job in the fourth layer runs under the [off-turn](off-turn.md) authority classes, and its alias is an ordinary recorded, reversible definition transition. Because aliasing is non-destructive, a wrong automatic alias costs one operator correction and no data.

## Definition audience

A coined relation renders only to audiences that clear its coining context until it is used in a clearing Occasion or the operator publishes it, under the coined-name rule in [privacy and provenance](privacy-and-provenance.md#influence-from-context-manifests). A parallel coinage in a wider context is a candidate for aliasing. Once cleared, a definition is public vocabulary, so its description and example must not contain entity handles or Occasion content. The example uses placeholder handles from the reserved `example/` namespace:

```text
relation/mentored @ v1
  description: a person guided another person's professional development
  domain: kind/person
  range: kind/person
  example: example/person-a mentored example/person-b
```

A hard critic rejects a live handle, one that resolves to an entity, in the description or the example, with a teachable error naming the `example/` namespace. The critic cannot detect paraphrased private content, such as a description that restates a confidence without naming anyone. A soft critic and operator audit sampling cover that case.

## Deprecation and aliasing

Deprecation stops a definition being preferred for new writes and preserves its interpretation. An alias records that reads may expand one relation to another canonical relation under a named alias-policy version:

```text
relation/event_at    deprecated; alias_to relation/located_at
relation/happened_at deprecated; alias_to relation/located_at
relation/located_at  established
```

Aliases resolve on read. Historical Propositions keep their original relation ID, their Assertions keep the definition version they were accepted under, and expanded query results report both the stored and the resolved IDs. Every alias, whether operator-made or automatic, is rejected if it creates a cycle, aliases a definition to itself, joins incompatible endpoint types, or narrows semantics so that expansion would be unsound. Transitive expansion follows only an acyclic, validated chain.

An alias does not claim that two definitions were always identical. Where old usage was mixed or narrower, the old definition stays deprecated, and an explicit mapping derivation or operator review replaces the alias.

## Context does not become hidden schema

A relation Assertion may cite source text and may keep unstructured context through its source locator or an explicitly non-queryable annotation. This is an escape hatch for nuance, not a second relation language. Critics and queries never infer structural semantics from that text. Context that repeatedly matters to queries becomes a coined relation, with its critics, rather than a convention inside annotations. A relation name that smuggles a one-off predicate is caught by the same critics as any other coinage.

## Seed vocabulary

Genesis includes only definitions that permanent mechanics and the first policies need: identity, participation, composition, membership, placement, origin, operatorship, acquaintance, the `presents` relation that the [`principal` redirect](statements.md#proposition) uses, and the universal Event-role parents. The exact manifest is decided before genesis. A missing or ambiguous seed leaves a dependent write source-only. It never lets an unregistered relation through.

The seed is minimal so that designer assumptions are not encoded as world knowledge, but not informal: every identity-bearing definition slot and its version semantics exist at genesis, including modalities and transmission principles whose richer policies are deferred.

## Event endpoints

Relations may take [Events](events-and-roles.md) as subject or object. Causation, consequence, and precedence are ordinary Propositions and Assertions under Event-compatible domain and range definitions. Event co-reference is deferred, so each duplicate Event keeps its own endpoints.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `relation-historical-version` | An Assertion accepted under `worked_at@v1` is read after `v2` narrows its range. | It stays live under `v1`. A current-policy diagnostic reports it, and no transition is appended. |
| `relation-reversed-endpoints` | A write puts an organisation in the subject of `worked_at`. | Rejected by the domain check with a teachable error. |
| `relation-coinage-incomplete` | The agent writes with an unregistered `mentored` relation and supplies no description. | A teachable error asks for a description, domain, range, and example; no Assertion is accepted. |
| `relation-coinage-accepted` | The agent coins `approved_by` with all required fields, then writes an Assertion using it. | The definition is `provisional`; the Assertion is accepted. |
| `example-handle-rejected` | The agent coins a relation whose example reads `person/rowan mentored person/quinn`. | A teachable error names the `example/` namespace; the definition is not registered. |
| `restricted-coinage-withheld` | The agent coins `mentored` in a context that rendered Quinn's confidence, then writes in a public channel. | The public context's in-use list and near-duplicate errors omit `mentored`. After the operator publishes it, both include it. |
| `near-duplicate-relation-teaches` | The agent coins `employed_at` with person and organisation endpoints while `worked_at` exists. | A teachable error names `worked_at`; the agent reuses it or states the difference. |
| `relation-in-use-rendered` | The agent writes about a person in a channel, and person-kind Assertions use `worked_at` publicly and `mentored` only under a confidence. | The write context's person-kind list includes `worked_at` and omits `mentored`. |
| `provisional-relation-auto-alias` | A provisional `event_at` sits near established `located_at` with compatible endpoints. | The background job aliases it; stored Propositions keep `event_at`; reads expand to `located_at` and report both. |
| `relation-promotion` | A provisional relation is used across the policy-defined number of Occasions. | It becomes `established`; later aliasing requires the operator. |
| `relation-alias-drift` | Four drifting names for one concept exist. | Acyclic aliases collapse them without rewriting stored Propositions; query output identifies the expansion. |
| `relation-alias-cycle-rejected` | An alias would close a cycle, or join incompatible endpoint types. | Rejected before it takes effect, whether operator-made or automatic. |
| `relation-cardinality-unconfirmed` | The agent coins a functional relation, and two conflicting values are asserted. | No `contradiction_detected` mark is appended until the operator confirms the cardinality. |
| `relation-context-not-schema` | An Assertion's annotation says "only on weekends". | Queries and critics ignore the annotation structurally; the text stays at the source locator. |
| `subrole-deprecation` | The operator deprecates a typed Event subrole. | Historical role Assertions and generic parent-role traversal remain intact. |
