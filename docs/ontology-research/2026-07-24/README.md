# Ontology redesign research, 2026-07-24

A dated research snapshot, not as-built documentation and not a committed plan: the deliverables of the July 2026 design-research program on zuihitsu's ontology, preserved as fodder for a future redesign. The intent is to gather more live-instance data before any redesign is actually designed — a later program would produce its own sibling snapshot beside this one rather than editing this in place. The failure survey that grounds it lives at [`../../ontology-failures/2026-07-23.md`](../../ontology-failures/2026-07-23.md).

## Intervening work (as of 2026-07-26)

The codebase has moved since the snapshot was written; a reader should hold these against the text rather than trusting its "current system" descriptions verbatim.

- **The identity-resolution corollary partly shipped; the core identity proposal has not.** The canonical-identity arc (PR #107, merged minutes before the snapshot) and the later link-write canonicalisation give the running system exactly the mechanical layer the report and `lanes/identity-belief.md` present as unbuilt: speakers are stamped with the canonical class-primary handle, reads canonicalise far endpoints to the primary, content and link writes redirect onto it, and the canonicalize maintenance pass re-homes edges stranded on non-primary members. `00-grounding.md`'s "reads traverse the class, writes land on one stub" predates this. Crucially, all of it is layered on top of the union-find `class_id` — the graded, revocable, assumption-stamped identity view that is the redesign's keystone remains unbuilt, so the proposal stands.
- **The time-memory lane's predicted failure modes are now filed issues**, gathered under the `temporal` label: #112 (episodic session recaps, absorbing a deferred `memory.between_times` design note), #113 (temporal extraction stamps undated events with the assertion day — the live instance of the lane's occurred/asserted conflation), #114 (fabricated content recorded for an unfetched URL and attributed to the teller — a concrete instance of the faithfulness gap `lanes/welding.md` theorises), #115 (correcting a date requires a full-text supersede), and #116 (the agent's name is not on the log), alongside the re-labelled #74, #103, and #106.
- **Surfaced entries now carry temporal stamps** (`when <occurrence> · recorded <relative age>`), so the lane's claim that recorded age is never lifted onto the surface is softened — though the deeper observed-versus-ingested pair it proposes remains unbuilt.
- Everything else — the Statement keystone, the credence model, the memory typology, the iCalendar-style temporal split, the provenance and privacy lanes, and the external-system surveys — is unaffected by the intervening work.

Reading order:

- [`report.md`](report.md) is the primary deliverable — the proposed redesign of the fact model, relations, identity, belief, time, memory typology, privacy, provenance, and the neural-symbolic seam, with every load-bearing claim cited into the lanes.
- [`draft-issue.md`](draft-issue.md) is the condensed proposal shaped as a GitHub issue, unfiled pending the operator's review.
- [`lanes/`](lanes/) holds the seven research-lane reports the main report cites — the evidence appendix.
- [`verification/`](verification/) holds the adversarial-verification passes over the report's claims.
- [`00-grounding.md`](00-grounding.md) and [`01-synthesis-plan.md`](01-synthesis-plan.md) are the program's own working notes: the brief it ran under and the synthesis plan it followed.

The proposal targets a future agent's genesis; there is no migration constraint. Nothing here binds the codebase — the current seed ontology remains what `seed_relations()` says it is.
