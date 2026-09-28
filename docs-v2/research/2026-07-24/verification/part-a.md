# Verification pass — Part A (sections 2.1, 2.2, 2.3 and their uses in 3.1–3.5, 3.8–3.9)

Adversarial verification of the fact-shape, provenance-privacy, and identity-belief halves of the
report. Today's date 2026-07-23; future-dated arXiv IDs were fetched directly rather than judged from
memory.

## Claim-by-claim verdicts

| # | Claim (report location) | Verdict | Evidence / corrected wording |
|---|---|---|---|
| 1 | ODKE+ reports 98.8% precision by constraining extraction to per-type ontology snippets (2.1, 3.1) | **CONFIRMED** | arXiv:2509.04696 abstract: 98.8% precision while ingesting **19 million** high-confidence facts, dynamically generated per-entity-type ontology snippets aligning extractions to schema constraints across 195 predicates. Report says "millions of facts"; actual is 19M — accurate, could be sharpened. |
| 2 | "~87% hallucination reduction" and "85% of errors corrected" schema-grounding figures (2.1, flagged) | **UNREACHABLE** (already flagged) | arXiv:2510.20345 PDF would not extract the body text; could not locate the figures as primary measurements. Report already flags these as secondary-sourced with the direction solid, not the numbers — consistent with what I found. No new support; keep the flag. |
| 3 | Once a graph is genuinely n-ary, the reification/serialization encoding "matters less" (3.1, flagged) | **CONFIRMED** | arXiv:2503.21804 abstract: "in complex HRKGs, the differences among MRMs in the LP tasks are minimal." Nuance the report omits: in *simple* graphs reification outperforms singleton property — the "matters less" holds specifically at genuine hyper-relational complexity, which is exactly the report's framing. |
| 4 | Wikidata keeps qualifiers exactly one level deep because a qualifier modifying a qualifier makes scope ambiguous (2.1, 3.1, 3.3) | **CONFIRMED** (claim); **CORRECTED** (citation) | Verbatim from Wikidata **Help:Qualifiers**: qualifiers "should not be used to modify the values of other qualifiers on a main statement, since this can make the meaning of the qualifier ambiguous." Report §2.1 attributes this to arXiv:2304.03375, which does **not** state the one-level rule (it only says "the qualifiers of the inferred statement are often a combination of the qualifiers in the rule condition"). Cite Help:Qualifiers for the one-level discipline. |
| 5 | owl:sameAs error rates ~2.8% (Hogan 2012) and ~20% (Halpin 2010) (2.3) | **CONFIRMED** | sameAs survey (ar5iv 1907.10528) verbatim: "between 2.8% Hogan et al. (2012) and 20% Halpin et al. (2010)." |
| 6 | sameAs.cc transitive closure of 558M statements collapsed 177K distinct entities into single classes (2.3, 3.4) | **CONFIRMED** | Survey: closure of "over half a billion statements" produced "false equivalence of over 177K names referring to a number of different countries, cities and people." The precise "558M" is the sameAs.cc paper's own figure; "over half a billion" corroborates. |
| 7 | Gamma/Lambda (indiscernibility vs propagation) split, Idrissou et al. (2.3, 3.4) | **CONFIRMED** | Survey presents Γ (indiscernibility-for-identity) vs Λ (propagation) with the two-medicines example, exactly as the report uses it. |
| 8 | Green/Karvounarakis/Tannen PODS 2007: "a provenance polynomial **is** an ATMS label," monomial = environment, retract = set variable to annihilator (2.3) | **CORRECTED** | The N[X]-polynomial-as-most-general-provenance and annihilator-on-retraction mechanics are genuinely from the paper. But the paper makes **no mention of ATMS / de Kleer / truth maintenance** (confirmed by search + the paper's own contribution summary). The "provenance polynomial IS an ATMS label" identification is the **lane's synthesis**, not a claim of the cited paper. The report presents it as if sourced to PODS 2007. |
| 9 | "Can we trust subjective logic for information fusion?" argues some fusion operators are not associative (3.5, 8.2) | **CONFIRMED** (and stronger) | The critique exists (ResearchGate 262735429). Confirmed: "Not all subjective logic operators are associative, and therefore multi-source fusion is not well-defined." It goes **further** than the report states: it also alleges "defects in the SL fusion rule and problems in the link between opinion and Beta probability density functions" — the opinion↔Beta link the report *relies on* for evidence-count-derived credence. |
| 10 | LLM verbalized confidence collapses to coarse saturated values (0.9/1.0) and is elicitation-protocol-sensitive (2.3, 3.5) | **CONFIRMED** | arXiv:2605.27752 = "Asking Is Not Enough: Protocol Sensitivity in LLM Confidence Calibration" (Kim & Kang). Confirms saturation to extreme values and protocol sensitivity. Real, future-dated, fetched. |
| 11 | LLMs systematically overconfident via a stable model-internal mechanism, not a task-specific artifact (2.3, 3.5) | **CONFIRMED** | arXiv:2604.01457 = "Wired for Overconfidence." Verbatim: "verbalized overconfidence is driven by a stable model-internal mechanism rather than a benchmark-specific artifact." Real, future-dated, fetched. |
| 12 | Nanopub: retraction is a separate signed nanopublication (npx:retracts); who-may-retract is a same-key signature check anchored to an introduction nanopublication (2.2, 3.8) | **CONFIRMED** | PMC7959648 verbatim: `npx:retracts` in a separate retraction nanopublication; `npx:supersedes` for updates; "only considered valid if... signed with the same key pair," anchored to an introduction nanopublication binding ORCID to key. |
| 13 | Fellegi–Sunter arithmetic depends on field-agreement independence; three-way match/review/non-match with a clerical-review band (2.3) | **CONFIRMED** | Standard result confirmed by multiple sources: FS assumes agreement is **conditionally** independent given match status; the three-region rule (match / clerical-review / non-match) is the model's design. The report's "independently generated" is a slightly loose paraphrase of *conditional* independence, but the recitation-as-independence-violation framing is a sound analogy. |
| 14 | Bhattacharya & Getoor: collective ER where two references co-refer when their neighbours co-refer (2.3, 3.4) | **CONFIRMED** (result; PDF unextractable) | The defining thesis of the TKDD 2007 paper; the PDF would not extract but the claim is the paper's central, well-established contribution. No contradicting evidence. |
| 15 | Reltio ships non-destructive, reversible (unmerge) MDM merge (2.3, 3.4) | **CONFIRMED** | Reltio docs: unique URIs minted before merge "enables you to unmerge them later." Caveat found: API `matchBeforeCreate=true` merges are *not* unmergeable — does not affect the report's use of it as an existence proof. |
| 16 | Barth et al. 2006 formalize CI in LTL over traces with positive (permitted) and negative (prohibited) norms; transmission principle as a temporal condition (2.2, 3.8) | **CONFIRMED** | The IEEE S&P 2006 paper formalizes transmission norms over "past and future actions by both the subject and users," "positive or negative depending on whether they refer to actions that are permitted or prohibited," with the five parameters incl. transmission principle. The LTL/temporal framing is confirmed. |
| 17 | OWL/LinkML deprecation predicate names (2.1, 3.3, flagged "confirm exact names") | **CONFIRMED** (claim); **CORRECTED** (capitalization) | Correct OWL spellings are `owl:DeprecatedClass` and `owl:DeprecatedProperty` (capital D), plus `owl:backwardCompatibleWith` / `owl:priorVersion` (W3C). LinkML has a `deprecated` slot mapping to `owl:deprecated`. Report writes lowercase `owl:deprecatedClass`/`Property` — fix the case. |

**Verdict counts:** CONFIRMED 14 · CORRECTED 3 (claims 4, 8, 17 — note 4 and 17 are confirmed on substance, corrected only on citation/spelling) · UNSUPPORTED 0 · UNREACHABLE 1 (claim 2, already flagged by the report).

The only substantive correction is claim 8 (the ATMS attribution). Everything load-bearing verified; no
claim was found flatly false.

---

## Reasoned judgement (a): is "severance is a fold filter" sound given event-sourced replay?

**The design (grounding + §3.4).** The event log is the sole truth, replay is deterministic, and
**model/embedder calls happen at record time only, never at replay**. Every derived event carries an
assumption stamp naming the revocable merges in force (`merge#k`). Severance appends a `MergeSevered`
event; on replay, events stamped `merge#k` are voided.

**Where it holds.** The *voiding* half is genuinely a pure, deterministic fold: dropping events by a
recorded stamp requires no model call and is replay-safe, so "the world as if never merged, minus the
voided derivations" is computable exactly as `fold(events, drop everything stamped merge#k)`. This is
sound, and it is the sameAs.cc "split the closure" operation computed rather than reconstructed. The
claim survives.

**Where the phrasing over-reaches.** The report writes severance "voids stamped events **and re-derives
from the separated stubs**" as one "fold filter." Re-derivation is *not* part of the fold and cannot
happen at replay: re-distilling descriptions, re-embedding, and re-judging consolidations for the now-
separate stubs are record-time neural/embedder activities that append *new* events — a forward
maintenance pass, not a replay. Strictly, replay of the post-severance log yields a materialization with
the voided derivations simply *absent* (a hole); a later record-time pass refills it. The report's own
§8 honest-costs section already books this as "re-derivation cost on severance," so the model is
internally consistent — but the one-sentence "fold filter that voids stamped events and re-derives"
conflates a deterministic replay-time fold with a nondeterministic record-time regeneration.

**Judgement: SOUND, with one phrasing fix.** The fold-filter argument is correct for the load-bearing
property (internal state is mechanically severable and the drop is deterministic). Tighten the wording so
"fold filter" names only the deterministic voiding, and re-derivation is explicitly a subsequent record-
time pass — otherwise it reads as if new model calls happen at replay, which would violate the
determinism fixed point the whole design rests on.

## Reasoned judgement (b): does the SL "recitation defence" survive the report's own fusion-operator conservatism?

**The tension the task names.** §3.5/§8.2 say: adopt SL's representation and its trust-discounting
operator, but "be conservative about claiming any one fusion operator is principled." Yet the recitation
defence is stated as *independent-vs-dependent fusion* — refusing spurious confidence gain from recited
(dependent) attestations. If the fusion operator is not trusted, what is left of the defence?

**Decompose the defence into two separable parts.**
1. **Dependence detection** — recognizing that two attestations are one recited fact rather than two
   independent witnesses. This is a provenance/evidence-tracing determination (does attestation B derive
   from A?), delivered by the assumption-stamp / derivation-record machinery, and has **nothing to do
   with the fusion operator's algebra**.
2. **Aggregation arithmetic** — how much an opinion moves when combining N sources. This is where a
   specific fusion operator lives, and where the "Can we trust subjective logic" critique (non-
   associativity, undefined multi-source fusion) actually bites.

**The load-bearing part is (1), and it is operator-free.** The defence's essential act is the *refusal*
to treat dependent attestations as independent corroboration — a policy ("do not count a recitation as a
second witness"), not a computation. In the degenerate but correct case, dependent fusion of a recited
fact is simply *idempotent* ("no update"), and an idempotent combine is trivially associative, sidestepping
the critique entirely. So the recitation defence survives the conservatism: it needs only the qualitative
independent/dependent *distinction* plus trust discounting (which the report explicitly retains), not a
contested associative fusion formula. The defence is in fact strongest exactly where the operator worries
are weakest.

**One caveat the report under-states.** The critique attacks not only fusion associativity but "the link
between opinion and Beta probability density functions." The report's credence model derives opinions
"from counting evidence... via the opinion-to-evidence-count (Beta/Dirichlet) mapping" (§3.5) — so the
*evidence-count-to-opinion* step inherits a piece of the same critique, independently of any fusion
operator. The report's conservatism is currently scoped only to fusion operators; it should extend to
the Beta/Dirichlet correspondence it leans on for deriving credence in the first place.

**Judgement: SURVIVES.** The recitation defence rests on dependence-detection + a conservative no-gain
rule, not on the contested fusion arithmetic, so it holds even under maximal scepticism about SL fusion
operators. But add a sentence acknowledging that the opinion↔evidence-count (Beta) mapping is exposed to
the same critique the report cites, and is a second place to validate before relying on it.

---

## Recommended report edits

1. **Fix the provenance-semiring↔ATMS attribution (§2.3).** The identification is a synthesis, not a
   claim of the paper.
   - Current: "Assumption-stamped derivations are made practical by **provenance semirings**: a
     provenance polynomial is an ATMS label, a monomial is an environment, retracting a base tuple is
     setting its variable to the annihilator and re-evaluating ([Green, Karvounarakis & Tannen, PODS
     2007])."
   - Replace with: "Assumption-stamped derivations are made practical by **provenance semirings**: the
     semiring of polynomials N[X] over base-tuple identifiers is the most general provenance, and
     retracting a base tuple is setting its variable to the semiring's annihilator and re-evaluating
     ([Green, Karvounarakis & Tannen, PODS 2007]). Identifying that polynomial with an ATMS label — a
     monomial as an environment, the sum of monomials as the alternative minimal environments — is this
     program's synthesis bridging the database and truth-maintenance literatures, not a claim of the
     paper." Add the ATMS-identification to the §9 flag list.

2. **Extend the SL conservatism to the Beta mapping (§3.5, and the §8.2 open question).**
   - Current (§8.2): "The specific fusion operator, if any, needs validation against the 'Can we trust
     subjective logic' critique before it is relied upon."
   - Replace with: "The specific fusion operator, if any, needs validation against the 'Can we trust
     subjective logic' critique before it is relied upon — and so does the opinion↔evidence-count
     (Beta/Dirichlet) mapping the credence model derives opinions through, which that same critique also
     questions."

3. **Correct the qualifier one-level citation (§2.1).**
   - Current: "...keep qualifiers exactly one level deep, because a qualifier modifying a qualifier makes
     scope ambiguous ([arXiv:2304.03375])."
   - Replace the citation with Wikidata **Help:Qualifiers** (which states the rule verbatim); keep
     arXiv:2304.03375 for the separate "qualifiers of inferred statements combine" point.

4. **Correct OWL predicate capitalization (§2.1, §3.3, §9 flag).** Write `owl:DeprecatedClass` and
   `owl:DeprecatedProperty` (capital D), alongside `owl:backwardCompatibleWith` / `owl:priorVersion`; the
   §9 flag can move from "confirm exact names" to confirmed. LinkML's `deprecated` slot is confirmed.

5. **Tighten "severance is a fold filter" (§3.4 point 2, and §2.3).** Make "fold filter" name only the
   deterministic replay-time voiding; state that re-derivation from the separated stubs is a subsequent
   record-time maintenance pass (new model/embedder calls), not part of replay — so the sentence cannot
   be read as making model calls at replay. (Substance is fine; this guards the determinism fixed point.)

6. **Optional sharpening (§3.1).** ODKE+'s figure is 98.8% precision over **19 million** facts; "millions"
   is accurate but the concrete number is stronger.
