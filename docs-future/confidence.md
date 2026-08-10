# Confidence register

The design chapters are written in the normative present, which commits them to statements they cannot all equally support. This register records what each load-bearing claim rests on.

The voice in the chapters is a drafting discipline. This file records the hedging, so the design can be read as a design and audited as a proposal.

## Status vocabulary

| | |
|---|---|
| Verified | checked against a fetched primary source by the adversarial verification pass |
| Observed | established by direct measurement of this system's own data |
| Corroborated | multiple independent sources agree, without a single primary check |
| Single-source | rests on one study or one system, unreplicated |
| Synthesis | an inference by this design from sources that do not themselves make the claim |
| Decided | a design decision, with a measurement that established the need or bounded the cost. The measurement is evidence for the premise, never for the decision |
| Open | genuinely unresolved, with the alternatives named |

## The verification pass

Two adversarial passes ran over the research report before the design was taken up ([`research/2026-07-24/verification/`](research/2026-07-24/verification/)). Of its load-bearing cited claims: 29 confirmed against fetched primary sources, 5 corrected, 0 unsupported, 1 unreachable. Every future-dated citation was fetched directly rather than judged from memory, and all resolved to real sources supporting the claims as reported.

The five corrections are folded into the chapters. They were: the identification of provenance polynomials with justification labels relabelled as this design's own synthesis rather than a claim the source makes; the conservatism about subjective logic extended from its fusion operators to its evidence-count mapping; the severance description tightened so re-derivation is a record-time pass rather than part of the fold; the drift figures for the long-running accumulator made precise, being a monotonic decline in newly promoted claims rather than a cliff; and the causal story for forced-choice elicitation tempered to distinguish omission variance from content variance.

## By chapter

### Statements

| Claim | Status | Note |
|---|---|---|
| A reified statement object with qualifiers, references, and rank is the battle-tested shape | Verified | Wikidata's data model |
| Qualifiers must stay one level deep or scope becomes ambiguous | Verified | Stated directly in Wikidata's own guidance |
| Schema-guided extraction lowers rather than raises the model's error rate | Corroborated | Direction solid; the specific reduction percentages remain unreachable and are not relied on |
| Keeping structure and source utterance together is the convergent choice | Verified | Multiple production memory systems |
| The referential frame | Observed, unvalidated | The need is measured directly (39% of the live corpus, with layer mixing inside single entries). The enum is this design's own proposal and has never been implemented or tested. Its coverage is narrower than the incidence figure suggests: referent misattribution is a wrong subject rather than a wrong layer, and it is answered by the redirect below rather than by any layer value |
| A Statement in an object slot | Observed, unvalidated | The need is measured (35 of 198 entries). The quoted-not-asserted rule is carried over from the inter-agent boundary, where it is established; applying it to nesting is new |
| A gloss belongs to an utterance, not a Statement | Observed | Falsified the original design assumption. Follows from compound entries carrying up to eight claims, and from one observed case where mixed visibility forced a manual split |
| Counts and measures as a typed value with amount, unit, and bounds | Verified | Wikidata's quantity datatype, adopted wholesale. The value type was originally specified from first principles with no survey behind it, and the bounds were the gap that survey found |
| Instance-level counting is distinct from class-level cardinality | Verified | OWL 2 allows asserting a cardinality class of an individual, but has no unique-name assumption and no temporal scoping, so the idea carries over and the mechanism does not |
| The frame is a simplification of Cyc's microtheories | Verified | Context-relative truth, with fictional content in its own context, is the established solution. The frame is closed, three-valued, and non-nestable where microtheories are none of those |
| Collective against distributive plurals is declined, not unknown | Verified | Conceptual graphs mark the distinction natively. This is a cost decision, and the design should not imply otherwise |
| Assertedness belongs in the equality key | Synthesis | Without it a quotation and a later flat assertion of the same proposition collapse into one object, which either launders a quotation into a belief or swallows a real assertion. Found by adversarial review, not by the corpus study |
| The `principal` redirect, resolved against a seeded `presents` edge | Synthesis | Answers the referent misattribution the frame cannot reach, without a fourth layer, which would leave the wrong subject on the triple, or a second subject coordinate, which is the scope ambiguity the qualifier discipline forbids. Declared and never inferred, because a misfire files a claim about a bot onto a human; revocable through the existing severance stamp. Untested, and stage 1 is where it is falsified |
| A gloss or turn reference in the object slot | Observed | Four of the live instance's 198 content entries are metalinguistic, one of them carrying three separate claims about specific phrases: grading a line, conceding one and disputing another, characterising how someone was described. Two percent is small and the shape is unrepresentable without this, and the alternatives are an unqueryable literal or a handle minted for a passing phrase |
| The one-level nesting bound covers most of the attitude class | Observed, partial | A fraction of the observed attitudes are depth two, an attitude toward a position rather than toward a proposition, and flattening loses that the disagreement is with a stance somebody holds |
| Dispositions and generics are declined | Synthesis, newly admitted | Named as unrepresentable in three chapters before it was ever registered. A habitual modality is the obvious extension and is not taken |
| Third-party deontics flatten | Observed | Present in the live log. They are Statements rather than configuration, and the deontic force ends up in the relation name |
| A closure records why it happened | Synthesis | Supersession recorded that a window ended and never what ended it, so a corrected claim and one that stopped holding read identically. The closed set reuses the decisions the staleness ladder already enumerates, with an explicit unknown so declining is recorded |
| Enumerations are many Statements, not an opaque list | Synthesis | Settled by implication from the counting rule already in the chapter: a list of named entities is individuated in the act of listing, and the opaque form is reserved for the unindividuated case, where it is the same object as a count over a kind |
| A derived Statement's principle is the intersection of its premises' | Synthesis | Previously stated only for off-turn passes, leaving on-turn derivation, which is the commoner case, with no audience rule |
| The witness sets on an utterance | Synthesis | Follows from multi-party channels being the ordinary case for a social agent. Nothing surveyed carries it, and the two mechanisms that need it, audience evaluation and dependence detection, each stated the requirement before the datum existed to satisfy it |

### Events and roles

| Claim | Status | Note |
|---|---|---|
| One event with role-edges is the principled fix for per-subject copies | Verified | Neo-Davidsonian event semantics; the W3C n-ary patterns describe the same two failures |
| Role inventories past the first two positions are inconsistent even among experts | Verified | Documented for the standard numbered inventories |
| The small closed role set is sufficient | Observed | Every multi-participant happening in the corpus was expressible with agent, theme, source, and time |
| Event-to-event relations are needed | Observed | Causation and consequence had no home and fell back to prose |
| Multiple fillers per role, with counts reserved for unindividuated participants | Synthesis | Follows from each role-edge being its own Statement with its own audience. The corpus study tested the sufficiency of the role inventory, never filler multiplicity, so the set-wise resolution test is unmeasured |

### Relations

| Claim | Status | Note |
|---|---|---|
| Deprecate-and-alias is the right lifecycle | Verified, corroborated | Standard deprecation vocabulary exists in OWL and LinkML; two independent production systems reached the same fix from the same problem |
| Declared domain and range catch reversed edges | Observed | The live link graph contains reversed and cross-typed edges that a declared range rejects |
| A free-text side channel keeps a closed vocabulary honest | Single-source | One production system, which found 78% of its edges were one-off free-text before normalising |
| Architecture claims about role-typed hypergraph stores | Corroborated | Vendor-authored in part; only the architecture is relied on, never the performance claims |
| Cardinality belongs on the relation definition | Verified | Confirmed against TypeDB's constraint documentation, previously unreachable. Its annotations are schema-level only, with no uncertainty bounds and no temporal scoping, which corroborates the class-level half and supplies nothing for instance-level counting |

### Identity

| Claim | Status | Note |
|---|---|---|
| Hard equivalence is too strong for how the link is used | Verified | Established in the linked-data literature |
| Transitive closure manufactures false identities at scale | Verified | 177,000 distinct entities collapsed in the measured case |
| Attribute overlap is unsound because knowledge can be recited | Verified | A record-linkage independence violation |
| Relational structure is expensive to forge | Verified | Collective entity resolution; the patient-attacker caveat stands and is stated in the chapter |
| The substrate wall fixes the behaviour leak | Synthesis | The architectural-metadata principle is established; that it fixes the measured 0.30 relay failure is this design's inference |
| Severance re-derivation is cheap in practice | Bounded by construction | Only revocable assumptions are stamped, in practice zero or one merge, so the voided set is what the record already names and there is no search. Measured small on the live log, whose entire derived-link population is in the low hundreds. Shares a tripwire with the stamp representation below: revisit if mean stamps per derivation ever exceeds one |
| Crumble and accretion thresholds | Open | Empirical tuning, needing data. Genuine same-person profiles also diverge |
| Assumption-stamp representation | Synthesis | Per-event set. A shared environment table is an indirection that pays off when stamps are large and shared, and at a cardinality of zero or one it buys fold-time joins with nothing. Same tripwire as above |

### Belief

| Claim | Status | Note |
|---|---|---|
| Verbalised model confidence is overconfident, saturated, and protocol-sensitive | Verified | Fetched and confirmed |
| A representation separating belief strength from evidence quantity is wanted | Verified | Subjective logic |
| Trust discounting is sound | Verified | |
| Fusion operators | Open, deliberately unused | Named critics attack both the operators and the mapping to evidence counts. The chapter relies only on dependence detection plus a no-gain rule, which is trivially sound |
| Non-prioritised revision is the right default | Verified | Credibility-limited belief revision |
| The exact credence shape | Deferred, not blocking | The lanes disagreed, and the corpus says it is not on the critical path: no claim in the live log is asserted by two distinct human tellers, so teller counts are zero or one and there is nothing to fuse. Ship the representation and trust discounting; the forcing condition is the first claim to accumulate two independent tellers, which the harness can alarm on |
| The agent is a witness to what it is told and never an independent teller of it | Decided | 77 of 198 live entries are agent-told, many restating a participant's own sentence, so without the rule a single sentence read back into the store counts as corroboration from one source |
| Dependence also runs through the agent's own relays | Synthesis | The agent's outbound utterances are glosses whose witnesses are their recipients, which is what makes a claim relayed and later told back detectable as an echo rather than a second source. Untested, and the commonest dependence path in a store the agent reads back to people |
| `expressed` is a provenance qualifier, and a hedge never moves credence | Synthesis | Resolves the showcase arc, where one teller hedges and later asserts flatly with no corroboration anywhere: the credence must not move, and what changed belongs on the telling. Cheaper than a nested attitude for something as constant as hedging, and it keeps "two tellers who both said probably" from reading as confirmation. [`evolution.md`](evolution.md) stage 6's gate is rewritten to match |
| Dependence detection is the common case in a shared channel | Synthesis | Sociality makes corroboration real and dependence ordinary in the same stroke. This moves the load onto the detection rule, which is now the load-bearing part of the credence design rather than a soundness caveat on it |

### Time

| Claim | Status | Note |
|---|---|---|
| Validity intervals on facts, superseded by window-closing | Verified, corroborated | Convergent across temporal databases and every surveyed production graph |
| The three-way occurrence, task, trigger split | Verified as a solution shape | iCalendar is mature prior art. But no surveyed peer has this failure, so the problem may be specific to this system; the fix is sound regardless and should not be oversold |
| The maximal tractable subclass of interval relations | Verified | A settled result. The open question narrowed to whether a simpler subalgebra already suffices |
| Anchor-aware durations and safe recurrence constructors | Verified | Established in modern date-time library design |
| Typed quantities | Observed | Found in prose in the corpus, in the same position dates once occupied |

### The memory typology

| Claim | Status | Note |
|---|---|---|
| The four-way typology | Verified, corroborated | Convergent from the cognitive architecture tradition through to current agent memory work |
| Episodic memory is automatic and architectural where semantic is deliberate | Verified | |
| Episodic as linked companion rather than fallback tier | Single-source | Rests entirely on the dual-trace study below |
| Procedural memory indexed by description embedding, decayed by invocation | Verified | An established agent design |
| Access-frequency and recency ranking is embedder-independent | Verified | The narrow claim is safe |
| That the same ranking is replay-deterministic | Decided | Decided: an agent-visible read appends an event. The objection was arithmetic and the arithmetic does not hold. Read events are a memory-id list against a payload dominated by model calls two orders of magnitude larger, so bytes rise by a fraction of a percent and the event count by roughly a tenth, and the ratio is scale-invariant. The unit is the agent-visible read, never the substrate lanes, so a fused search stays one event |
| The bulk-ingestion path | Synthesis | Designed in the chapter: a durable model-free source layer first, a symbolic pre-filter before a single batched gate call, context-sized extraction batches, per-proposal critics with gloss-only fallback, and one transmission principle per document. Cost is now a function of document length over context window rather than of chunk count. The cost model is owed beside stage 0c, whose harness produces it |
| That human recall activation transfers to agent salience | Synthesis | By analogy, not proof. Only the narrow claim above is relied on |
| Directives are a category error inside the fact model | Observed | 22 of 198 entries, one repeated verbatim ten times |
| An episodic narrative is composed under the intersection rule, never over a confidence | Decided | Closes a second read path with no audience computed on it, using the rule derivations already take rather than a boundary of its own. The measurement corrected an earlier public-only rule: attributed content is a fifth of the live corpus and is repeatable-with-attribution rather than withheld, so excluding it cost a fifth of the depth to protect a single entry. The residual risk, prose being a weaker attribution surface than a field, is met by a structural teller list beside the body |
| Promotion out of working memory intersects a per-note taint set | Decided | Closes the one path where an endorsement could exceed what it was founded under, since a note is founded under nothing. Measured on the live log: memories touched per block run to a median of 1 and a maximum of 11, and even the per-turn accumulation the chapter rejects stays small, so the monotone-convergence worry does not bite at observed deliberation depths |
| "Consulted" is defined operationally, not semantically | Observed | The measurement's real finding. The brief enters context as kilobytes of memory content with no read event, so a semantic definition taints every note with everything on the first note. Explicit reads plus the turn's ambient recall, both already recorded and foldable, with the brief excluded because it is audience-computed before composition |
| Directives as scoped configuration: scope, author, lifecycle, composition | Decided | The live log's connector-minted per-context directives are the shape that forced it: neither always-in-context nor operator-owned, so neither the slot nor the fact model can hold them. The connector authority is new and explicitly bounded to per-context scope |
| The self belongs in a slot rather than in a memory | Synthesis | Follows from the charter needing to be unreachable by the machinery that decides what to omit. The current system's immutable charter entries protect the wording and not the slot, which is an argument from mechanism rather than a measured failure |
| An agent that cannot edit its own charter avoids unbounded drift | Synthesis | The self-reinforcing-loop argument is the same one used for credence and merges. The cost, that growth in self-conception needs operator attention, is stated and unquantified |
| Scratchpad storage | Observed, forced | Settled toward log-with-compaction by the taint set, which is state the fold must reproduce, so a side table is not available. The volume worry that motivated the side table does not survive the ratio: a note is preceded by the recorded model call that wrote it, which is two orders of magnitude larger, and both terms scale with turns |

### Privacy and provenance

| Claim | Status | Note |
|---|---|---|
| Four independent literatures converged on the addressable context-scoped assertion | Verified | |
| Transmission principle as the governing condition | Verified | Contextual integrity |
| Zero residue is formally non-interference | Synthesis | Standard security theory, but the framing lacks one canonical citation to attach |
| Retraction against crypto-shredded erasure | Verified | Established event-sourcing practice |
| A redaction can be proven to have occurred without revealing content | Synthesis | Our combination of shredding with append-only tombstones |
| The retraction-authority lattice | Synthesis | The prior art leaves the subject-differs-from-author case explicitly open; the lattice is this design's answer |
| Combinatorial audience contexts are a genuine gap | Verified as a gap | No surveyed work handles a dynamic context population well. Predicates over the present set are the only tractable form found |
| Marking a derivation as owing recomputation when its premises gain support | Corroborated | Current memory systems mark a synthesis stale when unprocessed evidence bears on it. Ours rides machinery the derivation record already carries; the cost of maintaining the work list is unmeasured |
| Two witness sets, disclosure and exposure | Synthesis | Taken. The narrow set licenses and the wide set only ever suppresses, which is the general principle: a field that only suppresses may be generous, a field that licenses must be demonstrated. Both rest on stage 0b's judgement about who was in the conversation, so they are defined together |
| The belief is absolute and the evidence account is filtered | Synthesis | Taken, and it is the only option consistent with computing visibility once in the substrate: a room-dependent credence makes a derivation rest on different evidence per conversation. Where the only distinguishing fact is an unnameable endorser, the ordinal surfaces with no account, which is the zero-residue standard rather than a special case |
| Principles are universally quantified over the present set and fail closed on any member | Synthesis | The set-shaped evaluator was implied by audiences-as-predicates from the start; the per-principle consequences, and `in_confidence` becoming relative to who was there, are this design's own working out |

### The verified write

| Claim | Status | Note |
|---|---|---|
| Essentially no LLM-to-graph system verifies its neural writes against the source | Verified | The strongest single finding in the welding lane |
| Ontology constraints, not better extraction, suppressed drift in the long-running case | Verified | |
| Autoregressive models cannot reliably self-verify | Verified | The specific benchmark figure is search-sourced and not relied on |
| Prompt brittleness is large, persistent, and does not transfer across models | Verified | Measured spreads up to 76 points from meaning-preserving format changes |
| Forced-choice elicitation removes omission variance | Verified, tempered | It relocates variance into field content and introduces junk fill. Both are stated in the chapter |
| The constraint tax | Open | Schema forcing can degrade tool use or reasoning in some models. Must be measured per behaviour on the target model, never assumed |
| Record-at-call-time neural activities preserve determinism | Verified | Independently validated by durable-execution practice |
| Eager against lazy structuring | Open | How much deduplication judgement moves inline against deferring to a maintenance pass |
| Span justification as a hard critic | Decided | Taken from the current system, which shipped it for one extracted field and measured a roughly 40% reduction in dated occurrences with no date-dependent behaviour regressing ([`research/2026-08-06/current-system-fixes.md`](research/2026-08-06/current-system-fixes.md)). The measurement establishes the need on that field, never the generalisation to every extracted value, which is this design's and is untested. It checks groundedness rather than truth, and the current system's own caveat carries over: a span can be quoted faithfully and still not denote what was read from it |

### The two traces

| Claim | Status | Note |
|---|---|---|
| Narrative alongside structure gains 20 points overall, concentrated in temporal, aggregation, and update tasks, with a clean null on single-occasion lookup | Single-source | One study, one benchmark, an automated judge, 20 questions per category with wide intervals |
| The gain is encoding-side rather than retrieval-side | Unknown | The study could not separate them and says so. This decides whether the design costs a model call per occasion or nearly nothing, and is the first thing [`evolution.md`](evolution.md) resolves |
| Depth beats breadth | Single-source | Their development path: a few points from coverage against twenty from depth |
| Cost neutrality | Does not transfer | An artefact of their prompt volume dominating. For an event-sourced store it is a record-time model call and permanent log volume |
| Some content survives only as narrative | Observed | Independently found in the corpus study, which strengthens this beyond the single source |

### The query surface

| Claim | Status | Note |
|---|---|---|
| Fusing rank orders rather than scores | Corroborated | The convergent form in production retrieval, where parallel lanes are merged on rank position and reranked at the head; found in the [2026-07-24 survey](research/2026-07-24/lanes/survey-issue7.md) and in a contemporary system read directly afterwards. Adopted for its embedder-independence, never for a reported gain, and no surveyed figure is relied on |
| Structural questions are answerable without a model call | Synthesis | Each named question is a graph traversal given the substrate; that the set is sufficient for what the agent asks is untested against real turns |

### Off-turn work

| Claim | Status | Note |
|---|---|---|
| Passes are ordinary writers under the same critics, with no maintenance bypass | Synthesis | Follows from the doctrine that writes are verified (the neural half is never the final authority on a structural question). The current system's passes already write through the block path, so this tightens an existing posture rather than inventing one |
| Queues fed by write-time marks replace whole-store sweeps | Synthesis | Forced by the cost-per-fact constraint. The marks are cheap consequences of records the design already keeps; that the queues stay short under real traffic is unmeasured |
| Structural deduplication and cross-audience merging leave consolidation entirely | Synthesis | Both exist today to recover what the write path did not capture. This is the strongest economic claim in the chapter and it rests on extraction convergence, which [`evolution.md`](evolution.md) stage 0c measures and which is currently unmeasured |
| Triggers are drained before maintenance | Synthesis | A commitment starved by tidying is the failure this prevents. No surveyed system reports the failure; the ordering is cheap enough not to need one |
| Exploration runs on leftover budget and cannot promote itself | Synthesis | The gap is real: a mark-driven store cannot notice a connection nothing marked. The mechanism is trivial and its cost is the whole problem, so it is metered by construction, and its output takes the ordinary one-signal rule, which is stricter than novelty scoring. What is unknown is the yield: whether structural sampling finds anything worth the budget, on an instance of this size, is unmeasured and may be a reason not to run it at all |
| A pass may withdraw a value it did not author, and never substitute one | Decided | Taken from the current system, which reached it after finding that a date the agent wrote at append time was examined by nothing: across 405 runs, 334 appends carried an authored date against 112 the extraction resolved, and only the latter were checked ([`research/2026-08-06/current-system-fixes.md`](research/2026-08-06/current-system-fixes.md)). The asymmetry is what carries the rule, since a withdrawn value disarms and a substituted one arms something else under the original author's authority |
| What deserves initiation | Open | The salience judgement behind agent-initiated contact predates this design and is not settled by it. The chapter states constraints that hold whatever the answer is |

## Unresolved, gathered

Six of these block something. The rest are deferred with a named forcing condition, and saying which is which is the point of the list.

### Blocking

1. Encoding against retrieval for the second trace ([`evolution.md`](evolution.md) stage 0a). Decides the cost of the whole episodic layer, and therefore stage 4's scope.
2. The constraint tax on the target model, per behaviour (stage 2). Decides how many behaviours are schema-constrained.
3. Extraction convergence (stage 0c). The design's central economic claim, that structural equality replaces similarity-threshold deduplication, rests on an extractor converging on the same triple from different prose. The live log supplies a labelled re-mention set for free: every consolidation and arbitration event names entries the running system itself judged to be one claim.
4. The present set, and with it both witness sets (stage 0b). Defines who was in a conversation, which the audience evaluator and the dependence test both read.
5. The bulk-ingestion path's cost model. Its shape is designed in [`memory-typology.md`](memory-typology.md); the four numbers are owed beside stage 0c, whose harness produces them.
6. The console fold budget (stage 0d). The design roughly doubles the log's dominant term, and the replica folds the whole log in browser memory. [`coverage.md`](coverage.md) already calls this a prerequisite; it now has a stage.

### Deferred, with the condition that forces each

7. Eager against lazy structuring. The dial is already designed in [`write-surface.md`](write-surface.md). Forced by stage 2's constraint-tax measurement.
8. Whether the frame's three values are the right three. Forced by stage 1, which is where the referent redirect is tested.
9. Crumble and accretion thresholds for tentative merges. No data exists and none is manufacturable short of a long-running multi-platform instance, so stage 5 ships conservative defaults with an operator exception at the boundary.
10. The credence shape, with fusion operators deliberately unused. Nothing to fuse while teller counts are zero or one; forced by the first claim to gather two independent tellers, which the harness can alarm on.
11. What deserves initiation. [`off-turn.md`](off-turn.md) constrains it without settling it. Forced at stage 9, when an off-turn message is first composed.
12. Whether the structural questions are sufficient. [`query-surface.md`](query-surface.md) names five, and whether they cover what the agent actually asks is untested. Answerable cheaply by classifying the live log's recorded blocks against them, and worth doing before stage 2 freezes the surface.
13. Whether exploration earns its budget. [`off-turn.md`](off-turn.md) bounds the cost and the risk; neither says the yield is positive. Forced whenever the pass is first enabled, and answerable by counting how many exploration notes are ever corroborated into promotion.
14. The dispositional fallback. A habitual modality alongside the frame is the extension [`statements.md`](statements.md) declines. Forced if gloss-only writes on dispositional content become a material fraction of writes.

### Not measurable on this corpus

15. Relay-chain dependence. The live corpus cannot contain the phenomenon: one multi-party conversation, three tellers, and no claim asserted by two of them, so a null would prove nothing. The stage 6 gate is the measurement rather than a prior experiment. Its prerequisite is that the agent's outbound turns are first-class glosses carrying both witness sets, which is a stage 2 obligation.

## Claims deliberately not made

- That the design closes the eleven surveyed failures. It closes six structurally and answers five in design without validation. See [`coverage.md`](coverage.md).
- That any benchmark number here is a target. The one composite memory benchmark surveyed was found by independent audit to have a materially wrong answer key and a judge that accepted most intentionally wrong answers. Component success is measured by structural oracles against this system's own log, not by a self-reported score.
- That autonomy means no human attention. It means exception-triggered attention with a queue that shrinks per fact as the store grows.
- That the neural writer is verified for truth. The critics check well-formedness, typing, consistency, and whether an extracted value is traceable to words in its gloss. That last one is groundedness rather than truth: a well-typed falsehood still passes at write time, and runtime faithfulness checking remains unsolved here as it is everywhere else.
