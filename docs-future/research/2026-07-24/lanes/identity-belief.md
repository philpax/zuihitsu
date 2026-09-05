# Lane: identity, belief revision, and truth maintenance

Research lane for the zuihitsu ontology redesign. Scope: what the KR, entity-resolution, truth-maintenance, and belief-revision literatures say about identity-as-hard-equivalence, revisable merges, assumption-stamped derivations, and credence over attestations — mapped onto failure classes 5, 7, 8 and issues #94, #104, #15.

Citations are inline. Confidence is flagged where a claim is inferred rather than directly sourced.

---

## 1. The `owl:sameAs` pathology — identity as hard equivalence goes wrong

### What breaks

`owl:sameAs` asserts that two IRIs denote the same real-world entity, which under OWL semantics licenses *unrestricted substitution*: every property of one flows to the other, in both directions, transitively and forever. Halpin, Hayes, McCusker, McGuinness, and Thompson ("When owl:sameAs Isn't the Same: An Analysis of Identity in Linked Data", ISWC 2010) argue this is far too strong for the way people actually use the link — it "encodes only one point on a scale of similarity" and is routinely asserted when the author means something weaker ("same-ish", "same in this context") ([Springer](https://link.springer.com/chapter/10.1007/978-3-642-17746-0_20), [PDF](https://www.ibiblio.org/hhalpin/homepage/publications/ldow2010.pdf)). The follow-up, "…Redux: Towards a Theory of Identity, Context, and Inference on the Semantic Web" (2015), proposes RDFC — RDF-with-Contexts — so identity statements can be scoped to a context rather than asserted globally ([Edinburgh Research Explorer](https://www.research.ed.ac.uk/en/publications/when-owlsameas-isnt-the-same-redux-towards-a-theory-of-identity-c/)).

The failure has two roots, per the survey "The sameAs Problem: A Survey on Identity Management in the Web of Data" (Raad, Beek, van Harmelen, Pernelle, Saïs; arXiv 1907.10528) ([ar5iv](https://ar5iv.labs.arxiv.org/html/1907.10528)):

- **Philosophical.** Identity is *context-dependent* and *time-dependent*. The survey's canonical example: two medicines with the same chemical structure are "the same in a medical context but different in other contexts"; and identity-over-time is the Ship of Theseus. A single global equivalence cannot represent either.
- **Practical.** Even expert humans disagree wildly on what counts as the same entity. The survey reports that three KR experts evaluating 250 `owl:sameAs` links agreed poorly: one confirmed only 73 as correct while the others judged 132–181 correct. If experts cannot agree on the truth of an identity link, an automated (or LLM) judge certainly measures something noisier than "same person".

### Empirical error rates (the sameAs web)

The literature's error estimates for deployed `owl:sameAs` span an order of magnitude, which is itself the point — nobody knows because the semantics were never honoured in practice ([survey, ar5iv](https://ar5iv.labs.arxiv.org/html/1907.10528)):

- Hogan et al. (2012): ~**2.8%** erroneous.
- Halpin et al. (2010): ~**20%** erroneous.
- Raad et al. (2018), *sameAs.cc*: took the **transitive closure of 558M `owl:sameAs` statements** and showed the closure manufactured false equivalences — "over 177K names referring to a number of different countries, cities and people" collapsed into single equivalence classes ([sameAs.cc paper](https://www.researchgate.net/publication/325529520_sameAscc_The_Closure_of_500M_owlsameAs_Statements)). This is the transitive-closure hazard in its purest form: one bad link fuses two large, correct clusters, and equivalence's transitivity propagates the error across everything.

The lesson for zuihitsu is direct: **transitivity is the amplifier.** A union-find `class_id` (the current design) is exactly a transitive-closure structure, so a single wrong merge does not corrupt one pair — it corrupts the whole class, and every read that traverses the class inherits the corruption. This is the structural form of failure class 5 ("identity is binary and entangled with storage").

### Weaker identity vocabularies that were proposed

The field's response was to stop overloading one hard predicate and introduce a *graded* vocabulary:

- **SKOS mapping relations** — `skos:exactMatch` (weaker than `sameAs`: mapping-level equivalence, non-transitive by intent) and `skos:closeMatch` (explicitly "similar, use with caution, do not propagate"). The survey counts 566K and 371K triples of each in a 2015 crawl ([ar5iv](https://ar5iv.labs.arxiv.org/html/1907.10528)).
- **`umbel:isLike`** (461K triples) — an explicitly *probable-but-unconfirmed* identity link, meant to carry the "we think these are the same but haven't verified" state that `sameAs` cannot.
- **`wdt:P2888`** ("exact match", Wikidata, 356K triples).
- **Contextual identity.** The survey formalises a *context* Π as "a subset of all properties Ψ necessary and sufficient to determine indiscernibility" — i.e. two entities are identical *with respect to a chosen property set*, not absolutely. Idrissou et al. refine this into **indiscernibility properties (Γ)** vs **propagation properties (Λ)**: Γ decides whether two things are the same in this context; Λ decides which facts are allowed to flow across the link once they are ([ar5iv](https://ar5iv.labs.arxiv.org/html/1907.10528)). This Γ/Λ split is directly useful below — it is the formal skeleton of "merged for recall, not merged for disclosure".

### Contextual / relative identity in KR (the philosophical root)

Geach's **relative identity** (1972) holds that "is the same as" is always elliptical for "is the same *F* as": two printed letters "aa" are *the same type* but *different tokens* ([AltExploit summary](https://altexploit.wordpress.com/2017/05/05/geach-and-relative-identity/)). There is no absolute identity, only sameness relative to a sortal. The near-identity work of Recasens, Hovy, and Martí ("Identity, Non-identity, and Near-identity", *Lingua* 2011) turns this into a *scalar* relation — coreference as sameness "at the granularity level relevant to the context", with near-identity as a first-class middle state between identical and distinct ([PDF](https://www.cs.cmu.edu/~hovy/papers/11Lingua-near-identity-coref.pdf)).

**Takeaway for zuihitsu:** the entire field converged on the same conclusion — *identity should not be a single hard equivalence.* It should be (a) graded (a confidence, not a boolean), (b) contextual (same-for-recall need not mean same-for-disclosure — the Γ/Λ split), and (c) revisable (a link you can retract without the transitive closure having fossilised the error). The current design's operator-gated `same_as` is the conservative special case: it makes the link *rarely wrong* by making a human confirm it, but it does not make a wrong one *cheap to undo*, and it offers no middle state.

---

## 2. Probabilistic and collective entity resolution — and merge reversal in production

### Fellegi–Sunter: the probabilistic baseline and its precise weakness

Fellegi & Sunter (1969) is the foundational model of probabilistic record linkage. For each candidate pair it computes, per field, an **m-probability** (probability the field agrees given the pair is a true match) and a **u-probability** (probability the field agrees by chance given a non-match); the log-likelihood-ratio weights sum to a score, and two thresholds partition the score line into **match / non-match / indeterminate**, with the middle band routed to human "clerical review" ([Census working paper](https://www.census.gov/content/dam/Census/library/working-papers/1993/adrm/rr93-12.pdf); [Murray, Blocking & Fellegi-Sunter](http://www2.stat.duke.edu/~rcs46/linkage_readings/2015-Murray-Blocking-FellegiSunter.pdf)).

Two things matter here. First, the **three-way outcome with an explicit review band** is precisely the "tentative vs confirmed" structure #94 wants — Fellegi–Sunter has always had a "don't decide yet, escalate" region, and it is not a failure mode but the design. Second, the model's soundness rests on an **independence assumption**: the field agreement probabilities must be independently generated. This is the formal statement of #94's recitation attack — recited facts are *not* independently generated evidence, so summing their weights (the "scaling coincidences" move) double-counts a single act of copying. Fellegi–Sunter tells you *why* fact-overlap adjudication is unsound: it violates the independence premise the arithmetic depends on.

### Bhattacharya & Getoor: collective ER on relational structure

Bhattacharya & Getoor, "Collective Entity Resolution in Relational Data" (*ACM TKDD* 1(1), 2007), is the load-bearing citation for #94's strongest passive signal ([linqs PDF](https://linqs.org/assets/resources/bhattacharya-tkdd07.pdf), [Duke slides](https://courses.cs.duke.edu/spring17/compsci590.1/lectures/16-collective-er.pdf)). Instead of judging pairs independently on attribute overlap, it resolves references *collectively*, using **cluster-to-cluster** comparisons and **relational evidence**: two references are more likely co-referent if their *neighbours in the relationship graph* are themselves co-referent. Co-authorship is the canonical example — two "J. Smith" references resolve together when they share co-authors, not when their name strings match.

Why this resists recitation (the #94 argument, now grounded): relational evidence is **structurally expensive to forge**. An attacker reciting the operator's holiday facts produces attribute overlap cheaply, but to forge relational evidence they must independently appear connected to *the same third parties, events, and places, as attested by different tellers*. The memory graph already has this structure natively — participation, acquaintance, placement, composition are seed relations — and does not yet use it as merge evidence. This is a real, available, under-exploited signal. **Caveat (flagged):** collective ER assumes the relational graph itself is mostly trustworthy; a patient attacker who first insinuates themselves into the network (befriends the same people, attends the same events) can eventually forge relational evidence too. It raises the cost of attack; it does not close it. This matches #94's own "accretion favours the patient attacker" hazard.

### Merge reversal in production: MDM and the golden-record pattern

Production master-data-management (MDM) systems have confronted "we merged two customers who turned out to be different people" for decades, and the mature answer is **non-destructive merge**:

- **Reltio** creates a unique URI for each entity *before* merging so the merge is reversible: "a merged record… can be unmerged to the state of their original records, after which the parent victim record is restored" ([Reltio docs](https://docs.reltio.com/en/explore/get-a-crash-course/get-ready-to-turn-your-data-into-action/learn-about-multidomain-mdm/reltio-match-merge-and-survivorship)). The originals are never destroyed; the merge is an overlay.
- **Survivorship rules** decide, per attribute, which source value "wins" in the golden record when merged records conflict ([mdmlist: three survivorship approaches](https://mdmlist.com/2019/08/22/three-master-data-survivorship-approaches/); [Profisee](https://profisee.com/platform/golden-record-management/)). Crucially the golden record is a *derived view*, recomputable from surviving sources, not a destructive rewrite.

The zuihitsu architecture is *already* in the strong position MDM had to engineer toward: stubs persist forever, `same_as` is an overlay link over them (not a destructive union), and `told_by` is per-stub so attribution survives severance. This is exactly the "unmerge to original state" capability, and the log makes "the world as if never merged" a fold-filter rather than a forensic restore. **The gap is the same one #94 names**: derived content (link inferences, distilled descriptions, model conclusions baked into prose) does not record which identity assumptions were in force when it was produced, so severance cannot mechanically find and undo the dependents.

### Incremental / repairing ER: reclustering wrong decisions

The incremental-ER literature is where "repair a wrong merge/split" is studied directly. Gruenheid, Dong & Srivastava and successors frame ER as a *maintained clustering* that must absorb new evidence and **repair prior clusters**:

- **n-Depth Reclustering (nDR)** repairs existing clusters when new entities arrive, reclustering a bounded portion of the similarity graph rather than blindly attaching (Nentwig & Rahm, "Incremental Multi-source Entity Resolution for Knowledge Graph Completion", ESWC 2020) ([PMC](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC7250616/)).
- A **clustering-based framework for incrementally repairing ER** maintains a **provenance index of the evidence for each clustering decision**, so when an erroneous match/non-match is detected the system can "trace the evidence… split the incorrect transitive closure into multiple ones representing the correct clusters" (Springer, 2016) ([ResearchGate](https://www.researchgate.net/publication/301321207_A_Clustering-Based_Framework_for_Incrementally_Repairing_Entity_Resolution)).

That last system independently reinvented the ATMS idea below: *index the provenance of each merge decision so you can split the closure when a decision is retracted.* This is precisely the assumption-stamping #94 proposes, arrived at from the ER side rather than the TMS side. Both literatures point at the same mechanism.

---

## 3. Truth maintenance — assumption-stamped derivations, and their cost

### JTMS (Doyle) vs ATMS (de Kleer)

- **Doyle's JTMS** (1979) maintains one consistent belief set. Each node is labelled **IN** (believed) or **OUT** (not believed); a **justification** is a triple ⟨in-list, out-list, consequent⟩, and the consequent is IN iff every in-list node is IN and no out-list node is IN. Retracting an assumption triggers **dependency-directed backtracking** — recompute labels, retract dependents ([stilpo docs](https://stilpo.readthedocs.io/en/latest/jtms.html); [Northwestern TMS notes](https://users.cs.northwestern.edu/~forbus/c44/Lectures/TMS%20Intro.pdf)). One context at a time; cheap labels; expensive to explore alternatives.
- **de Kleer's ATMS** (1986) maintains *all* consistent contexts simultaneously. Every node carries a **label**: the set of *minimal environments* (sets of assumptions) under which it holds. **Nogoods** are minimal inconsistent environments. Retracting an assumption is then *free at query time* — you simply ask which nodes still have a non-empty label not depending on that assumption ([de Kleer, Foundations of ATMS](https://dekleer.org/Publications/Foundations%20of%20Assumption-Based%20Truth%20Maintenance%20Systems.pdf); [Wotawa chapter](https://www.dbai.tuwien.ac.at/staff/wotawa/atmschapter1.pdf)).

The **cost profile** is the well-known trade: the ATMS front-loads all the work into **label maintenance**, and labels can grow combinatorially — a node reachable through many assumption combinations carries many minimal environments, the "label explosion" (the search confirmed labels are the ATMS's central and dominant activity; the explosion is the standard critique of scaling ATMS to many assumptions). JTMS is cheap per-state but pays a re-derivation cost on every context switch.

### Modern echo: provenance semirings (why-provenance as a practical ATMS)

Green, Karvounarakis & Tannen, "Provenance Semirings" (PODS 2007), is the database-theoretic reincarnation ([PDF](https://web.cs.ucdavis.edu/~green/papers/pods07.pdf)). Each base tuple is annotated with an element of a commutative semiring K; query evaluation *propagates* the annotations — join uses ⊗ (a result needs both inputs), union uses ⊕ (either input suffices). Choosing K picks the provenance flavour: the semiring of **polynomials over the base-tuple identifiers**, ℕ[X], is the *most informative* provenance, and specialises down to why-provenance, trust, probability, and bag semantics as homomorphic images ([Tannen, semiring framework slides](https://www.cis.upenn.edu/~val/15MayPODS.pdf)).

The connection worth stating precisely: **a provenance polynomial *is* an ATMS label.** A monomial (product of base-tuple ids) is an environment — the set of assumptions that jointly justify the derived fact; the sum of monomials is the set of alternative minimal environments; retracting a base tuple means setting its variable to the semiring's annihilator and re-evaluating. Databases made this *practical* by (a) keeping annotations on *base* tuples only and letting the algebra propagate, and (b) letting the application choose *how much* provenance to keep via the semiring — the Boolean semiring keeps only "does it still hold", the polynomial semiring keeps the full dependency set. That knob is the direct answer to ATMS label explosion: **keep only as much of the label as severance actually needs.**

### Concrete shape for an event-sourced graph (the practical assessment #94 asked for)

For zuihitsu specifically, where derivations *are events*:

- **Per-derivation assumption sets.** Every derived event (a distilled description regeneration, a link inference, a consolidation absorb-and-attest, a maintenance-pass rewrite) carries a small **assumption-set stamp**: the set of *revocable assumptions in force at production time*. The identity-class assumptions are the important ones — "this event was derived treating stub A and stub B as the same person (assumption `merge#k`)". This is `produced_by` extended from "which template" to "under which assumptions". The stamp is small because the number of *revocable* assumptions touching any one derivation is tiny (usually zero or one merge), unlike a general ATMS where every inference step is an assumption.
- **Severance = a fold filter.** Retracting `merge#k` (recording a `MergeSevered` event) does not mutate history. It reclassifies: on replay, any derived event stamped with `merge#k` is *voided* (treated as not-having-happened) and re-derived from the now-separated stubs. Because replay is already the system's evaluation function, "the world as if never merged" is `fold(events, filter = drop everything stamped merge#k)` — exactly the sameAs.cc "split the transitive closure" operation, but computed rather than forensically reconstructed. The union-find `class_id` becomes an *assumption-conditioned* view rather than a fossilised structure.
- **Re-derivation cost.** The JTMS-style cost lands here: on severance you must *re-run* the voided derivations (re-distil descriptions, re-embed, re-judge the absorbed consolidations) for the two now-separate stubs. This is the same cost profile as failure class 9 (hygiene re-derivation) — bounded by the number of derivations that touched the merged class, not the whole log. Confidence: **high** that the mechanism is correct and computable; **medium** on the cost being small in practice — it depends on how much distillation/consolidation a merged identity accumulates before severance, which is empirical.
- **The irreducible residue.** #94 states it and TMS confirms it: *outbound disclosure cannot be re-folded.* An assumption-stamp can void a derived brief entry, but it cannot un-say a confidence already spoken across the identity boundary to a real person. This is why the reversibility tiering (§below and #94) is not optional decoration — TMS gives you mechanical severance of *internal* state and is silent on *external* effects, so the design must gate exactly the external, irreversible effects on a harder bar.

---

## 4. Belief — revision and credence over attestations

### AGM and why a personal agent needs its *non-prioritised* variants

Classical AGM revision (Alchourrón, Gärdenfors, Makinson) obeys the **Success postulate**: the newly-asserted belief is *always* accepted, and the old beliefs bend around it ([arXiv 2108.07769](https://arxiv.org/pdf/2108.07769)). This is exactly wrong for an agent aggregating attestations from tellers of varying reliability — a teller is *not always right*, and blind acceptance is how a confident liar overwrites the truth. The relevant family is **non-prioritised revision (NPR)**, which drops Success:

- **Screened revision** (Makinson): incoming information is checked against a screen of core beliefs; if it conflicts with the protected core it is *not* accepted ([survey context, arXiv 2409.07119](https://arxiv.org/pdf/2409.07119)).
- **Credibility-limited revision** (Hansson, Fermé, Cantwell): revision applies *only if the new sentence falls in a credible set*; otherwise the belief state is unchanged ([ResearchGate](https://www.researchgate.net/publication/38384419_Credibility_Limited_Revision); [arXiv 2409.07119](https://arxiv.org/pdf/2409.07119)). This maps onto zuihitsu directly: a teller's credibility gates whether their attestation is allowed to overturn an existing belief.
- **Source-sensitive belief change** makes the source's reliability an explicit input to the revision operator ([arXiv 1704.03396](https://arxiv.org/pdf/1704.03396)) — the formal home for "weight the attestation by who's attesting".

**Takeaway:** the current design's per-attester attestation model is already the right *shape* for NPR — an entry is "a fact a set of tellers stand behind", and each attestation is a source-tagged assertion. What's missing (failure class 8) is the *credibility* input: nothing weights an attestation by teller reliability or lets a low-credibility attestation fail to overturn a high-credibility one. AGM/NPR says: don't blindly accept; screen against a credibility measure.

### Credence representation — which formalism fits attestation aggregation

Three candidates, in increasing fit:

- **Bayesian credences** — a single probability per belief. Clean, but it *cannot distinguish* "I have strong balanced evidence for 0.5" from "I have no evidence, so 0.5 by default". For an agent that must know *how much it knows* about an identity or a fact, this conflation is disqualifying.
- **Dempster–Shafer (belief functions)** — represents ignorance explicitly via mass on the whole frame, but its combination rule (Dempster's rule) famously misbehaves under high conflict (the Zadeh counterexample), and fusing many independent attestations is awkward.
- **Subjective Logic** (Jøsang, 1997) — the best fit, and #94-adjacent. An opinion is a tuple **(belief, disbelief, uncertainty, base rate)** with b+d+u=1, so it separates "how much I believe" from "how sure I am I know" — *exactly* the distinction Bayesian credence loses ([diva-portal evaluation](https://www.diva-portal.org/smash/get/diva2:3403/FULLTEXT02); [Multi-Source Fusion Operations in Subjective Logic, arXiv 1805.01388](https://ar5iv.labs.arxiv.org/html/1805.01388)). Crucially it supplies exactly the two operators an attestation-aggregating agent needs:
  - **Trust discounting** — "the trust discounting operator increases the uncertainty of an opinion according to a separate opinion of the reliability of that [source]" ([search-sourced summary of the discounting operator](https://www.researchgate.net/publication/27470225_Trust_Network_Analysis_with_Subjective_Logic)). This is the credibility-limited-revision idea made arithmetic: an attestation from a partially-reliable teller is *discounted* (belief mass bleeds into uncertainty) rather than accepted at face value.
  - **Fusion operators** — cumulative fusion (for independent sources) and averaging fusion (for dependent ones) aggregate multiple attestations of the same fact into one opinion ([Cumulative and averaging fusion of beliefs, ScienceDirect](https://www.sciencedirect.com/science/article/abs/pii/S156625350900044X)). And the *independent vs dependent* distinction is the recitation defence again: two attestations that are actually one recited fact should be fused as *dependent* (no confidence gain), not *independent* (spurious confidence gain) — the same independence premise Fellegi–Sunter needs.
  - It is grounded in Dempster–Shafer but "integrates uncertainty directly rather than adding it as a separate component… so fusion becomes easier" ([diva-portal](https://www.diva-portal.org/smash/get/diva2:3403/FULLTEXT02)), and maps to a Beta/Dirichlet distribution so an opinion ↔ evidence-count correspondence exists (evidence-based subjective logic; [Springer](https://link.springer.com/article/10.1007/s10207-015-0298-5)).

  **Caveat (flagged):** subjective logic has critics — "Can we trust subjective logic for information fusion?" ([ResearchGate](https://www.researchgate.net/publication/262735429_Can_we_trust_subjective_logic_for_information_fusion)) argues some fusion operators are not associative/behave unexpectedly. The recommendation is to use SL's *representation* (belief/disbelief/uncertainty triple + base rate, and the discounting operator) as the credence datatype, and to be conservative about which fusion operator is claimed to be principled.

### Calibration — the neural writer's confidence cannot be trusted raw

If credence is to be produced by the LLM, the calibration literature is a hard warning. LLM **verbalised confidence is systematically overconfident** and **collapses to coarse saturated values** — "standard verbalized confidence methods often collapse to coarse, saturated predictions (e.g., 0.9 or 1.0)" ([Asking Is Not Enough, arXiv 2605.27752](https://arxiv.org/pdf/2605.27752)); "LLMs… tend to be overconfident, potentially imitating human patterns", driven by "a stable model-internal mechanism rather than a task-specific artifact" ([Wired for Overconfidence, arXiv 2604.01457](https://arxiv.org/html/2604.01457); [LLMs Are Overconfident in Their Own Responses, arXiv 2606.03437](https://arxiv.org/pdf/2606.03437)). Worse, the number is **elicitation-protocol-sensitive** — the same model gives different confidence depending on how you ask ([arXiv 2605.27752](https://arxiv.org/pdf/2605.27752)).

**Design consequence:** the agent must *not* mint fine-grained numeric credences (0.87) — that is false precision the model cannot actually produce (failure class 11, the unverified neural writer). Credence should come from **counting evidence**, not from asking the model to introspect a probability: subjective logic's opinion ↔ evidence-count mapping lets the *substrate* derive an opinion from the number and independence of attestations and the tellers' track records, rather than the model verbalising a number. Where the model must supply a judgement, quantise to a **coarse ordinal** (e.g. suspected / likely / confirmed) rather than a spurious real, matching the granularity the model can actually calibrate.

### Surfacing credence without false precision

The agent- and user-facing surface should stay ordinal and provenance-anchored, never numeric. Rendering "3 people told me this, one of whom is usually unreliable" (evidence) beats "confidence 0.72" (false precision). This is consistent with the fixed-point that the agent surface stays handle-shaped and simple: credence lives in the substrate as an opinion tuple, and surfaces as a coarse, explainable posture with its evidence attached.

---

## 5. Self-model governance (#15) and keeping operational identity simple (#104)

### What cognitive architectures do about self-models

Metacognition — "an agent reasoning about its own reasoning… reasoning about capabilities and knowledge" — is supported by only about a third of surveyed cognitive architectures, chiefly the symbolic/hybrid ones (Kotseruba & Tsotsos, "40 Years of Cognitive Architecture Research") ([arXiv 1610.08602](https://arxiv.org/pdf/1610.08602)). The most transferable design lesson comes from how Soar and ACT-R treat self-metadata: there is a **"wall" between agent data and agent metadata** — architectural metadata (activation, utility, retrieval bookkeeping) is "used exclusively by architectural processes… not represented in working memory and… cannot be tested or directly modified by agent data processing" ([ACT-R/Soar comparison, ACS 2021](https://advancesincognitivesystems.github.io/acs2021/data/ACS-21_paper_6.pdf)).

This wall is the exact design principle #104 needs. In #104 the agent *reasons over its own identity-resolution machinery* — it inspects handle objects, tests `p_forum == p_chat` for Lua object identity, second-guesses whether a merge "really" landed — and gets it wrong 7/10 times. The cognitive-architecture answer: **the substrate's identity bookkeeping should be architectural metadata behind a wall, not agent-visible data the agent reasons over.** The agent should never see "two stubs and a class_id and a merge state"; it should see *one resolved person*, with resolution performed by the substrate before the agent ever reasons.

### The design lesson: simple operational view, rich revocable substrate

This is the unifying prescription across the lane, and it directly answers the redesign's fixed point that "the agent-facing surface stays handle-shaped and simple; the ontology may be arbitrarily rich underneath":

- **One handle per person, at the surface.** The agent addresses a person by a single stable handle. It never chooses between `person/priya`, `person/priya@chat`, and `person/priya@forum` — that choice is #104's cluster-1 bug (5/7 failures were the agent minting a bare handle instead of reusing the substrate's platform-qualified stub). The substrate resolves the current speaker to their canonical handle and *hands it to the agent*; the agent writes to what it's given, not to a name it guesses.
- **Resolution done by the substrate.** All the composite/revocable machinery — stubs, `same_as`-as-assumption, class views, assumption-stamped derivations, severance — lives below the wall. The agent's *operational* view is "this is Priya"; the substrate's *representational* view is "class of stubs {chat, forum} joined under revocable assumption merge#k". #104's cluster-2 bug (agent distrusts a landed merge, tests handle equality) is the agent trying to *do resolution itself* over substrate internals it shouldn't see — behind the wall, there is nothing for it to second-guess.
- **`self` governance (#15).** The self-model is the same shape: an `Agent`-authority path (option B in #15) lets the agent append self-*observations* that feed a *regenerable description*, while the operator-fixed *charter/voice* is drawn only from `told_by ∈ {bootstrap, Operator}` entries — the persona cannot drift even as the description evolves. This is the Soar/ACT-R wall applied reflexively: agent-authored self-data is metadata feeding the description; the load-bearing voice is protected architectural data the agent cannot overwrite. The credence and assumption-stamping machinery applies to `self` too — a self-observation is an attestation by the agent (a teller of *known, but not infallible*, reliability), revisable like any other.

---

## Implications for zuihitsu

### Recommended identity model: revocable composite identity with assumption-stamped derivations

I recommend adopting (and I argue *for*, not against) the #94 direction, now grounded in the literature:

1. **Identity is a graded, revocable overlay, never a hard transitive equivalence.** Drop union-find `class_id` as *the* identity structure; keep stubs-forever + `same_as`-as-assumption. Each merge is a first-class **assumption** (`merge#k`) with a **credence** (a subjective-logic opinion), not a boolean. The literature is unanimous that hard `owl:sameAs`-style equivalence is the wrong primitive (Halpin; the sameAs survey; sameAs.cc's transitive-closure disaster; Geach/near-identity). The class becomes an *assumption-conditioned view* folded from the log, so a wrong merge is a retraction, not a fossil.

2. **Reversibility tiering (the organising principle #94 proposes — endorsed).** Split merge consequences by reversibility:
   - **Revocable effects** (unified recall, shared inference, composite briefs, distilled descriptions) ride a *tentative accretion merge* that strengthens with corroboration and crumbles on divergence. Safe to be imperfect *because severance genuinely undoes them* — and the Γ/Λ contextual-identity split ([sameAs survey](https://ar5iv.labs.arxiv.org/html/1907.10528)) gives the formal license: "same for recall" (Γ) need not propagate "same for disclosure" (Λ).
   - **Irrevocable effects** (a confidence *spoken across* the identity boundary to a real person) stay gated on the hard bar: a completed challenge-response (#93) or evidence past a far stricter threshold. TMS gives mechanical severance of *internal* state and is silent on *external* disclosure — so the design *must* gate the irreversible outbound step separately. Fellegi–Sunter's three-way match/review/non-match with a review band is the precedent for "tentative vs confirmed" being a first-class outcome, not a hack.

3. **Assumption-stamped derivations (ATMS, made practical via provenance semirings).** Every derived event carries a small assumption-set stamp — the revocable assumptions in force at production time, chiefly which merges it treated as holding. Severance is then a **fold filter**: void events stamped with the retracted merge and re-derive from separated stubs. Keep the stamp minimal (only revocable assumptions, not every inference step) to avoid ATMS label explosion — the provenance-semiring lesson that you keep only as much of the label as severance needs. Event sourcing already makes "the world as if never merged" computable; the stamp is what makes it *findable*.

4. **Collective ER as the passive signal (Bhattacharya & Getoor).** Use the graph's native relational structure (participation, acquaintance, placement) as merge evidence — two stubs independently linked to the same third parties by *different* tellers is expensive to forge, unlike recited attribute overlap. This is the strongest passive signal and is currently unused. Fellegi–Sunter names *why* fact-overlap fails (independence violated by recitation); collective ER supplies the evidence that recitation *can't* cheaply fake. Caveat: it raises attacker cost, doesn't eliminate it (the patient-attacker hazard).

### Recommended credence model

Adopt **subjective-logic opinions** (belief / disbelief / uncertainty / base rate) as the substrate credence datatype for both facts and merges, because it separates strength-of-belief from amount-of-evidence — the distinction Bayesian credence loses and the one an aggregating agent most needs. Derive opinions from **evidence counting** (number and independence of attestations, teller track record) via the opinion↔evidence-count mapping, *not* from LLM-verbalised numbers, because verbalised confidence is overconfident, saturated, and elicitation-sensitive ([arXiv 2605.27752](https://arxiv.org/pdf/2605.27752), [2604.01457](https://arxiv.org/html/2604.01457)). Use SL's **trust discounting** to weight attestations by teller reliability (this is credibility-limited / source-sensitive belief revision made arithmetic) and its **independent-vs-dependent fusion** distinction to refuse spurious confidence gain from recited (dependent) attestations. Surface credence to agent and user as a **coarse ordinal posture with its evidence attached** (e.g. "suspected / likely / confirmed", "3 tellers, one unreliable"), never a spurious real number. Use SL's representation confidently; be conservative about claiming any one fusion operator is principled (the "Can we trust subjective logic" critique).

### How the failure classes and issues are addressed

- **Failure class 5 (identity binary & entangled with storage):** replaced by graded, revocable, assumption-stamped composite identity; `same_as` becomes a credence-bearing assumption, not a destructive transitive union; resolution decoupled from storage by moving it below the substrate wall.
- **Failure class 7 / #104 (identity complexity leaks into behaviour):** solved by the Soar/ACT-R *data/metadata wall* — the agent sees one resolved handle per person and never reasons over stubs, class_ids, or merge state. Cluster-1 (bare-handle minting) dies because the substrate hands the agent the canonical handle for the current speaker rather than making it guess. Cluster-2 (distrusting a landed merge, testing handle `==`) dies because there is no substrate internal for the agent to second-guess — resolution already happened.
- **Failure class 8 (no credence model):** subjective-logic opinions over attestations, derived from evidence counts and teller reliability, replacing episodic prose arbitration notes; non-prioritised belief revision (credibility-limited / source-sensitive) replaces blind acceptance.
- **#94 (autonomous identity unification):** the composed endgame is coherent and literature-backed — collective-ER relational evidence (passive) + challenge-response/#93 (active, gating the irreversible tier) + assumption-stamped severance (making revocable-tier merges safe to be imperfect) + reversibility-tiered semantics (tying it together). The recitation attack is formally explained (Fellegi–Sunter independence violation) and its defence identified (relational evidence + dependent-source fusion).
- **#104 (merged identity fails to relay history):** both failure shapes are *architecture* bugs the wall removes; the substrate-hands-canonical-handle fix directly addresses cluster 1, and metadata-behind-the-wall removes cluster 2's second-guessing surface.
- **#15 (self-model governance):** adopt option B (an `Agent` authority) but constrained by the wall — agent self-observations feed a regenerable description (metadata), while the operator-fixed voice draws only from `bootstrap`/`Operator` charter entries (protected data). Self-observations are attestations by a known-but-fallible teller, revisable under the same credence and revision machinery as any other belief.

### Open uncertainties (for the adversarial-verification pass)

- **Cost of re-derivation on severance** is empirical (how much distillation a merged class accumulates); the mechanism is sound but the "cheap in practice" claim is unverified — flagged medium confidence.
- **Crumble/accretion thresholds** for tentative merges are an empirical tuning problem needing data (genuine same-person profiles also diverge); #94 already flags this.
- **Subjective-logic fusion operator choice** has principled critics; recommend using the representation + discounting, and validating any specific fusion operator against the "Can we trust subjective logic" objections before relying on it.
- **Exact minimal shape of the assumption stamp** (per-event set vs a shared assumption-environment table referenced by id) is a design choice trading storage against fold-time work — not resolved here.
