# Belief

The genesis belief mechanism computes support and corroboration, not truth-directed credence. Support is a versioned projection over visible [Attestations](statements.md#attestation), grouped by the [Assertion](statements.md#assertion) they support. [The object model](statements.md) defines Proposition, Assertion, and Attestation identity and lifecycle.

The narrower name matters. Provenance, dependence, source reliability, and non-prioritised revision are supported by prior work, but the arithmetic that turns them into a calibrated truth estimate is unsettled. Fusion operators and autonomous contradiction resolution stay deferred until multi-teller evidence exists ([research lane](research/2026-07-24/lanes/identity-belief.md); [confidence](program/confidence.md)). No Assertion in the live log has Attestations from two distinct human tellers, so there is currently nothing to fuse. The first real claim with independent Attestations from two tellers is the condition that reopens fusion.

## Genesis evidence

The substrate records the inputs a future policy may need without fixing their interpretation:

- one stable Attestation per distinct source act, even when teller, Occasion, and Assertion are the same;
- the Attestation's typed source: `testimony`, `observation`, or `derivation`, and the Assertion's mode, `asserted` or `quoted`;
- expression strength on a `testimony` Attestation: `hedged`, `plain`, or `emphatic`;
- source locators and the source Occasion or Activity;
- common-source and dependence lineage, including `relayed_from` lineage on a relay and relay through the agent;
- Occasion witness evidence and exposure derived under a named policy;
- the transmission principle, with influence computed from [context manifests](privacy-and-provenance.md#influence-from-context-manifests);
- Attestation supersession, retraction, invalidation, and erasure transitions;
- for a `derivation` Attestation, its policy and ontology versions and the frontier it read.

Only `testimony` contributes teller support or corroboration. An `observation` or `derivation` Attestation is direct support: it can make an Assertion usable, but it never counts as an independent teller.

Reliability observations are deferred. A later policy may record outcome observations with evaluator, method, time, and domain, and project domain-specific reliability from them. Reliability is then never a mutable score on a person, and a new policy produces a new projection without rewriting historical Attestations. Adding the observation records is additive.

## Expression is not corroboration

A hedge belongs to the Attestation because it describes how one teller expressed support. It is not the store's support value.

If one teller first says "probably X" and later says "X", the store has two Attestations from one teller, linked as a continuation or refinement. The expression changed from hedged to plain, but there is still one source lineage and no independent corroboration. If two independent tellers each say "probably X", there are two independent Attestations, both visibly hedged. The support projection does not round their expression into certainty.

This corrects the live-corpus case where a hedged claim and a later flat assertion of the same fact were stored as unrelated entries, both alive and unlinked. The observation is established. The Attestation treatment is this design's synthesis ([modelling study](research/2026-08-03/modelling-study.md#hedges-become-credence)).

## Dependence

Two Attestations that repeat one source are one corroboration lineage, not two independent supports. Dependence is determined from recorded provenance, never inferred from semantic similarity.

Attestations are dependent when a registered rule establishes a common source, including when:

- both derive from the same Attestation, Assertion, Perception, tool observation, or document passage;
- one teller had exposure to the other's source Occasion before attesting;
- several people repeat a claim in a shared room whose witness evidence shows common exposure;
- a relay's `relayed_from` lineage names the source person of another Attestation for the same content;
- the agent restates a received claim;
- the agent relays a claim and a recipient later tells it back.

A relay is a participant repeating another person's words: Rowan says "Quinn told me that X". The teller of a `testimony` Attestation is always the participant who produced the cited span, so the relay is a `quoted` Assertion with Rowan as teller, plus `relayed_from` lineage naming Quinn as the original source person ([write surface](write-surface.md)). Quinn is not a teller of the relay and has no teller rights over it ([privacy and provenance](privacy-and-provenance.md#testimony-principle-floor)). When Quinn later tells X directly, the flat `asserted` Assertion records corroboration lineage to the quoted one, and the `relayed_from` lineage places both Attestations in one dependence component. Two relays that name the same source person are likewise one lineage.

The agent is a witness to what it is told and never an independent teller of it. In the live corpus 77 of 198 entries are agent-told, many restating a participant's own sentence, so without this rule a sentence read back into the store would count as corroboration from its own source ([confidence](program/confidence.md)). The agent's utterances are outbound Occasions and never testimony by the agent about what it relays. Their recipients carry witness evidence of at least `delivered`, which is what lets the fold recognise a relayed claim that returns: when B, who received the agent's relay of A's claim, later tells it back, B's Attestation is dependent on A's. That mechanism is untested.

Exposure is conservative and can only suppress independence. Channel membership can therefore block a corroboration gain without licensing disclosure ([witness evidence](privacy-and-provenance.md#witness-evidence)).

The genesis policy collapses each established dependence component to at most one corroboration contribution. It applies no dependent-source fusion formula. Unknown dependence is represented explicitly and never counts as demonstrated independence, so a criterion that requires corroboration, such as a later high-risk action policy, does not treat unknown as independent.

## Support projection

A support policy is a registered, versioned projection. It consumes the audience-visible live Attestations of an Assertion and produces an ordinal with an evidence account. The genesis policy is deliberately simple:

1. Resolve the audience and the [subject guard](privacy-and-provenance.md#subject-guard) before aggregation.
2. Remove superseded, retracted, invalidated, and erased Attestations.
3. Partition the remaining `testimony` Attestations by recorded dependence lineage.
4. Keep expression strength and source kind in the account. Model-stated confidence never becomes weight.
5. Produce a coarse ordinal, `single_source`, `corroborated`, or [`contested`](#contest-and-contradiction), with the visible Attestation and independence account that justifies it.

Thresholds, labels, and eligibility criteria are policy data. A result is a function of the frontier and the audience, and the policy version in force follows from the log at that frontier. Changing the arithmetic rebuilds projections and never changes persisted content.

The store may keep a restricted global support projection for operator audit. Conversational rankings, decisions, derived results, and initiated actions use audience-safe support only ([zero residue](privacy-and-provenance.md#audience-safe-state-and-zero-residue)).

## Settlement and withdrawal

Settlement is a read-time projection, not a stored state. An Assertion is settled for an audience when eligible support is visible to that audience after the subject guard. Eligible support is:

- a live `testimony` Attestation, so one teller's lineage suffices;
- a direct operator `observation`;
- a direct agent `observation`;
- a `derivation` whose inputs are all settled for the same audience.

No transition records settlement or demotion, and no authority class appends one. Settlement means that default reads return the Assertion. It says nothing about corroboration, which the ordinal reports. Default conversational reads consider only Assertions settled for their audience, and explicit candidate inspection also returns the rest. A fact one teller told yesterday is therefore recallable today. Without this rule the agent could not recall any claim in the live corpus, since none has two human tellers. A hidden Attestation cannot make an Assertion default-readable for an audience that cannot see it.

Withdrawing, retracting, or erasing an Attestation removes only that source's support. The next read recomputes:

- if other eligible support remains visible, the Assertion stays settled with revised support;
- if none remains, default reads omit the Assertion;
- if the last independent support goes while dependent support remains, the Assertion stays settled and loses `corroborated`;
- a derivation whose inputs are no longer all settled stops being settled, and a `support_weakened` mark queues its re-evaluation ([queue marks](off-turn.md#queue-marks)).

No projection changes an object's lifecycle, and no source words or earlier projection are edited.

## Contest and contradiction

New information does not automatically win. Opposing Assertions coexist, each with its own Attestations and audience-safe support. [The object model](statements.md#mechanical-contradiction) defines the mechanical subset: opposite polarity over one proposition core, functional or exclusive relation conflicts over overlapping validity, mutually exclusive kinds, and incompatible exact or bounded quantities. A relation's functional or exclusive cardinality counts only after the operator confirms it ([relations](relations.md)).

Mechanical detection runs at publication, and on arrival of any Assertion whose core matches a registered rule. Each detection appends a `contradiction_detected` queue mark ([queue marks](off-turn.md#queue-marks)). The `contested` ordinal means that a live mechanical contradiction is visible to the audience: both Assertions are visible to it. A contradiction with one hidden side leaves no ordinal. Contest does not block settlement: each side settles on its own support, neither is retracted, and default reads return both with the ordinal and each side's visible evidence. [Dormant contested items](off-turn.md#dormant-contested-items) are mechanical contests only.

Mechanical contradiction classifies a relation between Assertions. It does not decide which is true. Non-mechanical contest detection, over similar, ambiguous, conditional, or context-dependent Assertions, is deferred, and such Assertions coexist without the ordinal. Autonomous arbitration and subjective-logic fusion are also deferred.

This follows non-prioritised belief-revision research and the observed failure of prose arbitration in the current system, where contradictions persist as flags and arbitration is episodic model judgement recorded as a prose note. The mechanical subset and transition policy are design synthesis ([research lane](research/2026-07-24/lanes/identity-belief.md#agm-and-why-a-personal-agent-needs-its-non-prioritised-variants); [current failure](../docs/ontology-failures/2026-07-23.md#belief-has-no-credence-model)).

## Agent-facing surface

The agent sees an ordinal anchored to visible evidence, for example:

> corroborated by two independent visible Attestations; one is hedged

It does not see a model-generated probability or a global count that reveals hidden support. Verbalised model confidence is overconfident, saturated, and protocol-sensitive, so the surface never presents one ([research lane](research/2026-07-24/lanes/identity-belief.md#calibration--the-neural-writers-confidence-cannot-be-trusted-raw)). Operator diagnostics may inspect the restricted global projection under authorisation. The conversational surface receives no hidden cardinality, rank shift, unexplained confidence change, or conspicuous gap.

## Scenarios

| ID | Scenario | Expected result |
|---|---|---|
| `shared-room-dependence` | Three participants repeat one statement after common exposure in a room. | One dependence component; the ordinal is `single_source`. |
| `agent-restatement` | The agent restates a teller's claim. | An outbound Occasion is recorded, no Attestation is added, and the ordinal is unchanged. |
| `relay-and-return` | A tells the agent, the agent tells B, and B later tells it back. | B's Attestation is in A's dependence component through the outbound Occasion; the ordinal stays `single_source`. |
| `relay-not-corroboration` | Rowan says "Quinn told me that X"; Quinn later tells X directly. | A `quoted` Assertion with Rowan as teller and `relayed_from` Quinn, and an `asserted` Assertion from Quinn, in one dependence component; neither is `corroborated`. |
| `hidden-support` | An independent Attestation hidden from the audience is added. | The audience's ordinal, ranking, prompt, and actions equal those without it. |
| `single-teller-settles` | One teller states a fact; the next day the agent is asked about it. | The default read returns it as settled; the log holds no settlement transition. |
| `settlement-audience-scoped` | An Assertion's only eligible support is hidden from the audience; a visible `derivation` with an unsettled input also supports it. | The default read omits it, and the response matches a store without the hidden Attestation. |
| `last-support-withdrawn-omitted` | A single-teller Assertion's only Attestation is retracted. | No transition is appended to the Assertion; the next default read omits it, and candidate inspection returns it. |
| `last-independent-withdrawal` | The only independent Attestation is withdrawn while dependent repetitions remain. | The Assertion stays settled; the ordinal drops from `corroborated` to `single_source`. |
| `derivation-follows-inputs` | A derivation's one unsettled input later gains visible testimony. | Before, default reads omit the derivation; after, they return it as settled, with no appended transition. |
| `hedge-then-flat` | One teller says "probably X" and later "X". | Two Attestations in one dependence component; the account shows `hedged` then `plain`; the ordinal stays `single_source`. |
| `contested-both-surfaced` | Rowan and Sam publish opposite-polarity Assertions over one core and overlapping validity. | Publication appends `contradiction_detected`; a read for an audience seeing both returns both, each settled and `contested`. |
| `contest-hidden-side` | One side of a mechanical contradiction is hidden from the audience. | The visible side is returned without `contested`. |
| `non-mechanical-no-contest` | Two incompatible Assertions match no registered rule. | No mark is appended; both are returned without `contested`. |
| `direct-observation-support` | An authorised `observation` Attestation supports an Assertion with no human testimony. | Default reads return it as settled; the account names no teller, and the ordinal is not `corroborated`. |
| `attestation-erasure` | The only testimonial Attestation is erased. | Default reads omit the Assertion, and replay exposes no erased payload. |
| `unknown-dependence` | Two Attestations have unknown dependence, and a criterion requires corroboration. | The criterion fails; the ordinal is not `corroborated`. |
