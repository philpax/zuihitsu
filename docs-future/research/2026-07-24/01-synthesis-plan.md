# Synthesis plan (working document)

## Deliverables
1. `report.md` — cited research report, adversarially verified before finalisation.
2. `draft-issue.md` — draft GitHub issue: redesign shape, staged milestones, welding architecture, cross-check table (every failure-survey entry → how addressed). Draft only; operator reviews.

## Cross-check table rows (from docs/ontology-failures/2026-07-23.md — every entry must appear)
1. Facts are sentences
2. One event, one subject, many copies
3. Relations are bare edges
4. Schedule and description conflate in the temporal model
5. Identity is binary and entangled with storage
6. Relation schemas are immutable and vocabulary drifts (#42)
7. Identity complexity leaks into behaviour (#104)
8. Belief has no credence model
9. Hygiene thresholds are embedder geometry
10. Load-bearing behaviour is prompt-sensitive
11. The neural writer is unverified

## Issues the proposal must speak to
#7 (survey — partially discharged), #15 (self governance), #20 (autonomy/zero-admin), #42 (relation schema evolution), #44 (bulk ingestion probe), #58 (procedural), #59 (workspace), #74 (episodic), #94 (identity/reversibility — required reading), #100 (neural judgement in symbolic transactions), #103 (typed time interface), #104 (identity behaviour leak).

## Fixed points checklist (proposal must preserve)
- Append-only log, deterministic replay, record-at-call-time.
- Privacy ≥ current: postures, hidden endorsements, zero residue.
- Teachable errors; handle-shaped simple agent surface.
- Unbounded scale; #44 ingestion handled natively.

## Autonomy framing requirements
- Tiered reversibility generalised to every derived conclusion.
- Name the genuinely irreversible decisions.
- Name exception classes reaching the operator + expected rates.

## Report outline (provisional)
1. Problem statement: the welding question; the 6%→75% datum.
2. Survey findings per lane (fact shape; provenance/privacy; identity/belief; time/memory; welding; convergent evolution).
3. The proposed ontology: fact model, relations, identity, time, belief, memory kinds, privacy, provenance, forgetting, schema evolution, query surface.
4. The welding architecture.
5. Evaluation design.
6. Autonomy and reversibility.
7. Adversarial-verification record.

## Verification pass plan
- After report draft: 2 Opus verifier lanes, split by half of the report, checking every cited claim (fetch the source or find corroboration; flag overclaims, misattributions, dead citations). Corrections folded before final.
