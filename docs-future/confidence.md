# Confidence register

The design chapters are written in the normative present, which commits them to statements they cannot all equally support. This register records what each load-bearing claim rests on.

The voice in the chapters is a drafting discipline. This file is where the hedging lives, so the design can be read as a design and audited as a proposal.

## Status vocabulary

| | |
|---|---|
| **Verified** | checked against a fetched primary source by the adversarial verification pass |
| **Observed** | established by direct measurement of this system's own data |
| **Corroborated** | multiple independent sources agree, without a single primary check |
| **Single-source** | rests on one study or one system, unreplicated |
| **Synthesis** | our own inference from sources that do not themselves make the claim |
| **Open** | genuinely unresolved, with the alternatives named |

## The verification pass

Two adversarial passes ran over the research report before the design was taken up ([`research/2026-07-24/verification/`](research/2026-07-24/verification/)). Of its load-bearing cited claims: **29 confirmed** against fetched primary sources, **5 corrected**, **0 unsupported**, **1 unreachable**. Every future-dated citation was fetched directly rather than judged from memory, and all resolved to real sources supporting the claims as reported.

The five corrections are folded into the chapters. They were: the identification of provenance polynomials with justification labels relabelled as our own synthesis rather than a claim the source makes; the conservatism about subjective logic extended from its fusion operators to its evidence-count mapping; the severance description tightened so re-derivation is a record-time pass rather than part of the fold; the drift figures for the long-running accumulator made precise, being a monotonic decline in newly promoted claims rather than a cliff; and the causal story for forced-choice elicitation tempered to distinguish omission variance from content variance.

## By chapter

### Statements

| Claim | Status | Note |
|---|---|---|
| A reified statement object with qualifiers, references, and rank is the battle-tested shape | Verified | Wikidata's data model |
| Qualifiers must stay one level deep or scope becomes ambiguous | Verified | Stated directly in Wikidata's own guidance |
| Schema-guided extraction lowers rather than raises the model's error rate | Corroborated | Direction solid; the specific reduction percentages remain **unreachable** and are not relied on |
| Keeping structure and source utterance together is the convergent choice | Verified | Multiple production memory systems |
| **The referential frame** | **Observed, unvalidated** | The need is measured directly (39% of the live corpus, with layer mixing inside single entries). The three-value enum is our own proposal and has never been implemented or tested |
| **A Statement in an object slot** | **Observed, unvalidated** | The need is measured (35 of 198 entries). The quoted-not-asserted rule is carried over from the inter-agent boundary, where it is established; applying it to nesting is new |
| **A gloss belongs to an utterance, not a Statement** | **Observed** | Falsified the original design assumption. Follows from compound entries carrying up to eight claims, and from one observed case where mixed visibility forced a manual split |
| Counts and measures as a typed value with amount, unit, and bounds | Verified | Wikidata's quantity datatype, adopted wholesale. The value type was originally specified from first principles with no survey behind it, and the bounds were the gap that survey found |
| Instance-level counting is distinct from class-level cardinality | Verified | OWL 2 allows asserting a cardinality class of an individual, but has no unique-name assumption and no temporal scoping, so the idea carries over and the mechanism does not |
| The frame is a simplification of Cyc's microtheories | Verified | Context-relative truth, with fictional content in its own context, is the established solution. The frame is closed, three-valued, and non-nestable where microtheories are none of those |
| Collective against distributive plurals is declined, not unknown | Verified | Conceptual graphs mark the distinction natively. This is a cost decision, and the design should not imply otherwise |
| **The witness set on an utterance** | **Synthesis** | Follows from multi-party channels being the ordinary case for a social agent. Nothing surveyed carries it, and the two mechanisms that need it, audience evaluation and dependence detection, each stated the requirement before the datum existed to satisfy it |

### Events and roles

| Claim | Status | Note |
|---|---|---|
| One event with role-edges is the principled fix for per-subject copies | Verified | Neo-Davidsonian event semantics; the W3C n-ary patterns describe the same two failures |
| Role inventories past the first two positions are inconsistent even among experts | Verified | Documented for the standard numbered inventories |
| The small closed role set is sufficient | Observed | Every multi-participant happening in the corpus was expressible with agent, theme, source, and time |
| Event-to-event relations are needed | Observed | Causation and consequence had no home and fell back to prose |
| Multiple fillers per role, with counts reserved for unindividuated participants | Synthesis | Follows from each role-edge being its own Statement with its own audience. The corpus study tested the sufficiency of the role *inventory*, never filler multiplicity, so the set-wise resolution test is unmeasured |

### Relations

| Claim | Status | Note |
|---|---|---|
| Deprecate-and-alias is the right lifecycle | Verified, corroborated | Standard deprecation vocabulary exists in OWL and LinkML; two independent production systems reached the same fix from the same problem |
| Declared domain and range catch reversed edges | Observed | The live link graph contains reversed and cross-typed edges that a declared range rejects |
| A free-text side channel keeps a closed vocabulary honest | Single-source | One production system, which found 78% of its edges were one-off free-text before normalising |
| Architecture claims about role-typed hypergraph stores | Corroborated | Vendor-authored in part; only the architecture is relied on, never the performance claims |

### Identity

| Claim | Status | Note |
|---|---|---|
| Hard equivalence is too strong for how the link is used | Verified | Established in the linked-data literature |
| Transitive closure manufactures false identities at scale | Verified | 177,000 distinct entities collapsed in the measured case |
| Attribute overlap is unsound because knowledge can be recited | Verified | A record-linkage independence violation |
| Relational structure is expensive to forge | Verified | Collective entity resolution; the patient-attacker caveat stands and is stated in the chapter |
| The substrate wall fixes the behaviour leak | Synthesis | The architectural-metadata principle is established; that it fixes our specific 0.30 relay failure is our inference |
| Severance re-derivation is cheap in practice | **Open** | Bounded by derivations touched rather than log size, but the cost claim is unmeasured |
| Crumble and accretion thresholds | **Open** | Empirical tuning, needing data. Genuine same-person profiles also diverge |
| Assumption-stamp representation | **Open** | Per-event set against a shared environment table: a storage-versus-fold-time trade, unresolved |

### Belief

| Claim | Status | Note |
|---|---|---|
| Verbalised model confidence is overconfident, saturated, and protocol-sensitive | Verified | Fetched and confirmed |
| A representation separating belief strength from evidence quantity is wanted | Verified | Subjective logic |
| Trust discounting is sound | Verified | |
| **Fusion operators** | **Open, deliberately unused** | Named critics attack both the operators and the mapping to evidence counts. The chapter relies only on dependence *detection* plus a no-gain rule, which is trivially sound |
| Non-prioritised revision is the right default | Verified | Credibility-limited belief revision |
| The exact credence shape | **Open** | The lanes disagreed. Three tested shapes exist; a lighter one is a documented fallback |
| Dependence detection is the common case in a shared channel | Synthesis | Sociality makes corroboration real and dependence ordinary in the same stroke. This moves the load onto the detection rule, which is now the load-bearing part of the credence design rather than a soundness caveat on it |

### Time

| Claim | Status | Note |
|---|---|---|
| Validity intervals on facts, superseded by window-closing | Verified, corroborated | Convergent across temporal databases and every surveyed production graph |
| The three-way occurrence, task, trigger split | Verified as a solution shape | iCalendar is mature prior art. **But no surveyed peer has this failure**, so the problem may be specific to us; the fix is sound regardless and should not be oversold |
| The maximal tractable subclass of interval relations | Verified | A settled result. The open question narrowed to whether a simpler subalgebra already suffices |
| Anchor-aware durations and safe recurrence constructors | Verified | Established in modern date-time library design |
| Typed quantities | Observed | Found in prose in the corpus, in the same position dates once occupied |

### The memory typology

| Claim | Status | Note |
|---|---|---|
| The four-way typology | Verified, corroborated | Convergent from the cognitive architecture tradition through to current agent memory work |
| Episodic memory is automatic and architectural where semantic is deliberate | Verified | |
| **Episodic as linked companion rather than fallback tier** | **Single-source** | Rests entirely on the dual-trace study below |
| Procedural memory indexed by description embedding, decayed by invocation | Verified | An established agent design |
| Access-frequency and recency ranking is embedder-independent | Verified | The narrow claim is safe |
| That human recall activation transfers to agent salience | **Synthesis** | By analogy, not proof. Only the narrow claim above is relied on |
| Directives are a category error inside the fact model | Observed | 22 of 198 entries, one repeated verbatim ten times |
| The self belongs in a slot rather than in a memory | Synthesis | Follows from the charter needing to be unreachable by the machinery that decides what to omit. The current system's immutable charter entries protect the wording and not the slot, which is an argument from mechanism rather than a measured failure |
| An agent that cannot edit its own charter avoids unbounded drift | Synthesis | The self-reinforcing-loop argument is the same one used for credence and merges. The cost, that growth in self-conception needs operator attention, is stated and unquantified |
| Scratchpad storage | **Open** | Log-with-compaction against a side table: replay purity against log size. The lane's own weakest recommendation |

### Privacy and provenance

| Claim | Status | Note |
|---|---|---|
| Four independent literatures converged on the addressable context-scoped assertion | Verified | |
| Transmission principle as the governing condition | Verified | Contextual integrity |
| Zero residue is formally non-interference | **Synthesis** | Standard security theory, but the framing lacks one canonical citation to attach |
| Retraction against crypto-shredded erasure | Verified | Established event-sourcing practice |
| A redaction can be proven to have occurred without revealing content | **Synthesis** | Our combination of shredding with append-only tombstones |
| The retraction-authority lattice | Synthesis | The prior art leaves the subject-differs-from-author case explicitly open; the lattice is our answer |
| Combinatorial audience contexts are a genuine gap | Verified as a gap | No surveyed work handles a dynamic context population well. Predicates over the present set are the only tractable form found |
| Marking a derivation as owing recomputation when its premises gain support | Corroborated | Current memory systems mark a synthesis stale when unprocessed evidence bears on it. Ours rides machinery the derivation record already carries; the cost of maintaining the work list is unmeasured |
| Principles are universally quantified over the present set and fail closed on any member | Synthesis | The set-shaped evaluator was implied by audiences-as-predicates from the start; the per-principle consequences, and `in_confidence` becoming relative to the witness set, are our own working out |

### The seam

| Claim | Status | Note |
|---|---|---|
| Essentially no LLM-to-graph system verifies its neural writes against the source | Verified | The strongest single finding in the welding lane |
| Ontology constraints, not better extraction, suppressed drift in the long-running case | Verified | |
| Autoregressive models cannot reliably self-verify | Verified | The specific benchmark figure is search-sourced and not relied on |
| Prompt brittleness is large, persistent, and does not transfer across models | Verified | Measured spreads up to 76 points from meaning-preserving format changes |
| Forced-choice elicitation removes omission variance | Verified, tempered | It relocates variance into field content and introduces junk fill. Both are stated in the chapter |
| The constraint tax | **Open** | Schema forcing can degrade tool use or reasoning in some models. Must be measured per behaviour on the target model, never assumed |
| Record-at-call-time neural activities preserve determinism | Verified | Independently validated by durable-execution practice |
| Eager against lazy structuring | **Open** | How much deduplication judgement moves inline against deferring to a maintenance pass |

### The two traces

| Claim | Status | Note |
|---|---|---|
| Narrative alongside structure gains 20 points overall, concentrated in temporal, aggregation, and update tasks, with a clean null on single-occasion lookup | **Single-source** | One study, one benchmark, an automated judge, 20 questions per category with wide intervals |
| The gain is encoding-side rather than retrieval-side | **Unknown** | The study could not separate them and says so. This decides whether the design costs a model call per occasion or nearly nothing, and is the first thing [`evolution.md`](evolution.md) resolves |
| Depth beats breadth | Single-source | Their development path: a few points from coverage against twenty from depth |
| Cost neutrality | **Does not transfer** | An artifact of their prompt volume dominating. For an event-sourced store it is a record-time model call and permanent log volume |
| Some content survives only as narrative | Observed | Independently found in the corpus study, which strengthens this beyond the single source |

### The query surface

| Claim | Status | Note |
|---|---|---|
| Fusing rank orders rather than scores | Corroborated | The convergent form in production retrieval, where parallel lanes are merged on rank position and reranked at the head. Adopted for its embedder-independence, never for a reported gain, and no surveyed figure is relied on |
| Structural questions are answerable without a model call | Synthesis | Each named question is a graph traversal given the substrate; that the set is *sufficient* for what the agent asks is untested against real turns |

## Unresolved, gathered

The questions that need evidence rather than more design:

1. **Encoding against retrieval** for the second trace. Decides the cost of the whole episodic layer.
2. **The constraint tax** on the target model, per behaviour.
3. **The credence shape**, with fusion operators deliberately unused until validated.
4. **Crumble and accretion thresholds** for tentative merges.
5. **Assumption-stamp representation**: per-event set against shared environment table.
6. **Scratchpad storage**: log-with-compaction against a side table.
7. **Eager against lazy structuring**, against the constraint tax. This is also the question of whether a model call belongs on the write path at all. Structuring inside the transaction buys the same-turn correction loop and pays for it with latency and a thrashing risk on the rejection-retry cycle. The mitigations are stated in [`write-surface.md`](write-surface.md); whether they suffice is unmeasured, and the fallback is to move structuring to end-of-turn or to a pass.
8. **Enumeration representation**: many Statements against one opaque list. The corpus found both defensible and the design states no preference.
9. **Severance re-derivation cost**, claimed cheap and unmeasured.
10. **TypeDB's cardinality annotations**, whose documentation was unreachable during the counting survey and which remain unverified.
11. **Whether the frame's three values are the right three.** Proposed from one corpus. A second instance with a different social world might need a fourth or find one redundant.
12. **Whether the witness set is knowable.** A channel's membership is readable; who actually saw a message is not. The chapter resolves this by asymmetry, building the set from demonstrated participation and falling back to the teller alone, so it may narrow freely and widens only on evidence. What counts as demonstrated participation is the same judgement [`evolution.md`](evolution.md) stage 0b owes for the present set, and it may be platform-specific, in which case it belongs to the connector contract.

## Things deliberately not claimed

- That the design closes the eleven surveyed failures. It closes six structurally and answers five in design without validation. See [`coverage.md`](coverage.md).
- That any benchmark number here is a target. The one composite memory benchmark surveyed was found by independent audit to have a materially wrong answer key and a judge that accepted most intentionally wrong answers. Component success is measured by structural oracles against our own log, not by a self-reported score.
- That autonomy means no human attention. It means exception-triggered attention with a queue that shrinks per fact as the store grows.
- That the neural writer is verified for **truth**. The critics check well-formedness, typing, and consistency. A well-typed falsehood still passes at write time, and runtime faithfulness checking remains unsolved here as it is everywhere else.
