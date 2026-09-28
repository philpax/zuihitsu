# docs-future revision 2: addendum brief

This addendum extends the first simplification brief (`simplification-brief.md` in the same directory), which still governs prose style, working rules, and the report format. An independent review of the simplified tree found the problems below. The operator approved the resolutions stated here. They are binding. Where this addendum and the first brief disagree, this addendum wins.

This pass fixes and completes. It does not re-expand removed machinery. Aim for each chapter to stay roughly its current size; a chapter may grow where it gains a definition listed here.

## Three policy decisions

### D1. Delivered-audience declassification, and testimony versus context taint

- The agent's outbound utterances are recorded as outbound Occasions (see G5). An outbound Occasion's restriction is the audience it was delivered to. That audience already passed the zero-residue check before delivery.
- When a delivered Occasion is rendered into a later context, such as conversation history, it contributes only its delivered-audience restriction. The restriction projection stops at a delivered Occasion and does not follow its ancestry further. This stops restriction from accumulating across a conversation. Without it, one confidence read in turn 3 would restrict every later write in that conversation, and the store would drift toward teller-only.
- A `testimony` Attestation takes the teller's transmission principle, not the taint of the context that wrote it. The teller spoke, and the teller's own act sets the principle.
- A testimony claim must be grounded in its span. A new hard critic, testimony grounding, checks that every typed value and entity reference in the Proposition appears in the cited span or resolves from it. Typed values are dates, quantities, and named handles. A soft critic and audit sampling check the relation choice. A claim that fails the hard check cannot be testimony. It may be proposed as a `derivation` Attestation, which inherits the context's ancestry restriction. This closes the laundering path in which a model files Quinn's confidence as `public` testimony by citing any span of Rowan's message.
- `derivation` Attestations, working notes, and other agent-derived outputs still inherit the ancestry restriction, except across a delivered Occasion.
- Non-evidentiary marks, currently `episodic_reconstruction`, keep propagating through delivered Occasions. Record this as an open question for the generated-episodes reopen condition: a strict ancestry check would block semantic writes for the rest of any conversation in which an episode was read.

### D2. Subject guard: keep v1, fix the scope, and measure

- The subject guard as written is the genesis-candidate policy `subject-guard/v1`. State plainly what it does: a person is not shown what others told the agent about them without scoped witness evidence. That includes operator-told and agent-observed facts. A group containing the person loses those facts too.
- Name the leading alternative as `subject-guard/v1-relaxed`, which exempts `public` testimony and operator-told `public` or `attributed` observations from the guard. Milestone 1's `guard-denial-rate` experiment decides between them. The chapter does not pick.
- Fix the scope rule. An ordinary search guards each candidate Assertion independently (per-Assertion scope). The candidate-set scope applies only to Event and composite reads, where roles must be evaluated together. One protected candidate never denies unrelated results.

### D3. Settlement at publication

The initial support policy settles single-lineage testimony at publication. Direct operator observations, direct agent observations, and derivations whose inputs are all settled also settle at publication. Demotion follows withdrawal of the last eligible support. Remove the candidate-only branch. `promotion_support` and `actionable_support` may merge into one mode if nothing else needs the distinction. Keep contested handling. This fixes the amnesia case, in which the agent could not recall a single-teller fact told yesterday. No claim in the corpus has two human tellers.

## Spec gaps

- G1. Entities. statements.md defines an Entity object: a minted ULID, a registered entity kind fixed at mint, and a lifecycle of `live`, `superseded` (naming a replacement entity), and `erased`. A mis-kinded entity is corrected by minting a new entity, superseding the old one, and superseding its Assertions onto the new one. `same_as` across kinds is refused. A human-readable handle, such as `person/rowan` or `organisation/northwind`, is a mutable label that resolves to the ULID. Renaming a handle appends a transition, and Proposition keys use ULIDs, never handles. Agents mint non-account entities: people without accounts, organisations, places, and topics. Connectors mint stubs, which are entities of a person kind with connector scope. Say how an agent-minted person entity and a connector stub come together, through a `same_as` hypothesis.
- G2. Writes through a composite (identity.md, with a pointer in statements.md). Each accepted identity composite designates a primary member. A write through the composite handle lands on the primary member, and its Attestation records the ResolutionEnvironment. On withdrawal, Assertions stay on the primary. Every write made through the composite while it was accepted can be enumerated from the recorded environments. Severance therefore appends a re-homing review item that lists them for the operator. Nothing moves automatically. Derivations that used cross-member evidence are invalidated as before.
- G3. The Proposition key holds definition IDs only, never definition versions. The Assertion records the definition versions it was accepted under.
- G4. Assertion reuse (statements.md). A re-mention attaches a new Attestation to an existing live Assertion when the Proposition key is equal and either the validity is identical, or the new validity is unknown or open and one live Assertion with that key has open validity or covers the Occasion's observed time. A different known validity mints a new Assertion. When several live Assertions qualify, the most recent attaches and the ambiguity is recorded. The rule is deterministic and runs in the critics.
- G5. Outbound Occasions and reply links. An Occasion has a direction, `inbound` or `outbound`. An outbound Occasion's author is the agent, its recipients carry connector witness evidence (at least `delivered`), and its restriction is its delivered audience (D1). An agent utterance is never testimony by the agent about what it relays. Occasions also carry an optional `in_reply_to` link to another Occasion, supplied by the connector. This covers issue #109. Both are raw input that cannot be recovered later, so they are in the genesis design.
- G6. `record` takes one or more existing Occasion IDs and never free text. The connector records the Occasion, and `record` only opens structuring over it. Remove the example that passes `utterance = "…"`.
- G7. Teller binding. A testimony teller must be the participant who produced the cited span on that Occasion. Relay ("Quinn told me that X") is a `quoted` Assertion, with Rowan as teller, plus `relayed_from` lineage naming the original source person. The source person gets no teller exception and no erasure authority over the relay. Remove "the teller when the speaker relays another person's words" from the caller decisions. Replace the `relayed-teller` scenario with `relay-is-quoted`. Make belief.md's relay-and-return dependence rules consistent with this.
- G8. Definition audience (relations.md). Relation definitions are public vocabulary. A coinage's description and example must not contain entity handles or Occasion content. The example uses placeholder handles from a reserved `example/` namespace, and a hard critic rejects a live handle in either field. A soft critic and operator audit sampling cover paraphrased private content. Because definitions are public by construction, rendering one contributes no restriction.
- G9. Lifecycle table (statements.md). Add a from-state column, or an equivalent compact legal-transition list, so the reference model can test legality. Make transition names consistent across chapters. The canonical clearance transition is `clearance_changed`, as identity.md defines it. Validity closure is a transition, not a state. Artefact retention counts every non-erased reference (`authorised`, `withdrawn`, and `retracted`), because withdrawal is reversible. Bytes are deleted only when every reference is erased, which folds the Artefact to `erased`.
- G10. Task actions (time.md; off-turn.md for scheduling). The genesis action vocabulary is `wake_turn(conversation, note)`: a due trigger starts an ordinary agent turn in the named conversation, with the note rendered into its context. The turn may reply to that conversation's audience under the normal checks. A due trigger is an initiating event at genesis, so reminders work. Deferred proactive initiation covers only initiation without a Task. Fix off-turn.md accordingly. Rephrase its "no live challenge-response exists" line: an off-turn message cannot use challenge-response in the moment, so its audience must resolve from clearances already recorded.
- G11. Erasure execution (privacy-and-provenance.md).
  - Erasure runs inside the server as an exclusive operation. The server quiesces turns and workers at their boundaries, executes, and resumes. The process need not stop, and both teller and operator requests use this path.
  - The erasure scope includes the requesting Occasion's governed text by default, because "forget that I'm …" repeats the content.
  - Model-call payloads whose manifests name an erased record are deleted.
  - Delivered outbound Occasions whose context rendered an erased record are listed for operator review. They are not erased automatically, because the reply may not contain the content.
  - Closure does not recurse through later history renders of a delivered Occasion unless that Occasion is itself erased.
- G12. Access accounting (query-surface.md, memory-typology.md).
  - Access recency and frequency are per audience. A read for audience A uses only accesses recorded in contexts whose audience is a superset of, or equal to, A. Anything rendered to that wider audience was already known to every member of A.
  - A read's inputs include an explicit as-of time alongside the frontier, so recency decay stays deterministic under replay.
- G13. Book grounding (artefacts-and-perceptions.md).
  - A claim from a read book is a `derivation` Attestation over the reading Activity, because interpretation is computed. It is `quoted` when it reports what the book says.
  - The grounding critic requires the cited span to be contained in a span that a context in the write's ancestry actually rendered. Overlap is not enough.
- G14. Authority classes (off-turn.md, verified-write.md).
  - Add a `structuring` class. It opens a verified-write proposal over existing Occasions for deferred structuring and the `source_only` retry. Its testimony tellers are bound by G7 and D1.
  - Add a `support_policy` class, which may append settlement and demotion under the support policy.
  - A proposal's source is one or more Occasions or Activities from one block, not "one source".
- G15. The recall clearance. Keep the field and the rule: only disclosure-cleared composites reach response-affecting context. Defer the recall-restriction propagation projection until `autonomous-identity`. At genesis, recall-cleared content is rendered only into operator-diagnostic Activities, which never feed a turn. Note it in the deferred list.
- G16. Hygiene.
  - Scenario duplicates: keep `hedge-then-flat` in belief.md and remove `hedge-then-flat-assertion` from statements.md. Keep `agent-restatement` in belief.md and remove `agent-restatement-no-support` from write-surface.md. Keep `lost-update-rejected` (statements.md), `stale-worker-result` (off-turn.md), and `publication-stale-predecessor` (verified-write.md) only if each tests something distinct, and say what in the row. Otherwise remove the redundant one. Remove or rewrite `identity-stale-decision` so it assumes no background identity authority at genesis.
  - The README's CI rule applies only to tables under a heading named "Scenarios".
  - Drop any claim that milestones compare projection digests unless the chapter defines a canonical projection encoding. A digest comparison is an implementation detail of the reference model.
  - Call the statements.md chapter "Object model" everywhere, and refer to it as "the object model". Keep the file name `statements.md`, because the research snapshots link into it.
  - lineage.md: fix "artifact" to "artefact" and retire stale vocabulary ("assumption stamp", "authority lattice", "derivation record").
  - Define or remove "crumble and accretion" and "patient-attacker".
  - Align coverage.md and confidence.md on identity leakage: the representation is closed structurally, and the behavioural fix remains inference.
  - Resolve dangling referents. "Descriptions" become a defined derived projection, public-input-only, or are cut. "Working item" becomes "working note". The `pending_ingest` and `episode_due` marks move under their deferred capabilities.

## Milestone 1 restructure (evolution.md)

- Add Milestone 1a, a provisional labelling vocabulary and entity policy. It is fixed for the milestone's experiments and is not a genesis decision. The write-surface comparison and extraction gold depend on it.
- Fix sequencing. `write-surface-comparison` runs after 1a and after Milestone 2's synthetic write surface. `relation-coinage-replay` runs after `extraction-convergence`. State that the other experiments can run in parallel with Milestone 2. Remove any unconditional "milestones 1 and 2 run in parallel" claim.
- `extraction-convergence`:
  - The primary gold is a hand-labelled restatement set sampled independently of the current consolidation machinery. `EntriesConsolidated` and `BeliefArbitrated` pairs are secondary gold, flagged as biased because the embedding-threshold machinery selected them.
  - Report convergence split into subject resolution, relation choice, and the remaining coordinates, with sample sizes and intervals.
  - Preregister a fallback for partial convergence: keep a supersession-proposal off-turn job, a narrow consolidation equivalent, if convergence falls below the pass rule.
- `write-surface-comparison` also measures the testimony-grounding critic's rejection rate and its laundering catch rate on seeded cases.
- Add `taint-breadth`. Approximate from the 417 recorded model-call prompts which restricted records each call rendered. Compute the resulting output restrictions with and without the D1 delivered-audience rule, and report the share of outputs restricted narrower than their conversation's audience. It decides whether D1 is sufficient.
- Add `guard-denial-rate`. Over real reads, report the share of results denied under `subject-guard/v1` and under `subject-guard/v1-relaxed`, by read kind and audience size. It decides the guard policy.
- `witness-assurance-audit` becomes a desk analysis of what each connector platform can demonstrate, per assurance kind and scope, with the one multi-party conversation as a sample rather than the basis.
- Add controlled multi-party corpora, scripted with the existing eval harness (`crates/eval`) using invented placeholders, for the witness, dependence, and guard experiments.
- Pin the corpus by log sequence range and a digest recorded in the preregistration.
- Settlement is decided (D3) and needs no experiment. Update Milestone 4's support-policy text.
- Update confidence.md and coverage.md for D1–D3 and the new experiments, and add the new deferred items to "Deferred capabilities".
