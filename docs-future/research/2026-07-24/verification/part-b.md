# Adversarial verification — Part B (sections 2.4, 2.5, 2.6 and uses in 3.6–3.7, 4, 5, 6)

Verifier lane. Every load-bearing cited claim in the assigned half was checked against its
primary source (or the closest reachable primary/secondary corroboration). Verdicts:
CONFIRMED / CORRECTED / UNSUPPORTED / UNREACHABLE.

## Verdict table

| # | Report claim (section) | Verdict | Evidence / correction |
|---|---|---|---|
| 1 | FormatSpread: meaning-preserving format changes cause accuracy spreads of "dozens of points", unfixed by scale/few-shot/instruction-tuning; rankings don't transfer across models (2.5, 5) | **CONFIRMED** (report is conservative) | arXiv:2310.11324 abstract: "performance differences of **up to 76 accuracy points** when evaluated using LLaMA-2-13B"; sensitivity "remains even when increasing model size, the number of few-shot examples, or performing instruction tuning"; "format performance only weakly correlates between models." Report's "dozens of points" understates the max (76) but is safely true. Earlier drafts' "up to 76" was the accurate figure. |
| 2 | LLM-modulo: auto-regressive LLMs cannot reliably self-verify, so hard properties need a sound symbolic verifier (2.5, 4.2) | **CONFIRMED** | arXiv:2402.01817 abstract: "auto-regressive LLMs cannot, by themselves, do planning or self-verification (which is after all a form of reasoning)." |
| 3 | LLM-modulo Blocks-World 82% figure (flagged §9) | **CONFIRMED** | Search-corroborated from the paper/UnfoldAI: "LLM performance in Blocks World improves to **82% within 15 back prompting rounds**" with VAL as external verifier (Logistics → 70%). |
| 4 | Constraint Tax (arXiv:2606.25605, future-dated): structured-output constraints can suppress tool-calling / degrade reasoning in some models (2.5, 4.2, 8.1) | **CONFIRMED** | Paper is real, dated 25 Jun 2026. Title: "Constraint Tax in Open-Weight LLMs: An Empirical Study of Tool Calling Suppression Under Structured Output Constraints" (Li, Zhang, Lv). Documents exactly the claimed effect on open-weight models. |
| 5 | ATOM: Graphiti's incremental LLM-based entity/relation resolution degrades with graph expansion (2.5, 5) | **CONFIRMED (verbatim)** | arXiv:2510.22590v1 full text: "ATOM shows an improvement over Graphiti, whose incremental, LLM-based entity and relation resolution **degrades with graph expansion (increasing context size)**." Empirically Graphiti < ATOM on entity F1 (0.959 vs 0.994) and relation F1 (0.902 vs 1.0). *Nuance:* the cause is context-window growth (LLM is prompted with all prior entities), i.e. a scaling/context-length problem — not semantic drift. The 2.5 gloss equating it to "the same shape as NELL's drift" is a loose analogy; §5's "drift-with-size" label is fair. |
| 6 | Two-graph drift monitoring (arXiv:2509.03857, future-dated): deterministic baseline graph + LLM graph as noisy sensors, anomaly score with dynamic threshold (4.5, 5) | **CONFIRMED** | Real paper, 4 Sep 2025, "Continuous Monitoring of Large-Scale Generative AI via Deterministic Knowledge Graph Structures." Verbatim: "we treat the two graphs as *noisy sensors* rather than 'ground truth' and emphasize relative change over absolute correctness." Anomaly score `A(Gt)=Σ w·|M(G_LLM)−M(G_base)|`, dynamic threshold `α_t = μ + λσ`. |
| 7 | LoCoMo audit (bloo-mind): 6.4% answer-key error, 63% wrong-answer acceptance (2.6, 5) | **CONFIRMED** | essays.bloo-mind.ai (citing Penfield Labs): "99 score-corrupting errors in 1,540 questions — a **6.4% rate**"; judge "accepted **62.81%** of intentionally wrong-but-topical answers"; "**56%** of adjacent-pair comparisons … statistically indistinguishable." Report's "63%" ≈ 62.81%. Attribution note: audit is by Penfield Labs, hosted/summarised in the bloo-mind essay; report calls it "one commentator's audit" — fair. |
| 8 | Voyager: skill library of executable Luau/code indexed by a description embedding (3.7, 2.4) | **CONFIRMED (verbatim)** | ar5iv 2305.16291: "Each program is indexed by the embedding of its description, which can be retrieved in similar situations in the future"; key = embedding of program description, value = the program; top-5 retrieval by query embedding. |
| 9 | CoALA: four-kind memory typology (working/episodic/semantic/procedural), procedural = code/LLM weights (2.4, 3.7) | **CONFIRMED** | ar5iv 2309.02427 §4.1: working, episodic, semantic, procedural; procedural = "*implicit* knowledge stored in the LLM weights, and *explicit* knowledge written in the agent's code." |
| 10 | Soar episodic memory is automatic/architectural, cue-based retrieval (2.4, 3.7) | **CONFIRMED** | Soar manual: "Episodic memory records new episodes without deliberate action/consideration by the agent"; default recording at end of each decision cycle; cue-based "nearest-neighbor search." |
| 11 | ACT-R base-level activation `B_i = ln(Σ t_j^-d)`, recency+frequency, embedder-invariant framing (2.4, 3.7, 4.5) | **CONFIRMED** | Equation confirmed across ACT-R literature: `B_i = ln(Σ_k t_k^-d)`, t = time since k-th retrieval, d = decay. Community default d = 0.5 (empirical estimates range ~0.3–0.7; the Petrov PDF itself would not extract, but the formula is canonical). The report body only claims "power-law"; the embedder-invariance inference (function of timestamps only) is sound and correctly flagged as human→agent-transfer-by-analogy. |
| 12 | RFC 5545: VALARM attaches to VEVENT **and** VTODO; TRIGGER relative to start/end/due; ACTION carried; no VALARM ⇒ never fires (2.4, 3.6) | **CONFIRMED** | icalendar.org/RFC-5545: "A VALARM … MUST only appear within either a VEVENT or VTODO"; TRIGGER "relative to the START or END of the event or to-do" (and DUE for VTODO); "MUST include the ACTION and TRIGGER"; an event with no VALARM has no alarm. Report's three-component split is accurate. |
| 13 | Temporal determinism model: workflow code deterministic; non-deterministic ops (incl. LLM/AI invocations) in Activities, recorded once, reused on replay (2.5, 4.4) | **CONFIRMED (verbatim)** | docs.temporal.io/workflow-definition: "Workflow code must be deterministic to support replay. To handle non-deterministic operations like **API calls, LLM/AI invocations, database queries** … put them in Activities." Activities execute outside the replay path; results stored in event history and reused. |
| 14 | bound edge-graph-normalization RFC: 78% of edges were one-off free-text relations; closed 10-relation enum + free-text `context` column + startup migration (2.6, 3.3) | **CONFIRMED** | Raw RFC: "Approximately **78% of non-deleted edges use relations that appear exactly once** in the corpus." 10-relation enum matches exactly; `context TEXT` (nullable) added; startup normalization rewrites bespoke relations to `related_to`, preserving the original in `context`. |
| 15 | Allen interval algebra: full 13-relation reasoning NP-hard; adopt a tractable subset (2.4, 3.6; flagged open in 8.2/9) | **CONFIRMED — flag now resolvable** | Full Allen satisfiability is **NP-complete**; **ORD-Horn (Nebel & Bürckert, JACM 1995)** is a *maximal* tractable subclass, polynomial-time, with path-consistency sufficient for satisfiability. Primary sources exist (JACM 42(1) 200848; JACM 50(5) full classification; Drakengren & Jonsson's further maximal subclasses). The 8.2 open question "which subset (pointisable, ORD-Horn)" can now cite ORD-Horn as the maximal-tractable answer. |
| 16 | NELL: coupled semi-supervised learning under ontology constraints (mutual exclusion, type-checking on relation args, multi-view agreement) is the drift brake; symbolic schema disposes, extractors propose (2.5, 4.2, 4.3, 4.5) | **CONFIRMED** | Primary (CPL/MBL papers, Carlson et al.): CPL "leverages mutual-exclusion and type-checking constraints"; each relation "has an ordered pair of argument types" (domain/range); MBL "couples the training of multiple extraction techniques using a multi-view constraint that requires them to agree," promoting only instances recommended by both while obeying mutual-exclusion/type-checking. CACM 2018 fulltext returned HTTP 403 but the mechanism is corroborated from the AAAI/CPL/MBL primary sources. |
| 17 | NELL: precision declined to ~57% after 66 unattended iterations (2.5, flagged §9) | **CORRECTED (nuance)** | The 57% is the estimated precision of beliefs **promoted during iterations 45–66**, not the cumulative KB precision "after 66 iterations." NELL AAAI-2010: precision ~90% (iters 1–22), ~71% (23–44), ~57% (45–66); overall ~242,453 promoted beliefs at estimated ~74% after 66 iterations. So "precision of newly promoted beliefs declined to ~57% by the last third" is the accurate statement; "precision decline to ~57% after 66 iterations" reads as a cumulative figure and slightly overstates the decay. |
| 18 | NELL: ~5 minutes of human supervision per relation every 10 iterations (2.5, flagged §9) | **CONFIRMED (minor conflation)** | Two distinct real cadences, both documented: (a) periodic review "every few weeks … about 5 minutes … scanning each category and relation"; (b) "10–15 minutes … approving RL-generated rules every 10 iterations." The report fuses (a)'s per-relation 5-minute figure with (b)'s every-10-iterations cadence. Both figures are genuine; the fusion is a minor imprecision, not a fabrication. |

**Counts:** CONFIRMED 15 · CORRECTED 2 (both NELL-cadence nuances; #5 and #11 carry sub-nuances) · UNSUPPORTED 0 · UNREACHABLE 0. Two primary PDFs/pages (NELL CACM 2018 → 403; Petrov ACT-R PDF, NELL AAAI-15 PDF → binary-only) could not be read directly, but each claim was corroborated from other primary sources.

---

## Synthesis-level judgement (a): would forced-choice structured elicitation have prevented the 6%→75% capture swing?

**Partly sound, but the report overstates it.** The causal core is correct: the 6%→75%
swing was over *whether the capture action happened at all* (a scaffold sentence the model
could silently ignore). Making capture a **required field of a tool schema the turn cannot
complete without filling** does eliminate the *omission* variance — the model can no longer
just forget. Forced tool-choice/required-field elicitation reliably removes "did it do it
at all" brittleness; that part is well-supported and consistent with FormatSpread (which is
about accuracy variance, and here the presence/absence axis is what's being pinned).

The overstatement is the word **"disappears."** Required-field elicitation *relocates* the
variance rather than dissolving it, and introduces a new failure mode the report underweights:

- **Junk-fill.** A required field must be filled *with something*. The model can satisfy the
  schema with a hallucinated subject, a degenerate default, or a low-confidence guess it
  would previously (correctly) have declined to record. Omission-variance becomes
  content-correctness-variance and a false-positive capture rate.
- **Field *values* are still autoregressive.** FormatSpread's brittleness applies to the
  *content* the model emits, not just to whether it emits. A schema pins the slot, not the
  slot's value — so paraphrase sensitivity survives inside the field.
- **The Constraint Tax cuts the other way.** The report's own §4.2 cost (2606.25605) says
  schema-forcing can degrade reasoning/tool-calling — i.e. the very act of forcing the field
  can lower the quality of what lands in it, even as it raises the fill rate.

Consequence for the eval design (§5): the paraphrase-spread probe as written frames
*near-zero spread* as success, but a required field trivially has near-zero *presence*
spread while its *values* still swing. The probe must measure spread of **field content and
capture correctness**, not merely capture presence, or it will give false comfort.
Recommended: soften "the swing disappears" to a claim about the *omission* axis, and state
the residual content/junk-fill axis explicitly.

## Synthesis-level judgement (b): do canary facts + re-derivation audits detect the drift classes NELL exhibited?

**A genuine improvement, but with structural blind spots — the §4.5 framing is too strong.**
NELL's signature drift was *gradual, monotonic, category-specific* precision decline in
*newly promoted* beliefs (90→71→57 across iteration bands), with some relations staying >90%
while others cratered <50%. Against that specific shape:

- **Coverage-limited.** Canaries detect drift *where a canary was seeded*. NELL's drift was
  category-specific and concentrated in unanticipated relations — exactly the regions a fixed
  seed set is least likely to cover. Canaries test known-knowns; the dangerous drift is in the
  unseeded tail.
- **Weak against slow monotonic drift.** Re-derivation audits assert *stability* (no
  oscillation, no merge/un-merge, no contradiction accumulation). But a steadily worsening,
  non-oscillating drift passes a stability check — and NELL's decline was precisely monotonic,
  not oscillatory. Stability ≠ correctness-vs-ground-truth.
- **Seeded-fact retention ≠ new-write precision.** Canaries probe whether *specific old facts*
  survive/reject/leak. NELL's failure was in the *precision of the flow of new writes*. The
  §5 faithfulness oracle (every write entailed by an utterance) covers *same-run* writes but
  not *longitudinal precision decay* of new writes in uncovered categories.
- **Blind to internally-consistent poisoning.** If drift produces coherent-but-wrong structure
  (the recitation-attack shape, a self-consistent false merge cluster), re-derivation is
  *stable* — it re-derives the same wrong thing — and no canary flips unless it happens to
  touch the poisoned region. Consistency is not correctness.

The stronger defences in the report are actually the **two-graph divergence monitor**
(2509.03857) and **NELL-style coupling / agreement-before-promotion** (4.3) — both compare
independent signals rather than re-probing a fixed sample. Canaries + audits should be framed
as *necessary but not sufficient*, and the claim that they "detect the drift classes NELL
exhibited" should be qualified: they detect *seeded* and *oscillatory* drift well, and
*unseeded, monotonic, new-write* drift poorly — which is the very shape NELL suffered.

---

## Recommended report edits

1. **§4.2 (and the echo in §4.5/§5): soften "disappears."**
   - Current: *"The 6%-to-75% capture swing disappears because capture is no longer a function of wording; it is a field that must be filled."*
   - Replace: *"The 6%-to-75% swing in *whether* capture happens collapses, because capture is no longer an optional scaffold step but a required field the turn cannot complete without filling. The variance does not vanish, though — it moves from omission to content: a required field can be filled with a wrong or low-confidence value, so the paraphrase-spread eval (§5) must measure the spread of captured *content and correctness*, not merely capture presence, and the Constraint Tax (§4.2) means the forcing itself can degrade what lands in the field."*

2. **§5 paraphrase-spread probe: make the success criterion content-level.**
   - Current: *"A behaviour correctly moved from prompt to structure shows near-zero spread; a wide spread flags that it is still prompt-borne."*
   - Add: *"Spread must be measured over the captured field *values and their correctness*, not over mere presence — a required field has near-zero presence spread by construction while its content can still swing."*

3. **§2.5: correct the NELL 57% framing.**
   - Current: *"a precision decline to ~57% after 66 unattended iterations."*
   - Replace: *"a precision decline in newly promoted beliefs to ~57% by the last of its first 66 iterations (≈90% in iterations 1–22, ≈71% in 23–44, ≈57% in 45–66; the cumulative KB sat near ~74%)."*

4. **§4.5: qualify the canary/audit drift-coverage claim.**
   - Add after the canary/audit bullets: *"Canaries and stability audits detect *seeded* and *oscillatory* drift well but *unseeded, monotonic, new-write* drift poorly — which is the shape NELL actually suffered (gradual, category-specific precision decline in the unanticipated tail). They are necessary but not sufficient; the two-graph divergence monitor and agreement-before-promotion coupling are the load-bearing longitudinal defences, with canaries as a cheap alarm on the regions you thought to cover."*

5. **§2.5 / §5: note ATOM's degradation is context-length, not drift.**
   - Optional: where Graphiti's degradation is likened to "the same shape as NELL's drift," add a half-sentence: *"(ATOM attributes this to growing context size at resolution time, a scaling limit rather than semantic drift per se)."*

6. **§8.2 / §9: resolve the Allen tractable-subclass open item.**
   - The boundary is no longer open: **ORD-Horn (Nebel & Bürckert, JACM 1995)** is the maximal tractable subclass of the full (NP-complete) algebra, decidable in polynomial time by path-consistency. Cite it directly rather than leaving "which subset" unresolved.

7. **§2.5 / §9: NELL human-touch cadence — minor.** Optionally split the fused figure into its two real cadences (≈5 min/relation on a several-week review; 10–15 min RL-rule approval every 10 iterations) rather than "5 minutes per relation every 10 iterations."
