# Identity

Identity resolution links permanent platform stubs without rewriting them. The canonical identities and lifecycles of [Occasion, Activity, Assertion, Attestation, and Derivation](statements.md) remain owned by the assertion model. This chapter defines identity hypotheses and the resolved views built from them.

Hard equivalence is too strong. Linked-data deployments show that transitive closure amplifies one incorrect link across an entire class, including a measured case that collapsed names for more than 177,000 distinct entities. Record-linkage research also shows why shared attributes are unsafe evidence: copied facts violate the independence assumption. Relational evidence is harder, but not impossible, to forge ([research lane](research/2026-07-24/lanes/identity-belief.md); [confidence register](confidence.md#identity)).

## Permanent stubs

A connector mints a platform stub from the most stable identifier the platform supplies. The stub has a permanent ID, connector and platform scope, creation evidence, and lifecycle. It is never consumed, renamed, or repurposed by a merge. Connector identifier rotation appends an alias or replacement record while retaining the original stub.

A stub is not proof of a person. It is the stable source identity against which Occasions, witness evidence, and authorisation decisions were recorded. Reads may present a resolved person view, but stored provenance continues to name the original stub.

## Resolution hypotheses

A proposed co-reference is a versioned identity-resolution hypothesis with its own stable ID. It records:

- member stubs or prior composite identities;
- evidence and dependence lineage;
- scope: the contexts and entity kinds for which the hypothesis applies;
- proposing authority;
- policy, ontology, implementation, and source-head versions used to propose it.

The immutable proposal has no status or clearance field. Its versioned fold defaults to `candidate`. Appended `accepted`, `rejected`, `withdrawn`, and `superseded` transitions name the deciding authority, evidence, source head, recall clearance, disclosure clearance, and any composite identity. A later clearance change appends a replacement acceptance or withdrawal and cannot mutate the earlier decision. Acceptance does not convert the hypothesis into universal equality. It authorises a specific composite view under a named resolution environment.

Recall and disclosure clearances are ordered capabilities, not one confidence score. Recall clearance permits source discovery for merge investigation. Disclosure clearance permits cross-stub information to enter response-affecting context for a resolved audience. Challenge-response or operator confirmation may establish disclosure clearance; a passive score alone does not.

The literature supports graded, contextual, revisable links and non-destructive merge overlays. The exact fields, clearance split, and lifecycle here are permanence-driven design choices ([research lane](research/2026-07-24/lanes/identity-belief.md)).

## Overlap and non-transitivity

Hypotheses are not transitively closed. Candidate hypotheses may overlap: one can propose `{a,b}` while another proposes `{b,c}`. They remain separate candidates with separate evidence. Acceptance follows these rules:

1. Operational composites in one resolution environment must be disjoint. A stub cannot belong to two accepted composites in the same scope.
2. Accepting a candidate that overlaps an accepted composite requires explicitly withdrawing, superseding, or replacing the conflicting resolution. The system does not infer `{a,b,c}`.
3. Conflicting candidates remain inspectable and may collect evidence, but they do not affect ordinary resolved reads.
4. A candidate spanning scopes does not authorize propagation outside the intersection of its declared scope and clearance.
5. Every resolved read records or returns the unified ResolutionEnvironment used, including its identity component: accepted hypothesis/composite IDs and versions, conflict policy, clearance level, audience assurance, and source head.

When several non-conflicting accepted hypotheses need to be presented as one operational person, the folded composite receives a stable resolution identity. It does not consume any source stub. A later environment may produce another composite while historical reads remain interpretable under the environment they recorded.

## Recall and response-affecting context

Tentative unified recall is not harmless. Once information from another stub enters a model prompt, retrieval rank, summary, tool argument, or action choice, it can affect an irreversible disclosure even if the final text omits the source. Therefore:

- candidate or recall-only hypotheses may support an internal merge-investigation Activity;
- their cross-stub content may not enter conversational prompts, actionable support, generated summaries, tool calls, or initiated actions;
- response-affecting cross-stub context requires disclosure-grade clearance for the current audience and must also pass [transmission and subject-guard evaluation](privacy-and-provenance.md);
- operator-only diagnostics may inspect weaker hypotheses under an explicit restricted audience and retain their influence envelope.

The check is audience-wide. Disclosure clearance to one account does not clear a group containing other people. Unknown audience identity denies. This is the same fail-closed set rule used by privacy evaluation.

## Resolution environments and derivation

Every read or Derivation that resolves identity or Event co-reference names one immutable ResolutionEnvironment. It contains:

- optional identity and Event components, each with accepted hypothesis and composite IDs and definition versions;
- the applicable overlap/conflict and clearance policy versions;
- audience identity inputs and assurance where disclosure is relevant;
- ontology and policy versions plus source head;
- any unresolved candidates deliberately admitted for a restricted investigation.

The identity and Event hypothesis types and policies remain separate. The single envelope prevents a Derivation from recording one while silently omitting the other.

This replaces a single merge stamp. A Derivation can depend on several identity and Event-resolution decisions, schema aliases, and policy versions. Its environment makes those dependencies explicit without rewriting its inputs. The canonical [Derivation record](statements.md#derivation) owns the rest of the lineage.

Withdrawing or severing an accepted hypothesis appends a transition. On the next fold, composite views produced by environments containing it are no longer current, and dependent Derivations are invalidated. Source stubs, Occasions, Attestations, and Assertions remain unchanged. Model-driven re-derivation is a later recorded Activity, never part of replay.

Truth-maintenance and provenance-semiring research support dependency-indexed invalidation, while limiting dependencies to revocable assumptions avoids general ATMS label growth. The complete resolution environment is a broader design requirement than the earlier single-merge stamp ([research lane](research/2026-07-24/lanes/identity-belief.md)).

## Evidence and authority

Attribute overlap, biography similarity, and knowledge recitation do not independently support a merge. Evidence retains its kind and dependence lineage. Useful evidence includes connector continuity, authenticated challenge-response, operator confirmation, independent relational structure, and explicit self-identification. Relational structure raises the cost of impersonation but does not close the patient-attacker case.

The conversational agent may propose a candidate hypothesis by recording an observation Activity and its evidence. It cannot accept a hypothesis, inspect unrestricted sibling history, choose around the resolved handle it was given, or grant disclosure clearance. An operator may accept, reject, withdraw, or confirm a hypothesis within operator authority. Connector-authenticated challenge-response may grant only the clearance defined by its registered policy.

Autonomous scoring is an open experiment until there is evidence for calibration, overlap handling, adversarial resistance, and one-handle behaviour. Initial policy uses operator-confirmed, disjoint composites.

## The operational wall

Identity resolution runs before conversational context is assembled. The agent receives one resolved handle per person under the current environment and writes to that handle. It does not see sibling stubs, candidate scores, designated primaries, or conflict metadata.

The wall keeps architectural metadata out of ordinary reasoning, but it is not itself a security boundary. The audience resolver and influence rules still enforce disclosure. The current system's sibling-history relay failures motivate the one-handle surface; that it fully fixes the behaviour leak remains a design inference rather than an established result ([confidence register](confidence.md#identity)).

Operator diagnostics expose hypotheses, evidence, conflicts, and environments without exposing their hidden cardinality or content to the conversational agent. Every diagnostic access is audience-scoped and recorded.

## Worked severance

Suppose `stub/chat-a` and `stub/forum-b` are accepted as composite `identity/r7` under environment `env/12`. An Assertion derived from both histories records `env/12`; the original Attestations still name their source stubs.

Later evidence shows the stubs are different people. The operator appends withdrawal of the hypothesis. Folding under a new environment restores separate views for both stable stubs and makes `identity/r7` historical. The derived Assertion is invalidated because its Derivation names `env/12`; it is not edited or reassigned. A later Activity may produce separate replacement Assertions from the surviving source evidence. Any disclosure already made cannot be undone, which is why response-affecting context required disclosure clearance before the merge was used.
