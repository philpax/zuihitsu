# Belief

The initial belief mechanism computes support and corroboration, not truth-directed credence. Support is a versioned projection over visible [Attestations](statements.md), grouped by the [Assertion](statements.md) they support. Proposition, Assertion, and Attestation identity and lifecycle are defined in the canonical assertion model.

This narrower name matters. Provenance, dependence, source reliability, and non-prioritised revision are supported by prior work, but the arithmetic that turns them into a calibrated truth estimate is unsettled. Fusion operators and autonomous contradiction resolution remain gated until multi-teller evidence exists ([research lane](research/2026-07-24/lanes/identity-belief.md); [confidence register](confidence.md#belief)).

## Genesis evidence

The permanent substrate records the inputs future policies may need without fixing their interpretation:

- one stable Attestation per distinct source act, even when teller, Occasion, and Assertion are the same;
- evidence kind, including direct testimony, quotation, operator assertion, agent observation, tool observation, and Derivation;
- expression strength on the Attestation: `hedged`, `plain`, or `emphatic`;
- source locators and the source Occasion or Activity;
- common-source and dependence lineage, including relay through the agent;
- Occasion witness evidence and derived exposure under a named policy;
- teller domain and the domain in which a reliability observation applies;
- append-only reliability observations, including outcome, evaluator, method, and time;
- transmission principle and influence envelope;
- Attestation supersession, retraction, and erasure transitions;
- policy, ontology, unified ResolutionEnvironment, and source head versions.

Reliability is not a mutable score on a person. A policy projects domain-specific reliability from recorded observations. A new policy produces a new projection; it does not reinterpret or rewrite historical Attestations.

## Expression is not corroboration

A hedge belongs to the Attestation because it describes how one teller expressed support. It is not the store's support value.

If one teller first says “probably X” and later says “X,” the store has two Attestations or source acts from one teller, linked as a continuation or refinement. The expression changed from hedged to plain, but there is still one source lineage and no independent corroboration. If two independent tellers each say “probably X,” there are two independent Attestations, both visibly hedged. The support projection must not round their expression into certainty.

This corrects the live-corpus case where a hedged claim and a later flat assertion were stored as unrelated entries. The observation is established; the exact Attestation treatment is this design's synthesis ([modelling study](research/2026-08-03/modelling-study.md#hedges-become-credence)).

## Dependence

Two Attestations that repeat one source are one corroboration lineage, not two independent supports. Dependence is determined from recorded provenance rather than inferred from semantic similarity.

Attestations are dependent when any registered rule establishes a common source, including:

- both derive from the same source Attestation, Assertion, Perception, tool observation, or document passage;
- one teller had exposure to the other's source Occasion before attesting;
- several people repeat a claim in a shared room whose witness evidence shows common exposure;
- the agent restates a received claim;
- the agent relays a claim and a recipient later tells it back;
- an inter-agent quotation preserves an upstream source already represented locally.

Exposure is conservative and can only suppress independence. Channel membership may therefore block a corroboration gain without licensing disclosure. See [witness evidence](privacy-and-provenance.md#witness-evidence).

The initial policy collapses each established dependence component to at most one corroboration contribution. It does not apply a dependent-source fusion formula. Unknown dependence is represented explicitly; high-risk promotion may require demonstrated independence rather than treating unknown as independent.

## Support projection

A support policy is a registered, versioned projection. It has two explicit modes. `promotion_support` may evaluate live candidate or settled Assertions to decide whether a lifecycle transition should be proposed. `actionable_support` accepts only settled Assertions for default conversational rankings, decisions, Derivations, and actions. Both consume audience-visible live Attestations and produce an ordinal plus an evidence account. The first policy is deliberately simple:

1. Resolve audience and subject guards before aggregation.
2. Remove superseded, retracted, or erased Attestations. Apply the mode's Assertion lifecycle filter: live candidate or settled for promotion; settled only for actionable use.
3. Partition remaining Attestations by recorded dependence lineage.
4. Preserve expression strength and evidence kind in the account; do not turn model-stated confidence into weight.
5. Apply only registered, domain-matched reliability observations. Reduced reliability can reduce actionable support or move it to `uncertain`; it never becomes evidence for the opposite proposition.
6. Produce a coarse ordinal such as `single_source`, `corroborated`, or `contested`, with the visible Attestation and independence account that justifies it.

Thresholds, labels, reliability mapping, and promotion criteria are policy data. Every result names its projection mode, policy version, and unified ResolutionEnvironment. Changing arithmetic rebuilds projections and may append promotion or demotion transitions; it does not change persisted semantic content.

The store may maintain a restricted global support projection for operator audit, but conversational rankings, decisions, Derivations, and initiated actions use audience-safe support. Hidden Attestations cannot alter a visible ordinal or ordering. If a computation does use hidden support, its result inherits the hidden influence restriction. See [audience-safe support](privacy-and-provenance.md#audience-safe-support-and-zero-residue).

## Promotion and withdrawal

Candidate and settled are folded Assertion states defined by appended transitions, not intrinsic truth values. A versioned promotion policy may append a promotion transition when its stated support and critic conditions pass. It records the support projection, policy versions, evidence IDs, resolution environment, and source head used.

Withdrawal of an Attestation removes only that teller's support. The fold then recomputes:

- if other live Attestations remain, the Assertion remains with revised support;
- if the last independent support is withdrawn but dependent support remains and settlement criteria no longer hold, policy may append `assertion_demoted`;
- if the last live testimonial support is withdrawn or erased, the Assertion has no testimonial support and is omitted from actionable default reads unless an independently authorised direct Activity or Derivation supports it;
- any promotion or derived result whose criterion no longer holds receives an explicit demotion, invalidation, or recomputation transition; no fold changes another object's lifecycle implicitly.

No source words or prior support projection is edited.

## Contest and contradiction

New information does not automatically win. Opposing Assertions coexist with their own Attestations and audience-safe support. The canonical assertion model defines the mechanical contradiction subset: opposite polarity over one proposition core, functional or exclusive relation conflicts over overlapping validity, mutually exclusive kinds, and incompatible exact or bounded quantities.

Mechanical contradiction classifies a relation between Assertions; it does not decide which is true. Similar, ambiguous, conditional, or context-dependent candidates are merely contested unless the mechanical test passes. The initial support policy can surface both. Autonomous arbitration, general linguistic contradiction detection, and subjective-logic fusion are gated extensions.

This follows non-prioritised belief-revision research and the observed failure of prose arbitration, while the exact mechanical subset and transition policy are design synthesis ([research lane](research/2026-07-24/lanes/identity-belief.md#agm-and-why-a-personal-agent-needs-its-non-prioritised-variants); [current failure](../docs/ontology-failures/2026-07-23.md#belief-has-no-credence-model)).

## Agent-facing surface

The agent sees an ordinal anchored to visible evidence, for example:

> corroborated by two independent visible Attestations; one is hedged

It does not see a model-generated probability or a global count that reveals hidden support. Operator diagnostics may inspect the restricted global projection under authorisation. The conversational surface receives no hidden cardinality, rank shift, unexplained confidence change, or conspicuous gap.

## Deterministic fixtures

Each policy version must pass fixtures that assert both support and privacy outcomes.

| Fixture | Required result |
|---|---|
| Shared room | Three participants repeat one statement after common exposure. The result has one independence lineage, not three. |
| Agent restatement | The agent restates a teller's claim. A new Occasion is recorded, but support does not increase. |
| Relay and return | A tells the agent, the agent tells B, and B later tells it back. B's Attestation retains the relay lineage and is dependent on A. |
| Reliability change | A later reliability observation changes a new support projection under its policy and domain; historical Attestations and old projection records remain unchanged. |
| Hidden Attestation | Adding an undisclosable independent Attestation does not change any visible ordinal, ranking, decision, prompt, or action for the uncleared audience. |
| Last independent withdrawal | Withdrawing the only independent Attestation removes corroborated status even if dependent repetitions remain. |
| Hedge then flat assertion | One teller's later unhedged Attestation changes the expression account, not the independent-source count. |
| Contested candidates | `promotion_support` may compare incompatible live candidates, but `actionable_support` omits them until settled. Non-mechanical candidates remain contested and neither is silently retracted. |
| Direct Activity Assertion | An independently authorised direct observation can support settlement without a human Attestation; its Activity and any required Derivation remain explicit. |
| Attestation erasure | Erasing the only testimonial source removes it from both modes and appends any required Assertion demotion or invalidation; replay exposes no erased payload. |
| Unknown dependence | Unknown dependence does not count as demonstrated independence for a high-risk promotion. |

These fixtures are gates for activating support policy. The first real claim with multiple independent tellers is the forcing condition for evaluating richer fusion; until then, the genesis fields are recorded and the extension remains disabled.
