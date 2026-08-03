# Lane: provenance/attestation, privacy scoping, and forgetting

Research for the zuihitsu ontology redesign. Every load-bearing claim is cited. Uncertainty is flagged inline (marked *[uncertain]*). The closing section maps findings onto the fixed points and failure classes.

---

## 1. W3C PROV (PROV-DM / PROV-O): the derivation record

### What PROV is

PROV is a W3C family of specifications (Recommendations, 30 April 2013) for representing and interchanging provenance. PROV-DM has six components: (1) entities and activities and their timing; (2) agents and responsibility; (3) derivations (with subtypes Revision, Quotation, Primary Source); (4) properties linking entities that refer to the same thing (`alternateOf`, `specializationOf`); (5) collections; (6) a simple annotation mechanism ([PROV-DM, W3C TR](https://www.w3.org/TR/prov-dm/); [PROV Primer](https://www.w3.org/TR/prov-primer/)).

The three PROV core types are the load-bearing triad zuihitsu should study:

- **Entity** — a thing with some fixed aspects (a fact, a description, a conclusion).
- **Activity** — something that occurs over time and acts on entities (a maintenance pass, a turn, a model call).
- **Agent** — something bearing responsibility (the human teller, the LLM, the operator).

The relations that matter for an auditable-derivation record:

- `wasDerivedFrom` (entity → entity), specialised as `wasRevisionOf`, `wasQuotedFrom`, `hadPrimarySource` ([PROV-DM §component 3](https://www.w3.org/TR/prov-dm/)).
- `wasGeneratedBy` / `used` (entity ↔ activity), `wasAssociatedWith` (activity → agent, optionally with a `prov:Plan` via `qualifiedAssociation` — a plan is exactly the "template/recipe" slot), `wasAttributedTo` (entity → agent), `actedOnBehalfOf` (agent → agent, delegation).
- **Qualified relations**: the binary form (`wasDerivedFrom`) can be reified into an `Influence`/`Derivation` object so the edge itself carries attributes (time, role, the activity that did it). This is PROV's answer to "an edge needs attributes" — directly relevant to failure class 3.

### How PROV would model zuihitsu's derivation record

The target — "the agent derived conclusion C from entries E1, E2 under assumption A using model M with template T v3" — maps almost cleanly:

```
Entity(C)
Activity(derive_C)                              # the maintenance/turn step
Agent(LLM_instance)  ;  Agent(operator)         # responsibility chain via actedOnBehalfOf
Entity(T_v3)  a prov:Plan                        # the template as a recipe
wasGeneratedBy(C, derive_C)
used(derive_C, E1) ; used(derive_C, E2)
qualifiedAssociation(derive_C, agent=LLM_instance, plan=T_v3)
wasAttributedTo(C, LLM_instance)
wasDerivedFrom(C, E1) ; wasDerivedFrom(C, E2)
```

The two things PROV does **not** give out of the box, and both are exactly zuihitsu's failure class 11:

- **Assumption A as a first-class influence.** PROV has no "assumption" or "defeasible-support" relation. You would model A as another `Entity` that `derive_C` `used`, but PROV carries no semantics that A is *defeasible* — that retracting A should retract C. This is precisely where an **ATMS/justification-based truth maintenance** layer is wanted (already flagged for zuihitsu in #94, #5). PROV records *that* A was used; it does not record that C's truth is *contingent* on A. That gap is the difference between an audit trail and a defeasible derivation graph.
- **The model M and its non-determinism.** PROV can name M as the agent/plan, but PROV is silent on whether the activity is reproducible. For a neurosymbolic system whose fixed point is deterministic replay, the derivation record must additionally pin M's identity *and* the fact that the derivation was a record-time neural call (not replayable) — PROV gives you the slot (`prov:Plan`, `used(M)`) but not the reproducibility contract.

### Twenty years of PROV edge cases (what it got wrong or found hard)

- **Provenance of provenance / bundles.** PROV added **bundles** (component 4) late and somewhat awkwardly: a bundle is a named set of provenance assertions that is *itself* an entity, so you can assert provenance about it ([PROV-DM bundles](https://www.w3.org/TR/prov-dm/); [PROV Primer](https://www.w3.org/TR/prov-primer/)). This is the same move as named graphs (§3) and nanopub's pubinfo graph (§2) — three independent reinventions of "you need a handle on the assertion to talk about it." The awkwardness: bundles are a second-class citizen syntactically (PROV-N wraps them; RDF serialises them as named graphs), and nesting provenance-of-provenance-of-provenance has no clean recursion story. **Lesson for zuihitsu:** make the *attestation itself* a first-class addressable node from day one, not a bolt-on, so "who said that X was said by Y" is expressible without a new mechanism.
- **Granularity.** A recurring, documented core challenge: PROV extensions "are not directly interoperable because of differences in their granularities" and finding the balance between fine-grained (storage/compute cost) and coarse-grained (loses expressiveness) modelling is called out as a core difficulty ([Towards dimensions and granularity in a unified workflow provenance model, LWDA 2024](https://www.informatik.uni-wuerzburg.de/fileadmin/1003-lwda24/LWDA_Paper/KM_LWDA_CRC_161.pdf); dataset/feature/attribute-level PROV, [Cerba & Lévesque, 2017](https://www.sciencedirect.com/science/article/pii/S0198971517300558)). zuihitsu hits the same wall: is the provenance-bearing unit the entry, the attestation, the whole consolidation pass, or the individual claim inside a sentence? Failure class 1 ("facts are sentences") is a granularity mismatch — the provenance unit is coarser than the belief unit.
- **`alternateOf` / `specializationOf` are famously underspecified** — the "two entities refer to the same thing" relation (component 4) has weak semantics and is rarely used correctly. This mirrors zuihitsu's identity problem (failure class 5): PROV, like zuihitsu, found that "same-thing-ness" is genuinely hard and punted it to a thin relation.
- **PROV is descriptive, not defeasible.** PROV records what happened; it has no retraction, no belief revision, no truth maintenance. It is an *audit* format, not a *reasoning* substrate. zuihitsu needs both, and should not expect PROV alone to carry the belief-revision weight.

**Recommendation seed:** adopt the PROV *shape* (entity/activity/agent, qualified derivation with a plan slot) as the schema of the derivation record, but layer a justification/assumption relation with defeasible semantics on top (ATMS), which PROV deliberately lacks.

---

## 2. Nanopublications: the three-graph split and retraction

### The model

A nanopublication is three named graphs plus a head: **assertion** (atomic domain claim, a few RDF triples), **provenance** (provenance *of the assertion* — where the claim came from, evidence, method), and **publication info / pubinfo** (provenance *of the whole nanopublication* — who minted it, when, signature) ([Nanopublication Guidelines](https://nanopub.net/guidelines/working_draft/); [Kuhn et al., PeerJ CS 2016](https://peerj.com/articles/cs-78/)). The subject of pubinfo triples MUST be the nanopublication URI itself.

This is a sharper, more disciplined version of PROV bundles: nanopubs make the assertion/its-provenance/its-metadata split **mandatory and structural**, where PROV leaves it optional. The split is exactly the distinction zuihitsu needs and currently blurs:

- **assertion** = the claim (zuihitsu ContentEntry prose / the future structured claim).
- **provenance** = `told_by`, `told_in`, evidence, the derivation record from §1.
- **pubinfo** = the attestation metadata: which agent recorded it, when, under what authority, with what signature/visibility.

zuihitsu's attestation model **independently reinvented the assertion/pubinfo split**: an entry is "a fact a set of tellers stand behind," and each attestation carries its own posture and its own `told_by`/`told_in`. That is nanopub's pubinfo-per-assertion, arrived at from the privacy side rather than the citation side.

### Trusty URIs — immutability by content hash

Nanopubs are identified by **trusty URIs**: the URI embeds a cryptographic hash computed over the entire content, so any change is detectable and anyone can verify the object against its identifier ([Kuhn & Dumontier, trusty URIs](https://arxiv.org/abs/1401.5775) via [arxiv 1809.06532](https://arxiv.org/pdf/1809.06532)). Nanopubs were "immutable from the start," but only trusty URIs gave a technical guarantee. This is *directly* the zuihitsu event-log invariant expressed at the object level: content-addressed, tamper-evident, append-only. **The event log's seq is zuihitsu's trusty-URI analogue** — the difference is zuihitsu hashes the whole log's causal chain, nanopubs hash each atom independently. The independent-atom approach is worth noting: it lets you cite and verify one claim without the whole log, which is what inter-agent quotation (§6) needs.

### Retraction, superseding, and who-may-retract (the edge cases)

This is the richest part for zuihitsu, because nanopubs are immutable yet must support update and retraction — the same tension as an append-only log:

- **Superseding**: a new version is a *new* nanopublication that declares `npx:supersedes <old-trusty-uri>` in its pubinfo. The old one is never mutated; the graph of versions is reconstructed by following supersedes edges ([Kuhn, nanopub versioning](https://nanopub.net/guidelines/working_draft/); [Semantic micro-contributions, PeerJ CS 2021](https://pmc.ncbi.nlm.nih.gov/articles/PMC7959648/)).
- **Retraction**: to declare a nanopub obsolete *without* a replacement, publish a **separate retraction nanopublication** whose assertion graph contains `npx:retracts <trusty-uri>`. Retraction is itself a first-class, signed, addressable assertion — you can retract a retraction, and you can see who retracted and when.
- **Who may retract (authorization)**: "Updated versions and retractions should only be considered valid if authorized by the author of the original nanopublication" — validity is checked by verifying the retraction/update is **signed with the same key pair** as the original, anchored to an **introduction nanopublication** where a user cryptographically introduces their key ([Semantic micro-contributions, PMC](https://pmc.ncbi.nlm.nih.gov/articles/PMC7959648/)). This is a clean, decentralised answer to zuihitsu's "per-attester retraction with last-attestation death": retraction authority = the attester's identity, verified by signature, not by mutating shared state.

**Edge cases nanopubs hit that zuihitsu will also hit:**

1. **Retraction is advisory, not enforced.** In a decentralised network the retracted nanopub still exists and can still be fetched; consumers must *choose* to honour retractions. zuihitsu, being a single closed log, can enforce (the tombstone is authoritative) — an advantage worth keeping. But once knowledge crosses to *another* agent (§6), zuihitsu inherits the nanopub problem: you can retract on your side, but you cannot un-tell.
2. **Who-may-retract is genuinely contested when authorship ≠ subject.** Nanopubs let only the *author* retract. But the person the claim is *about* (the CI subject) may want it gone (§5, GDPR). Nanopubs have no answer; zuihitsu's subject-guard + right-to-forget must add one — retraction authority is a *lattice* (author, subject, operator), not a single key.
3. **Superseding chains fork.** Two independent updates of the same base produce a diamond; nanopubs leave conflict resolution to the consumer. zuihitsu's single-writer log avoids forks *internally* but not across agents.
4. **Signatures bind identity to keys, and keys rotate.** The introduction-nanopublication indirection exists precisely because raw keys are not stable identity. zuihitsu's ULID/handle two-tier identity is the same insight — bind attestations to the stable ULID, not to a mutable handle or transport key.

**Recommendation seed:** model attestation, supersession, and retraction as *first-class signed events that reference a content-addressed prior*, exactly as nanopubs do; make retraction authority a lattice over {author/teller, subject, operator}; keep the enforcement advantage of the closed log but design the inter-agent boundary knowing retraction becomes advisory there.

---

## 3. Named graphs and contextualised KGs: audiences as first-class contexts?

### Named graphs

A named graph is a set of triples named by a URI, which can be referred to from inside or outside the graph ([Carroll, Bizer, Hayes, Stickler, "Named Graphs, Provenance and Trust," WWW 2005](https://dl.acm.org/doi/10.1145/1060745.1060835); [Journal of Web Semantics 2005](https://www.sciencedirect.com/science/article/abs/pii/S1570826805000235)). Their motivating use cases are **provenance and trust**: publishers communicate "assertional intent" and can *sign* a graph; consumers apply "task-specific trust policies" and act only on graphs they accept. "Graphs are trusted depending on their content, information about the graph, and the task the user is performing." This is the formal ancestor of nanopub graphs and PROV bundles.

Key semantic subtlety: a named graph *quotes* triples without necessarily *asserting* them — naming a graph lets you talk about a claim without endorsing it. This "quotation vs. assertion" distinction is **exactly zuihitsu's `Attributed` posture** (visible, carries `[via X]`, *never distilled* — i.e., quoted-not-endorsed) versus `Public` (asserted, distilled into descriptions). zuihitsu reinvented the named-graph assertion/quotation split as a *visibility* distinction.

### Contextualised knowledge graphs / CKR

Contextualized Knowledge Repository (CKR) is a logical framework (Homola & Serafini; Bozzato, Eiter, Serafini) that encapsulates description-logic knowledge bases into **contexts**, with a meta-language specifying the contextual structure, and inference realised as forward SPARQL rules over per-context named graphs ([CKR, FBK/DKM](https://dkm.fbk.eu/technologies/theoretical-frameworks/ckr-contextualized-knowledge-repository/); [Bozzato et al., "Reasoning with Justifiable Exceptions in CKR"](https://link.springer.com/chapter/10.1007/978-3-319-17966-7_4)). Contexts are **multidimensional and hierarchical**, with the hierarchy inviting an OLAP-cube comparison ([Knowledge Graphs: Research Directions, Hogan et al.](https://link.springer.com/chapter/10.1007/978-3-030-60067-9_8); [Knowledge Graphs, ACM Computing Surveys 2021](https://dl.acm.org/doi/10.1145/3447772)). CKR supports **knowledge propagation** between contexts along the hierarchy, with **justifiable exceptions** — a fact holds in a general context but a more specific context may override it. Also relevant: description logics of context (Klarman & Gutiérrez-Basulto).

### Could audiences be first-class contexts rather than a posture enum?

This is the sharpest structural question in the lane. The CKR/named-graph literature says **yes, in principle, and it buys real expressiveness** — but with concrete costs.

**What first-class audience-contexts would buy:**
- A **context lattice** (who-can-see-what as a partial order) replaces the flat posture enum. `Public ⊒ Attributed ⊒ PrivateToTeller ⊒ Exclude(set)` is *already* a lattice zuihitsu evaluates ad hoc in `visible(entry, present_set)`; CKR would make it the primary structure and give `visible` a *query semantics* (evaluate the fact in the context of the present audience) instead of a hand-written predicate.
- **Justifiable exceptions** map onto zuihitsu's subject-guard: a fact is public in general but *excepted* in the context where the subject is present. CKR has formal machinery for exactly this override.
- **Per-context truth** cleanly models "the agent believes X in front of Alice but must behave as if it does not know X in front of Bob" — the hidden-endorsement / zero-residue fixed point *is* a per-context-truth requirement. In CKR terms, the uncleared context genuinely does not entail the hidden fact.

**What breaks / the costs:**
- **Combinatorics and the closed-world audience.** CKR contexts are relatively static and few; zuihitsu audiences are dynamic sets of present participants drawn from an open population. A context per audience-subset is a powerset — untenable. You would need contexts parameterised by *predicates over the present set* (as `Exclude(set)` already is), not enumerated contexts. *[uncertain: I found no CKR work handling a combinatorial/dynamic context population well — this appears to be a genuine gap, not just my ignorance.]*
- **Query semantics across contexts can leak.** CKR *propagates* knowledge up/down the hierarchy by design. For zuihitsu that is a **hazard**: propagation is the opposite of compartmentalisation. You would have to invert the default — no propagation unless explicitly cleared — which is fighting the framework's grain.
- **Reasoning cost.** Cross-context DL reasoning is expensive; zuihitsu's `visible` is a cheap deterministic predicate evaluated at every surface. Making it a context-query risks the hot path.

**Assessment:** treat audiences as **first-class contexts in the data model** (each attestation is *scoped to* a context expressed as a predicate over present participants), but **keep the evaluation as a deterministic predicate**, not a general DL cross-context inference. Borrow CKR's *conceptual* framing (per-context truth, justifiable exceptions for the subject-guard) and named-graphs' *quotation-vs-assertion* distinction (which zuihitsu already has as Attributed-vs-Public), but do **not** import CKR's knowledge-propagation engine — propagation is antithetical to compartmentalisation. The posture enum becomes a small, closed vocabulary of *transmission-principle* templates over a context predicate, which is the natural bridge to §4.

---

## 4. Contextual integrity: transmission principles as first-class data

### The formal model

Contextual integrity (CI) defines privacy as **appropriate information flow** governed by norms over a **five-tuple**: (data **subject**, **sender**, **recipient**, information **type**, **transmission principle**) ([Nissenbaum, "Privacy as Contextual Integrity"](https://www.researchgate.net/publication/228198982_Privacy_As_Contextual_Integrity); [Barth, Datta, Mitchell, Nissenbaum, "Privacy and Contextual Integrity: Framework and Applications," IEEE S&P 2006](https://nyuscholars.nyu.edu/en/publications/privacy-and-contextual-integrity-framework-and-applications)). The transmission principle is the constraint on *how* information may flow (in confidence, reciprocally, with consent, for a stated purpose).

Barth et al. formalise CI in **linear temporal logic (LTL)** over traces of communicating agents. Agents send messages containing attributes about subjects; each agent has a *knowledge state*. Norms are of two kinds ([PrivaCI-Bench, arxiv 2502.17041](https://arxiv.org/pdf/2502.17041), summarising Barth et al.):

- **Positive norms**: flows that are *permitted* in a context (a permitting condition must hold for the flow to be allowed).
- **Negative norms**: flows that are *prohibited*.

The **transmission principle is expressed as a temporal condition**: LTL past/future operators let a norm say "attribute a may be sent to r *only if* in the past the subject consented" (past operator) or "*only if* in the future the recipient will not forward it" (future/obligation). This is the crucial move for zuihitsu: **the transmission principle is a temporal formula, not a static tag** — consent, reciprocity, and "in confidence" are conditions over the *history and future* of the trace, which an append-only event log is uniquely well-suited to evaluate.

### Would a fuller CI formalisation strengthen or bloat zuihitsu?

zuihitsu already cites CI informally and its postures are *implicit* transmission principles:
- `PrivateToTeller` ≈ "in confidence, to this sender's context only" + a *negative* norm against the subject as recipient (the subject-guard).
- `Attributed` ≈ "may flow to all recipients but must carry the sender attribution" (a transmission principle that mandates provenance-on-transmission).
- `Public` ≈ "may flow to all, may be aggregated/distilled."
- `Exclude(set)` ≈ a negative norm parameterised by recipient set.

**Where a fuller CI formalisation clearly strengthens the model:**
1. **Reciprocity and consent as first-class conditions.** Today zuihitsu cannot express "Alice told me this in confidence but said I may share it with Bob" or "I may repeat this once Alice has repeated it herself." As LTL-style transmission principles over the log, these become expressible: the condition references prior events. This directly extends the posture enum from four hard-coded cases to a small algebra of principles.
2. **Purpose limitation / obligations.** A future-tense transmission principle ("may be used to remind the subject, but not to inform third parties") gives the calendar/reminder surface a principled privacy story it currently lacks — relevant to failure class 4 (schedule/description conflation leaks intent).
3. **The subject-guard becomes a derived negative norm**, not a special case — cleaner and generalisable to "never to recipients in set S," of which "never to the subject" is one instance.

**Where it bloats:**
- Full LTL norm-checking is a model-checking problem; zuihitsu's `visible` must stay a cheap deterministic predicate. Do **not** ship a general temporal-logic evaluator on the hot path.
- The five-tuple's *sender* and *subject* are often the same or entangled in a personal-agent setting; the full generality (arbitrary sender/recipient/subject triples across institutional contexts) is more than a single personal agent needs.

**The fixed point (zero residue, hidden endorsement).** CI gives the *cleanest formal statement* of zuihitsu's hardest fixed point: on an uncleared surface, the trace must be **observationally equivalent** to one in which the hidden fact was never known. In CI terms this is a *negative norm* that must hold not just for direct disclosure but for any *derived* artefact (a description, a distilled summary, an endorsement, a scheduling side effect). Barth et al.'s knowledge-state model makes this precise: the recipient's inferable knowledge, not just the literal message, is what the norm constrains. **This is the argument for why `Public`-only distillation (the write-time compartmentalisation guarantee) is correct and must be preserved: distillation is a derived flow, and the negative norm binds derived flows.** *[This is my synthesis of Barth's knowledge-state semantics applied to zuihitsu; the observational-equivalence framing is standard in non-interference security, which CI's knowledge model parallels — see below.]*

**Connection to non-interference.** The zero-residue requirement is formally **non-interference** (Goguen–Meseguer): low-clearance observations are independent of high-clearance inputs. CI's negative norms + knowledge states are the information-flow-security statement of the same property. zuihitsu's "fail-closed, zero residue on uncleared surfaces" is a non-interference property, and the literature on *declassification* (controlled downgrade) is the right home for `Public` distillation and for consent-based sharing — declassification is exactly "a transmission principle that permits an otherwise-forbidden flow when a condition holds." *[uncertain on exact citation to attach, but the non-interference/declassification framing is well-established security theory and worth the redesign consulting directly.]*

**Recommendation seed:** promote transmission principles to **first-class data** — a small, closed, agent-coined-but-registered vocabulary of principles (`in_confidence`, `attributed`, `reciprocal`, `with_consent(event)`, `purpose(reminder)`) each compiling to a deterministic predicate over (present set, log history). The posture enum becomes the *default library* of these principles. Keep evaluation deterministic and fail-closed; treat the zero-residue property as non-interference and hold distillation to the `Public`-only rule because it is a derived flow.

---

## 5. Forgetting vs append-only: reconciling erasure with deterministic replay

This is where the append-only fixed point collides hardest with the personal-agent ethics fixed point ("a person asks the agent to forget; a teller revokes").

### The industry patterns (documented)

The tension is standard and has canonical patterns ([EventStore GDPR compliance](https://docs.eventsourcingdb.io/best-practices/gdpr-compliance/); [Verraes, "Eventsourcing Patterns: Crypto-Shredding"](https://verraes.net/2019/05/eventsourcing-patterns-throw-away-the-key/); [Conduktor, GDPR & Kafka](https://www.conduktor.io/blog/gdpr-kafka-right-to-erasure); [oneuptime crypto-shredding guide](https://oneuptime.com/blog/post/2026-02-17-how-to-set-up-crypto-shredding-for-gdpr-right-to-erasure-compliance-in-google-cloud/view)):

1. **Crypto-shredding (throw away the key).** Encrypt each subject's sensitive payload with a **per-subject key**; to erase, destroy the key. The event stays in the log (row counts, causal links, timestamps intact) but the payload is irrecoverable ciphertext ([Verraes](https://verraes.net/2019/05/eventsourcing-patterns-throw-away-the-key/); [Granit](https://granit-fx.dev/blog/crypto-shredding-gdpr-erasure-without-deleting-rows/)). **Per-entity key isolation is essential**: one key per subject so erasing one does not break others.
2. **Forgettable payloads.** Keep only a *reference id* in the event log; store the actual personal data in an external mutable store keyed by that id. Erasure deletes the external record; the log keeps the reference ([EventStore](https://docs.eventsourcingdb.io/best-practices/gdpr-compliance/); Verraes recommends this over crypto-shredding for regulated data).

### The replay problem, stated precisely

Verraes' own analysis concedes the article "doesn't explicitly address how replay reconciles with erasure" — and the tension is fundamental: **once the key is destroyed, replay can still read the event but cannot decrypt it.** Historical encrypted payloads become inaccessible; the ciphertext remains ([Verraes](https://verraes.net/2019/05/eventsourcing-patterns-throw-away-the-key/)).

For zuihitsu's *deterministic replay* fixed point this is the crux: **replay must be deterministic given whatever survives erasure.** The resolution the patterns imply, adapted to zuihitsu:

- Deterministic replay is defined over the log **as it currently stands**, not over the log-that-once-was. Erasure is a *forward* operation (destroy a key / delete a forgettable payload) that changes the current log; replay of the post-erasure log is still deterministic — it just deterministically produces a materialisation in which the forgotten fact is *absent*.
- The critical design constraint: **the forgotten payload must not be load-bearing for the replay of anything that must survive.** If conclusion C was derived from erased entry E, replay after erasure cannot reconstruct C's derivation. This is where §1's defeasible-derivation graph pays off: erasing E should *propagate* — C's justification loses a support, and either C is re-derivable from other support or C too must fall. An ATMS makes this tractable: erasure retracts an assumption/premise and the truth-maintenance layer recomputes what still holds. Without it, erasure leaves dangling derivations that reference a hole. **This is the strongest single argument in the lane for a justification-based derivation layer.**

### Tamper-evidence vs erasure

You can keep the log **tamper-evident** (each entry hash-chained, nanopub-style §2) *and* support erasure, if erasure is modelled as an **explicit, signed, appended event** ("forget E, authorised by subject/operator, at time T") whose effect is to shred E's payload key while leaving E's *envelope* (hash, timestamp, causal position) intact. The chain still verifies (the envelope is unchanged; only the sealed payload is gone), and the *fact that a forgetting happened* is itself auditable — you can prove something was forgotten without revealing what. This is the reconciliation: **redaction is a first-class, tamper-evident, replayable event, not an out-of-band mutation.** ([Redactable/tamper-evident log framing generalised from crypto-shredding + Kafka tombstone patterns; the "prove a redaction occurred without revealing content" property is the design goal — *[uncertain: I did not find one canonical citation for redactable-blockchain-style logs in the search; the mechanism is a synthesis of crypto-shredding + append-only tombstones, both cited above].]*)

### What a *personal* agent is ethically/legally obligated to forget

- **The subject's right (GDPR Art. 17 analogue).** A person the data is *about* can demand erasure. Note GDPR treats **encrypted personal data as still personal data** ([Conduktor](https://www.conduktor.io/blog/gdpr-kafka-right-to-erasure); Verraes' legal note), so crypto-shredding's legal sufficiency is *contested* — a breach of not-yet-destroyed keys still exposes data. For a personal agent, the ethically safer default is forgettable-payloads (true deletion of content) with crypto-shredding as the fallback where causal-integrity must be preserved.
- **The teller's revocation.** Distinct from the subject: the *teller* said something "in confidence" and now revokes. This maps to nanopub's who-may-retract (§2) — teller-authored retraction. zuihitsu already has per-attester retraction; the new requirement is that retraction can escalate to *erasure* (not just tombstone) when the teller or subject demands the content gone, not merely hidden.
- **Retraction ≠ erasure.** zuihitsu today has *tombstones* (retract/supersede) which *hide* but keep the content for replay. True forgetting needs a *second, stronger* operation that destroys content. The redesign should distinguish **retract** (append a tombstone; content survives for audit; reversible) from **forget** (shred the payload; content gone; irreversible; itself audited as having-happened). Both are appended events; only the second breaks content-replayability, and only for the shredded atom.

**Recommendation seed:** (a) model forgetting as an explicit signed appended event (`Forget`), distinct from `Retract`; (b) shred content via per-subject/per-atom keys or forgettable payloads, keeping the hash-chained envelope so the log stays tamper-evident and replay stays deterministic over the post-forget log; (c) require the derivation layer (§1) to be defeasible so shredding a premise propagates to conclusions instead of leaving dangling references; (d) forgetting authority is the same lattice as retraction {teller, subject, operator}, with subject/operator able to force erasure the teller alone could only tombstone.

---

## 6. Inter-agent knowledge: provenance and trust when the teller is an agent

### The problem shape

When the teller is itself an agent with its own store, a claim arriving at zuihitsu carries (or should carry) *nested* provenance: "Agent B told me X, and B says B derived X from B's teller C." This is **testimony chains** — the epistemology-of-testimony problem applied to multi-agent systems.

### What the literature offers

- **Quotation vs. assertion (named graphs, §3; PROV Quotation, §1).** The formal primitive already exists: quoting a claim without asserting it. An inter-agent claim should enter zuihitsu's store as a **quotation** (`wasQuotedFrom` the other agent), *not* an assertion — it is `Attributed` by construction, never distilled into zuihitsu's own descriptions until independently corroborated. zuihitsu's `Attributed` posture and `told_by=Agent` variant already encode this; the refinement is that the *chain* (B-said-C-said) must be representable, which needs the first-class-attestation-node move (§1 bundles / §2 pubinfo).
- **Trust propagation and evidence-based trust.** Recent MAS work frames inter-agent trust as **evidence-based** and **traced**: "important cases for provenance tracing include claims derived from multiple sources, tool outputs contradicting memory items, and **inter-agent messages that propagate unsupported assumptions**" ([From Agent Traces to Trust: A Survey of Evidence Tracing and Execution Provenance in LLM Agents, arxiv 2606.04990](https://arxiv.org/pdf/2606.04990)); trust as evidence-based level evaluation ([Actions Speak Louder Than Words, Springer 2026](https://link.springer.com/chapter/10.1007/978-981-95-3543-9_14)). The "propagate unsupported assumptions" failure is *exactly* zuihitsu failure class 11 crossing the agent boundary — and it argues for **stamping assumptions on inter-agent-derived claims** (the ATMS assumption-stamp from §1/#94) so a downstream agent can see what a claim rests on.
- **Architecting trust in epistemic agents.** Framing epistemic AI agents as creating "new informational interdependencies" requiring provenance systems and "knowledge sanctuaries," with trustworthiness (sincerity, honesty, conscientiousness — the testimony virtues) as the governance target ([Architecting Trust in Artificial Epistemic Agents, arxiv 2603.02960](https://arxiv.org/abs/2603.02960)).
- **FIPA/KQML `tell` semantics.** The classic agent-communication-language `tell` performative carries the sender's *sincerity* assumption (the sender believes what it tells) but the receiver is not obligated to adopt it as belief — the receiver may hold it as "B believes X," which is precisely quotation-not-assertion. FIPA's semantic language separates the *illocutionary force* (tell) from the receiver's *belief adoption* ([FIPA trust/security](https://www.researchgate.net/publication/229015248_Towards_improved_trust_and_security_in_FIPA_agent_platforms)). *[uncertain on precise FIPA-SL formal detail from the search; the belief/assertion separation is standard ACL semantics.]*

### Edge cases specific to inter-agent zuihitsu

1. **Retraction across the boundary becomes advisory** (as §2 warned): if B retracts X, zuihitsu must *honour* the retraction, which requires a live provenance link back to B's attestation identity (a trusty-URI-style content reference to B's claim). Without a stable reference, you cannot propagate B's retraction.
2. **Transitive confidence / CI across agents.** If B tells zuihitsu something "in confidence," the transmission principle (§4) must travel *with* the claim across the boundary — a first-class transmission principle is what makes cross-agent confidence expressible at all. A posture enum local to one instance cannot cross; a transmission-principle-as-data can.
3. **The recitation attack (#94) is a testimony problem.** Knowledge overlap is unsound as an identity signal *because testimony can be recited* — B can truthfully report C's knowledge without being C. The testimony literature's distinction between *possessing* knowledge and *transmitting* it is the formal grounding for #94's conclusion.

**Recommendation seed:** inter-agent claims enter as **quotations with a preserved provenance chain and travelling transmission principle**, never as native assertions; keep a content-addressed reference to the source agent's attestation so retractions propagate; stamp inter-agent-derived conclusions with their assumptions (ATMS) so downstream unsupported-assumption propagation is visible and defeasible.

---

## Implications for zuihitsu

Mapped to the fixed points and failure classes. Concrete, redesign-facing.

### The derivation/provenance record (failure class 11; #94, #100)

Adopt the **PROV shape** for the auditable-derivation record: an activity `wasGeneratedBy` a conclusion, `used` the premise entries, `wasAssociatedWith` the agent via a `qualifiedAssociation` whose `plan` is the template `T v3` and whose agent names the model `M`. This directly upgrades `produced_by` from "records model+template" to "records model+template+premises+agent+time." **But PROV is not enough**: it is descriptive, not defeasible. Layer a **justification/ATMS relation** on top so that:
- Assumption A is a first-class *defeasible support*, not just another `used` input (failure class 11's "never records criteria/evidence" and #94's "assumption-stamped derivations").
- The neural, record-time-only nature of M's call is pinned (deterministic-replay fixed point): the derivation record must mark the step as non-replayable and freeze its output, consistent with #100's in-block-LLM tension.
- **Make the attestation a first-class addressable node** (PROV-bundle / nanopub-pubinfo / named-graph lesson): "who said that X was attributed to Y" must be expressible without a new mechanism, and it is the anchor for cross-agent references and retraction propagation.

### Audiences as first-class contexts? (fixed point: privacy ≥ current; failure classes 3, 8)

**Yes in the data model, no in the evaluator.** Make each attestation *scoped to a context expressed as a predicate over present participants* (generalising `Exclude(set)`), and treat the posture enum as the default *library* of such scopes — this is the named-graphs quotation/assertion split (Attributed = quoted, Public = asserted) plus CKR's per-context truth and justifiable exceptions (the subject-guard is a justifiable exception). **Do not import CKR's knowledge-propagation engine** — propagation is the opposite of compartmentalisation — and **keep `visible()` a cheap deterministic fail-closed predicate**, not a DL cross-context query. The powerset-of-audiences combinatorial blowup is real; predicates-over-present-set is the only tractable form.

### Transmission principles as first-class data (fixed points: privacy, hidden-endorsement, zero-residue; failure class 4)

Promote transmission principles from a four-value enum to a **small, registered, closed vocabulary of principles**, each compiling to a deterministic predicate over (present set, log history): `in_confidence`, `attributed`, `reciprocal`, `with_consent(event)`, `purpose(...)`. This:
- Lets confidences travel across the inter-agent boundary (§6) where a local enum cannot.
- Makes the **subject-guard a derived negative norm** (generalises to "never to recipients in S").
- Gives calendar/reminder flows a principled privacy story (`purpose(reminder)`), touching failure class 4.
- Treat the **zero-residue fixed point as non-interference**: the uncleared surface must be observationally independent of hidden facts, and because *distillation is a derived flow*, hold the `Public`-only distillation rule as a non-interference invariant, not a convention. `Public` distillation is *declassification* — a transmission principle that permits an otherwise-forbidden aggregation.

### The forgetting mechanism, replay-compatible (fixed point: append-only + deterministic replay; ethics)

Distinguish two appended, signed operations:
- **`Retract`** (existing tombstone): hides, content survives for audit, reversible.
- **`Forget`** (new): shreds the payload (per-subject/per-atom crypto-shredding, or forgettable-payloads for true deletion where the law demands it — GDPR treats ciphertext as still personal data), keeps the **hash-chained envelope** so the log stays tamper-evident and *the fact of forgetting is itself auditable without revealing content*. Replay is redefined as deterministic over the **post-forget** log.
- **Erasure must propagate through the derivation graph** (§1 ATMS): shredding a premise retracts its support and the truth-maintenance layer recomputes what still holds, rather than leaving conclusions dangling on a hole. Forgetting without a defeasible derivation layer produces incoherent replay — this is the second strong argument (with §6) for building the ATMS layer.
- **Forgetting/retraction authority is a lattice** {teller, subject, operator}: the teller can tombstone; the subject and operator can force erasure (the nanopub who-may-retract question, answered for the case where subject ≠ author, which nanopubs leave open).

### Inter-agent provenance (failure class 11 across the boundary; #94)

Inter-agent claims enter as **quotations, not assertions** (`told_by=Agent` ⇒ `Attributed` by construction, never distilled until independently corroborated), carrying:
- a **content-addressed reference** to the source agent's attestation (trusty-URI analogue) so the source's later *retraction propagates*;
- the **travelling transmission principle** (so cross-agent confidence is honoured);
- **assumption stamps** (ATMS) so a downstream agent can see and defease what the claim rests on — directly countering the surveyed "inter-agent messages that propagate unsupported assumptions" failure.
The **recitation attack (#94) is formally a testimony problem**: knowledge can be transmitted without being possessed, so knowledge-overlap is unsound for identity — the epistemology-of-testimony distinction between possessing and transmitting knowledge is the grounding for #94's move to relational-structure identity signals plus challenge-response.

### One cross-cutting recommendation

Four independent literatures — PROV bundles, nanopub pubinfo, named graphs, CKR contexts — **all reinvented "make the assertion a first-class, addressable, context-scoped object."** zuihitsu's attestation model already got halfway there from the privacy side. The redesign's single highest-leverage move is to **unify attestation + provenance + visibility into one first-class object**: a signed, content-referenced, context-scoped attestation that carries its derivation (PROV+ATMS), its transmission principle (CI-as-data), and its retract/forget lifecycle (nanopub-style). Getting that one object right dissolves failure classes 3, 8, and 11 simultaneously and gives the inter-agent boundary and the forgetting mechanism a shared substrate.

---

### Source list (load-bearing)

- PROV-DM: https://www.w3.org/TR/prov-dm/ ; PROV Primer: https://www.w3.org/TR/prov-primer/
- PROV granularity: https://www.informatik.uni-wuerzburg.de/fileadmin/1003-lwda24/LWDA_Paper/KM_LWDA_CRC_161.pdf ; https://www.sciencedirect.com/science/article/pii/S0198971517300558
- Nanopublication Guidelines: https://nanopub.net/guidelines/working_draft/ ; Kuhn et al. PeerJ CS 2016: https://peerj.com/articles/cs-78/ ; Semantic micro-contributions (retraction/supersede/signing): https://pmc.ncbi.nlm.nih.gov/articles/PMC7959648/ ; Nanopublications resource survey: https://arxiv.org/pdf/1809.06532
- Named graphs: Carroll/Bizer/Hayes/Stickler WWW 2005 https://dl.acm.org/doi/10.1145/1060745.1060835 ; JWS 2005 https://www.sciencedirect.com/science/article/abs/pii/S1570826805000235
- CKR / contextualised KGs: https://dkm.fbk.eu/technologies/theoretical-frameworks/ckr-contextualized-knowledge-repository/ ; Bozzato et al. https://link.springer.com/chapter/10.1007/978-3-319-17966-7_4 ; Hogan et al. Knowledge Graphs survey https://dl.acm.org/doi/10.1145/3447772 ; Research Directions https://link.springer.com/chapter/10.1007/978-3-030-60067-9_8
- Contextual integrity: Nissenbaum https://www.researchgate.net/publication/228198982_Privacy_As_Contextual_Integrity ; Barth/Datta/Mitchell/Nissenbaum 2006 https://nyuscholars.nyu.edu/en/publications/privacy-and-contextual-integrity-framework-and-applications ; PrivaCI-Bench (formalisation summary + LLM application) https://arxiv.org/pdf/2502.17041
- Forgetting/erasure: EventStore GDPR https://docs.eventsourcingdb.io/best-practices/gdpr-compliance/ ; Verraes crypto-shredding https://verraes.net/2019/05/eventsourcing-patterns-throw-away-the-key/ ; Conduktor GDPR & Kafka https://www.conduktor.io/blog/gdpr-kafka-right-to-erasure ; Granit https://granit-fx.dev/blog/crypto-shredding-gdpr-erasure-without-deleting-rows/
- Inter-agent trust/provenance: From Agent Traces to Trust https://arxiv.org/pdf/2606.04990 ; Architecting Trust in Artificial Epistemic Agents https://arxiv.org/abs/2603.02960 ; Evidence-based trust MAS https://link.springer.com/chapter/10.1007/978-981-95-3543-9_14 ; FIPA trust/security https://www.researchgate.net/publication/229015248_Towards_improved_trust_and_security_in_FIPA_agent_platforms
