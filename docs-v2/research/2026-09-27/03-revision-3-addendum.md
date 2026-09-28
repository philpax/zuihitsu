# docs-future revision 3: final paper-pass addendum

This addendum extends `simplification-brief.md` and `revision-2-addendum.md` in the same directory. Both still apply where this addendum is silent. Where they disagree, this addendum wins. The operator approved everything below. It is the last paper revision. The next check is an executable reference model, so this pass must leave the tree smaller and easier to implement, not larger.

## Consolidation and size (applies to every agent)

Rules are currently restated in six to eight places each, and every copy is a drift point. In this pass each rule is stated once, in the chapter that owns it. Every other mention becomes a one-sentence pointer with a link. Scenario rows state testable results: name what is returned, rejected, or recorded, and where. A vague result such as "reported as a contradiction" is not testable.

Owners:

| Rule | Owner |
|---|---|
| Object definitions, lifecycles, and legal transitions | statements.md |
| Occasion restriction, delivered-audience restriction, the testimony principle floor, the pre-delivery check, the subject guard, zero residue, influence projections, and erasure | privacy-and-provenance.md |
| Testimony-grounding and the other hard critics, as mechanics | verified-write.md |
| The settlement projection, dependence, and contest | belief.md |
| Identity hypotheses and clearance | identity.md |
| Task actions and time values | time.md |

Size targets:
- The whole tree, excluding `research/`: at most 300KB. It is about 364KB now.
- statements.md: at most 35KB.
- privacy-and-provenance.md: at most 35KB.
- evolution.md: at most 30KB.
- confidence.md: at most 22KB.
- coverage.md: at most 20KB.
- Every other chapter at or below its current size.

Measure with `wc -c` before reporting. Meet the targets by removing restatement, not substance.

## Privacy foundations

### C1. Inbound Occasion restriction and the pre-delivery check

- Every inbound Occasion has a restriction: its availability audience at receipt.
  - A DM's restriction is its parties.
  - A channel's restriction uses a new principle, `channel(C)`. Its members are those of channel C under the connector roster. The connector declares whether the platform shows history to members who join later. If it does, membership is evaluated at read time. If it does not, the roster at receipt applies.
  - An inbound Occasion rendered into a context contributes this restriction. It is no longer "teller-only for its supplier".
- The pre-delivery check is defined here. Before an outbound Occasion is delivered, the restriction projection of the draft's manifest must admit every member of the delivery audience. The delivery audience is the conversation's current availability audience, not only its witnessed presence.
- On failure, the draft is regenerated once with the offending manifest entries excluded, and both attempts are recorded. If it still fails, the turn ends with a non-disclosing deferral. No content from the failed draft is delivered.
- A `wake_turn` Task's note and arguments are checked against the target conversation's audience when the Task is created. At firing, only content that clears the audience at that moment is rendered. A note that no longer clears is replaced by a non-disclosing marker, and the failure is recorded for the operator.
- Source-lane search results (`human_utterance`) are Occasion text parts under their Occasion's restriction.
- Entity handles and agent-coined relation names carry the restriction of the context that minted them, until they are used in an Occasion whose audience clears or until the operator publishes them (see H4). A "relations in use" list is computed per entity kind from audience-visible Assertions only.

### C2. The testimony principle floor (option a)

- The floor for a testimony Attestation's principle is its source Occasion's restriction. The model may narrow it freely.
- Widening needs a teller grant or an operator action. A teller grant is a span by the same teller that expresses permission to share, such as "feel free to tell people". It is checked like grounding: the span must be cited, and a soft critic plus audit sampling judge whether it expresses permission.
- Transmission is removed as a free caller decision. The caller may only narrow the floor or cite a grant.
- Deferred additive policy (option b): per-person or per-conversation widening defaults, set by the teller or the operator. Record it in the deferred list with its reopen condition: recall cost measured under option a.
- Cost, stated plainly in privacy-and-provenance.md: a fact told in a DM stays in that DM's audience unless widened.

### H5. Resolution inputs are inputs

When grounding resolves a span to a value or entity through stored records, such as "my sister" resolving to Quinn through a kinship Assertion, the records used are inputs. Their restrictions are conjoined with the testimony principle. The "resolves from the span" rule is defined as recorded resolution whose inputs are listed.

### C3. Audience-safe Assertion state

- Validity and every Assertion lifecycle transition carry evidence and fold per audience.
- Each Attestation records the validity its source asserted.
- Each Assertion transition (validity closure, supersession, retraction, invalidation) names its evidence source: an Attestation, an Occasion, or an Activity. An audience sees only the transitions whose source it can see.
- The validity and state an audience sees are folded from the Attestations and transitions visible to that audience.
- A public read therefore never renders a date or a closure that came from a confidence.

### H1. Mode and reported speech

- Mode is part of the Assertion-reuse match.
- Reported speech ("Quinn said X", or a claim a book makes) is always a `quoted` Assertion. It is readable only as reported speech, rendered with its teller and its `relayed_from` source or its Artefact source ("Rowan says Quinn said X"), and never as a flat claim.
- A Proposition-reference object is reserved for propositional attitudes about mental state ("believes", "wants"), not for reported speech.
- Make the definitions and `extraction-convergence` labelling consistent with this.

### H2. The subject guard

- Protected positions are every person-valued position, subject or object, including any Event role whose filler range admits a person. Protection does not depend on role names. Fix every role-name mismatch, such as `guard-participation-alone`.
- There are three candidate policies. `guard-denial-rate` decides between them:
  - `subject-guard/v1`, as written.
  - `subject-guard/v1-relaxed`, which exempts `public` testimony and operator-told `public` observations.
  - `subject-guard/v1-own`: v1, plus P may see an observation or derivation about P when every input is P's own testimony, or when it was produced in a context whose audience was exactly P. The operator's own `include([operator])` notes about the operator fall under the second clause.

### H3. Erasure closure follows typed dependencies

- Closure invalidates dependants reached through typed dependencies: Attestation sources, derivation inputs, cited locators, Perception inputs, Task sources, and resolution inputs (H5).
- A dependant linked to the erased record only by having been rendered in the same model call goes to operator review. It is not invalidated.
- A model-call payload that rendered erased content is still deleted, because it contains that content.
- Milestone 1 measures closure size.

### H4. Coined names

Covered by the last bullet of C1. A relation coined, or a handle minted, in a restricted context renders only to audiences that clear that context. This lasts until the name is used in a clearing Occasion or the operator publishes it.

## Simplifications

- S1. Settlement becomes a read-time projection.
  - Remove the stored `assertion_settled` and `assertion_demoted` transitions and the `support_policy` authority class.
  - An Assertion is settled for an audience when eligible support under the D3 rule is visible to that audience.
  - Remove the scenarios that test settlement races and settlement transitions, and replace them with projection scenarios.
  - The predecessor rule stays for every remaining transition.
- S2. Defer Event co-reference entirely.
  - Resolution hypotheses become identity-only.
  - Events keep stable identity. Duplicate Events are acceptable at this scale.
  - Remove Event composites, the Event component of the resolution environment, and the related scenarios. `event-weekly-meetings-not-merged` survives as a statement that no merge exists.
  - Add `event-coreference` to the deferred list with a reopen condition: measured duplicate-Event cost.
- S3. Record the frontier, not a stamped ResolutionEnvironment. The environment is recomputed from the log at the recorded frontier. The term stays as a derived read concept.

## Medium fixes

- `attributed` is a rendering obligation on the context: the content is rendered with its teller's handle. It is not a verifiable property of a reply. Compliance of replies is audited and measured, not enforced. Only testimony can be `attributed`, so fix the relaxed-guard wording above.
- Contest:
  - Mechanical contradiction detection runs at publication, and on arrival of any Assertion whose core matches a registered rule.
  - It appends a `contradiction_detected` queue mark.
  - The `contested` ordinal means that a live mechanical contradiction is visible to the audience.
  - Non-mechanical contest detection is deferred. The dormant-contested machinery applies only to mechanical contests.
- Undefined marks:
  - `validity_boundary_due` moves under deferred volatility automation.
  - `working_review_due` is deferred with working-note promotion review. Explicit promotion by the agent remains at genesis.
- Perception unusability while its reference is not authorised is a read-time projection, not a transition. `perception_invalidated` stays for real invalidation causes.
- Extraction reuse: a derived text Artefact is reusable across references to the same Artefact. Its readability always follows the consuming reference, never the reference that first produced it.
- Embeddings:
  - Vectors are recorded Activity outputs, keyed by embedder version and stored in erasable payload. They are erased with that payload.
  - An embedder change re-embeds as a recorded job.
  - Replay makes no embedder calls.
- Lifecycle nits:
  - Reconcile "erasure from any non-erased state" with the preconditions of `artefact_erased`.
  - After `entity_superseded`, the handle moves to the replacement Entity.
- Prose: use "towards" consistently.

## Milestone 1 changes (evolution.md)

- `taint-breadth` takes the controlled multi-party corpora as inputs alongside the real prompts, because the real corpus holds one private entry. It uses the C1 and C2 definitions.
- `guard-denial-rate` compares the three H2 candidates.
- Both experiments state that they import the current system's posture labels as approximate principles, and they report sensitivity to that choice.
- Add `principle-assignment`. It measures the model's narrowing decisions and grant detection against gold labels, and the recall cost of the option-a floor. It decides the grant critic's thresholds and whether option b reopens.
- Add erasure closure-size measurement (H3), as part of `taint-breadth` or as its own row.
- Milestone 1a's vocabulary includes transmission principles, source-kind conventions, and the reported-speech convention (H1).
- The preregistration states the default that applies when a gate's minimum sample cannot be reached. The default is the conservative option.
- Add Milestone 2a, a thin executable reference model: Occasion restriction, Proposition, Assertion, and Attestation, the per-audience fold, and the hard critics, with the scenario tables as tests. It unblocks `write-surface-comparison`, and it is where privacy semantics are checked from now on, in place of further prose review.
- Update confidence.md and coverage.md for C1–C3, H1–H5, and S1–S3, and add the new deferred items: option-b defaults, event co-reference, non-mechanical contest, and working-note review.
