# Confidence

The normative chapters' claims do not have equal support. This register grades each load-bearing claim, lists unsettled questions, and names the claims the design deliberately does not make. [Evolution](evolution.md) holds the milestones that gather the missing evidence, and [coverage](coverage.md) maps the observed failures to mechanisms.

## Grades

| Grade | Meaning |
|---|---|
| Observed | Established by measurement, code, tests, or current-system documentation. |
| Verified | Checked against a fetched primary source by the verification pass. |
| Corroborated | Independent sources agree. It may qualify Observed or Verified. |
| Single-source | Rests on one study or system and is unreplicated. |
| Decided | An operator or architecture decision. Evidence can support its premise, not prove it. |
| Synthesis | An inference by this design that the cited sources do not themselves make. |
| Unvalidated | Specified, but no test or measurement has exercised it. Usually qualifies Synthesis. |
| Open | Named alternatives exist and no evidence yet chooses. |

Two adversarial passes ran over the research report ([verification](research/2026-07-24/verification/)). Of its load-bearing cited claims, 29 were confirmed against fetched primary sources, 5 corrected, 0 unsupported, and 1 unreachable; every future-dated citation was fetched directly. The corrections are folded into the chapters: provenance polynomials as justification labels are this design's synthesis rather than the source's claim; the conservatism about subjective logic extends from its fusion operators to its evidence-count mapping; severance re-derivation is a record-time pass, not part of the fold; the long-running accumulator's drift is a monotonic decline in newly promoted claims, not a cliff; and forced-choice elicitation is tempered to distinguish omission variance from content variance.

From milestone 2a onwards, a privacy-semantics claim graded Synthesis, unvalidated is re-graded when its scenario passes in the [thin reference model](evolution.md#milestone-2a-thin-reference-model).

## Evidence map

Internal evidence is local code, documentation, tests, or measurement; external evidence is under `research/`. Where no suitable evidence exists the row says so rather than substitute a weak analogue. Claims whose only source is their owning chapter are listed after each table by grade.

### Permanence and object model

| Claim | Grade | Source |
|---|---|---|
| An append-only log with deterministic replay, and durable records of nondeterministic Activities, are a sound base | Observed locally; Verified, corroborated externally | [storage contract](../docs/events-and-storage.md); [verification](research/2026-07-24/verification/part-b.md); [survey](research/2026-07-24/lanes/survey-giants.md) |
| A reified contextual assertion with qualifiers and references is a proven shape | Verified | Wikidata's data model; [fact-shape lane](research/2026-07-24/lanes/fact-shape.md) |
| The Proposition, Assertion, and Attestation split, with Proposition as a computed key | Synthesis | [report](research/2026-07-24/report.md) |
| Qualifiers stay one level deep or scope becomes ambiguous | Verified | Wikidata guidance |
| Source text belongs to an Occasion; one utterance grounds many claims | Observed | Up to eight claims in one entry, 20 entries with three or more clauses ([modelling study](research/2026-08-03/modelling-study.md#compound-entries-fragment-and-the-gloss-stops-being-one-to-one)) |
| Some content has no faithful structure and stays source-only | Observed | [figurative content](research/2026-08-03/modelling-study.md#figurative-content) |
| Referential frames are needed | Observed | 77 entries (39%) sit on one persona memory with layer mixing ([modelling study](research/2026-08-03/modelling-study.md#referential-layering-and-it-is-39-of-the-corpus)) |
| The closed frame set and the `principal` redirect through `presents` | Synthesis, unvalidated | Simplifies Cyc microtheories (Verified); `extraction-convergence` tests it |
| Proposition references in an object slot, one level deep, only for attitudes about mental state | Observed need; nesting bound Unvalidated | 35 attitude entries, some depth two ([modelling study](research/2026-08-03/modelling-study.md#propositional-attitudes-need-a-statement-as-an-object)) |
| Metalinguistic claims need Occasion and span references as objects | Observed | 4 of 198 entries ([log measurements](research/2026-08-06/log-measurements.md#metalinguistic-content)) |
| Counts as typed values; instance counting differs from class cardinality | Verified | Wikidata quantity datatype; OWL 2 ([lineage](lineage.md#cardinality-moved-from-the-class-to-the-individual)) |
| Polarity canonicalisation and the mechanical contradiction subset | Synthesis informed by Verified revision principles | [identity and belief lane](research/2026-07-24/lanes/identity-belief.md) |
| Third-party deontic content requires a modality coordinate | Observed | [modelling study](research/2026-08-03/modelling-study.md#third-party-deontics) |
| Collective and distributive plurals stay source-only | Verified premise; Decided scope | Conceptual graphs mark it natively |

Decided: the permanence contract, upcast rule, and keep-at-genesis test ([overview](overview.md#permanence-contract)); reported speech always `quoted` ([object model](statements.md#mode-and-reported-speech)); Entities with a kind fixed at mint and handles as labels over ULIDs ([entity](statements.md#entity)); definition IDs only in the Proposition key; outbound Occasions and `in_reply_to` links as genesis data, for #109; vectors as recorded, erasable outputs ([embeddings](query-surface.md#embeddings)). Synthesis: an immutable Assertion mode in the reuse match, without which a quotation and a later flat assertion collapse. Synthesis, unvalidated: non-preclusion of distributed operation ([overview](overview.md#distributed-operation)); the typed Attestation source ([attestation](statements.md#attestation)); predecessor-naming transitions ([lifecycle mechanics](statements.md#lifecycle-mechanics)); and the Assertion reuse rule, whose ambiguous attachments `extraction-convergence` counts.

### Privacy, provenance, and erasure

| Claim | Grade | Source |
|---|---|---|
| Transmission principles as the governing condition on a flow | Verified | [contextual integrity](research/2026-07-24/lanes/provenance-privacy.md#4-contextual-integrity-transmission-principles-as-first-class-data) |
| Dynamic audience populations are a gap in the literature | Verified as a gap | [provenance lane](research/2026-07-24/lanes/provenance-privacy.md#implications-for-zuihitsu) |
| An inbound Occasion's restriction is its availability audience, `channel(C)` for a channel | Decided; roster semantics Unvalidated | `witness-assurance-audit` records later-joiner history |
| Scoped witness evidence; disclosure licensed narrowly, exposure suppressing broadly | Synthesis, unvalidated | #123 is the observed failure; `witness-assurance-audit` tests it |
| The testimony floor is the source Occasion's restriction, widened only by a grant or the operator (option a) | Decided | The cost is recall, which `principle-assignment` measures to decide whether option b reopens |
| Grants and relation confirmation fail closed on their soft critics | Decided rule; thresholds Open | `principle-assignment` sets the thresholds ([soft critics](verified-write.md#soft-critics)) |
| The testimony-grounding hard critic closes value and reference laundering | Synthesis, unvalidated | `write-surface-comparison` measures its rejection and catch rates |
| A delivered Occasion contributes only its delivered-audience restriction | Synthesis, unvalidated | Sound only while the pre-delivery check is; `taint-breadth` tests drift towards teller-only |
| The three subject-guard candidates | Open | `guard-denial-rate` decides; `v1` is the minimum-sample default |
| Central audience resolution before ranking | Decided from observed leaks | A visible link row carried a withheld entry's date ([current-system fixes](research/2026-08-06/current-system-fixes.md#redaction-decided-per-read-path)) |
| Access is content rendered into a model context | Decided | Brief content enters context with no read event ([log measurements](research/2026-08-06/log-measurements.md#what-a-block-touches)) |
| Working-note taint is small enough to allow promotion | Observed proxy only | Memories touched per block: median 1, maximum 11; manifests add briefs of 2,240 to 8,106 characters, so real taint is larger; `taint-breadth` measures it |
| Retraction against payload-deleting erasure beside an append-only envelope | Verified | [provenance lane](research/2026-07-24/lanes/provenance-privacy.md#5-forgetting-vs-append-only-reconciling-erasure-with-deterministic-replay) |
| Erasure closure follows typed dependencies; co-rendered dependants go to operator review | Synthesis, unvalidated | `erasure-closure-size` measures closure and review load |
| Marking a derivation as owing recomputation when its inputs change | Corroborated | Current memory systems mark syntheses stale |

Decided in [privacy and provenance](privacy-and-provenance.md): `attributed` as a rendering obligation, with replies audited; resolution inputs conjoined with the testimony principle; the teller as producer of the cited span ([teller binding](write-surface.md#teller-binding)); every person-valued position guarded; minted Artefact IDs with the digest in erasable payload. Synthesis: universal quantification failing closed; zero residue as non-interference, standard theory without one canonical citation; the erasure authority table, where prior art leaves the subject-differs-from-author case open; public definitions with a hard critic against live handles, paraphrase left to a soft critic ([definition audience](relations.md#definition-audience)); separate global and audience-safe support. Synthesis, unvalidated: context manifests through one choke point, with influence, taint, restriction, and access as projections, whose completeness is load-bearing; the pre-delivery check; restricted coined names and handles; per-Assertion guard scope, with candidate-set scope only for Event reads; per-audience folding of validity and every Assertion transition ([per-audience state](statements.md#per-audience-state)); per-audience access at an explicit as-of time; the salted payload commitment; the ledger with boot reconciliation; and exclusive in-server erasure.

### Events, relations, identity, and belief

| Claim | Grade | Source |
|---|---|---|
| One Event with role Assertions fixes per-subject copies; role inventories past two positions are inconsistent even among experts | Verified | Neo-Davidsonian semantics; W3C n-ary patterns ([report](research/2026-07-24/report.md#32-events-and-roles-the-fix-for-one-event-many-copies)) |
| The small universal role set suffices for the corpus, and Event-to-Event relations are needed | Observed | [modelling study](research/2026-08-03/modelling-study.md#the-multi-participant-event), [event relations](research/2026-08-03/modelling-study.md#events-have-no-relations-to-other-events) |
| Duplicate Events are acceptable, with no co-reference at genesis | Decided | Two of four local entries denoted one happening; `event-coreference` reopens on measured cost |
| Deprecate-and-alias; domain and range catch reversed edges | Verified, corroborated; Observed | OWL and LinkML; reversed edges in the live graph ([issue 7 survey](research/2026-07-24/lanes/survey-issue7.md)) |
| Cardinality belongs on the relation definition | Verified | TypeDB constraint documentation |
| Agent coinage under critics contains vocabulary drift | Synthesis, unvalidated | One peer found 78% one-off edges (Single-source); the critic depends on geometry; `relation-coinage-replay` tests it |
| Hard equivalence and transitive closure are unsafe for identity | Verified | 177,000 entities collapsed in the measured case ([identity lane](research/2026-07-24/lanes/identity-belief.md)) |
| Attribute overlap is unsound merge evidence; relational structure is costly to forge | Verified | Costly is not impossible: a patient attacker who joins the same network can forge it ([identity](identity.md#evidence-and-authority)) |
| One handle fixes the sibling-relay behaviour | Synthesis, unvalidated | The measured 0.30 relay rate supports the need, not the fix |
| Resolution environments stay small | Observed at current scale | Zero or one merge per derivation; revisit above one on average |
| Verbalised model confidence is overconfident and protocol-sensitive | Verified | [identity and belief lane](research/2026-07-24/lanes/identity-belief.md#calibration--the-neural-writers-confidence-cannot-be-trusted-raw) |
| Non-prioritised revision; trust discounting; strength separate from evidence quantity | Verified | Credibility-limited revision; subjective logic |
| Fusion operators | Open, deliberately unused | Named critics attack the operators and the evidence-count mapping |
| The agent is a witness, never an independent teller, of what it is told | Decided | 77 of 198 entries are agent-told ([log measurements](research/2026-08-06/log-measurements.md#visibility-and-telling)) |
| Hedge-then-flat is one lineage with two Attestations | Observed case; Synthesis treatment | [modelling study](research/2026-08-03/modelling-study.md#hedges-become-credence) |
| Relay-chain dependence through outbound Occasions and `relayed_from` lineage | Synthesis, unvalidated | `dependence-lineage` exercises it on scripted corpora only |
| Settlement is a read-time projection per audience over eligible support | Decided | No claim has two human tellers, so requiring corroboration would make every claim unrecallable (Observed premise); a single teller's falsehood is default-readable as `single_source` |

Decided: contest is mechanical only, and `contested` means a live mechanical contradiction visible to the audience ([contest](belief.md#contest-and-contradiction)); the ResolutionEnvironment is recomputed from a recorded frontier ([identity](identity.md#resolution-environments)). Synthesis: multiple fillers per role, stable Event identity, and disclosure-safe projection, since the study tested only the role inventory; identity-only resolution hypotheses, never transitively closed; the one-handle representation, a structural closure the reference-model scenarios test. Synthesis, unvalidated: the `recall` and `disclosure` split, enforced at genesis by rendering recall-cleared content only into operator diagnostics; writes through a composite landing on its primary, with severance listing them for re-homing ([identity](identity.md#writes-through-a-composite)).

### Time, memory, writes, reads, and jobs

| Claim | Grade | Source |
|---|---|---|
| Validity intervals closed by window-closing | Verified, corroborated | [time lane](research/2026-07-24/lanes/time-memory.md#the-canonical-two-axes) |
| Separating occurrence from action (a Task with triggers) | Verified solution shape; failure is local | iCalendar VEVENT, VTODO, and VALARM ([verification](research/2026-07-24/verification/part-b.md#verdict-table)) |
| Anchor-aware durations and safe recurrence constructors | Verified | [time lane](research/2026-07-24/lanes/time-memory.md#rrule-pathologies-to-guard-at-the-typed-boundary) |
| Event-bounded validity suffices at genesis | Synthesis, unvalidated | The tractable Allen subclass is Verified; whether a smaller set suffices is Open |
| The four-way memory typology | Verified, corroborated | [time and memory lane](research/2026-07-24/lanes/time-memory.md) |
| Directives and the self are configuration, not memory | Observed need; Decided | 22 of 198 entries are instructions, one repeated ten times ([modelling study](research/2026-08-03/modelling-study.md#directives-are-not-assertions)) |
| Generated episodes gain on temporal, aggregation, and update tasks | Single-source | 40, 30, and 25 points, automated judge, about 20 questions each; encoding and retrieval unseparated ([dual trace](research/2026-08-03/dual-trace.md)) |
| Almost no LLM-to-graph system verifies neural writes against the source | Verified | [welding lane](research/2026-07-24/lanes/welding.md) |
| Ontology constraints suppressed drift in the long-running case; models cannot reliably self-verify | Verified | [verification](research/2026-07-24/verification/part-b.md) |
| Prompt brittleness is large and does not transfer across models | Verified | Spreads up to 76 points from meaning-preserving format changes |
| Forced choice removes omission variance | Verified, tempered | It relocates variance into junk fill |
| Span justification as a grounding critic | Decided | About 40% fewer dated occurrences on one field; untested on every value ([current-system fixes](research/2026-08-06/current-system-fixes.md#span-justification)) |
| Rank fusion by position | Corroborated | Adopted for embedder independence, not a reported gain ([issue 7 survey](research/2026-07-24/lanes/survey-issue7.md)) |
| Structural deduplication removes the need for consolidation | Synthesis | Rests on unmeasured convergence, with a supersession-proposal fallback |
| A background job may withdraw a value it did not author, never substitute one | Decided | 334 appends carried an authored date against 112 resolved by extraction, and only the latter were checked ([current-system fixes](research/2026-08-06/current-system-fixes.md#withdrawal-without-substitution)) |
| Composite benchmark scores are not targets | Verified critique; Decided practice | [verification](research/2026-07-24/verification/part-b.md) |

Open: write surface A against B, and eager against deferred structuring, which `write-surface-comparison` decides. Decided: `wake_turn` as the one genesis Task action, checked at creation and firing ([time](time.md#occurrence-and-task)); a book claim as a `derivation` whose span lies within a rendered span ([reading a document](artefacts-and-perceptions.md#reading-a-document)). Synthesis: proposal states and atomic publication, forced by current machinery and permanence; structural questions need no model call (`query-classification` tests sufficiency); change-driven queue marks, with queue length under real traffic unmeasured; triggers drain before maintenance, a cheap ordering for a failure no surveyed system reports. Synthesis, unvalidated: the episodic wall as a manifest-ancestry check ([two traces](two-traces.md#the-episodic-wall)); stateless workers with first-valid-result commit ([off-turn work](off-turn.md#workers)).

## Open questions

Each [milestone 1 experiment](evolution.md#experiments) resolves the question it decides, which is not repeated here. The questions below cut across experiments or wait on later evidence.

- Which coordinate carries the extraction failures, and do one-level nesting and source-only fallback cover depth-two attitudes? `extraction-convergence`.
- How sensitive are the privacy experiments to importing posture labels as principles? Their sensitivity reports.
- Does over-taint block useful working-note promotion? `taint-breadth`, then milestone 3.
- Do the dependence rules hold on real relays? The first real relay chains after genesis.
- Which seed definitions, Event subroles, and temporal uncertainty, timezone, and recurrence policies does genesis need? Milestone 4.
- What thresholds strengthen or withdraw a tentative identity composite, given that genuine same-person profiles also diverge? `autonomous-identity`.
- What support arithmetic applies once independent tellers exist? `support-fusion`.
- Is a generated episode's value encoding-side or retrieval-side, and is a rule narrower than the strict wall safe through delivered Occasions? `generated-episodes`.
- What deserves proactive initiation, and does exploration yield anything at this scale? `proactive-initiation` and `exploration`.
- Can any checker establish runtime faithfulness? No mechanism is proposed.

## Claims deliberately not made

- That the design closes the eleven surveyed failures. It closes six structurally and answers five in design without validation ([coverage](coverage.md)).
- That any benchmark number is a target. The one composite memory benchmark surveyed had, by independent audit, a materially wrong answer key and a judge that accepted most intentionally wrong answers. Success is measured by structural oracles against this system's own log.
- That autonomy means no human attention. It means exception-triggered attention with a queue that shrinks per fact as the store grows.
- That the neural writer is verified for truth. Critics check well-formedness, typing, consistency, and traceability to a source locator: groundedness, not truth. Runtime faithfulness checking remains unsolved.
- That testimony grounding shows a teller asserted the relation chosen. Relation choice rests on a fail-closed soft critic and audit sampling.
- That the grant and relation soft critics are accurate. Failing closed turns a false refusal into lost recall; a false confirmation still widens or launders, bounded only by the measured rate.
- That an `attributed` reply keeps its attribution. The obligation binds the rendered context; replies are audited.
- That settlement means corroboration. The ordinal reports corroboration.
- That one handle fixes identity leakage in behaviour. The representation closes it structurally; the behavioural fix remains an inference.
- That scripted corpora show how real participants behave, or that current posture labels are transmission principles. Both are reported as approximations.
- That influence tracking or erasure closure is complete. Each is exactly as complete as the manifests and typed dependencies, and a renderer that bypasses the choke point under-taints without detection.
- That the successor supports distributed operation. It avoids precluding it and designs no sync layer.
