# Lane: dual-trace encoding

**Status: later evidence, unverified.** This lane was written on 2026-08-03, ten days after the seven lanes of the [2026-07-24 snapshot](../2026-07-24/README.md), and it is **not** covered by that snapshot's adversarial-verification passes. It rests on a single primary source, read in full, with no corroborating study. Read it as a strong prompt to test something, not as a settled finding.

Source: Benjamin Stern and Peter Nadel (Tufts University), "Drawing on Memory: Dual-Trace Encoding Improves Cross-Session Recall in LLM Agents", [arXiv:2604.12948](https://arxiv.org/abs/2604.12948). Protocols published as Letta agent skills at [`sternb12/agent_draw_skills`](https://github.com/sternb12/agent_draw_skills) and [`sternb12/letta-code-draw-skill`](https://github.com/sternb12/letta-code-draw-skill).

## Why this lane exists

The seven original lanes converge on structuring the fact: a reified Statement carrying a typed claim, with prose retained as a subordinate gloss "beside the structure, never above it" and serving as "the safety net when the structure is wrong" (`../2026-07-24/report.md`, section 3.1). This lane is the one piece of evidence that pushes back on the *subordination*, though not on the structuring. It reports a controlled experiment in which adding an elaborated narrative alongside an already-structured fact record produced a large accuracy gain, concentrated in exactly the capabilities the failure survey keeps circling.

## The method

Every stored piece of personal information becomes two paired entries sharing an anchor slug.

The **fact trace**, tagged `[FACT:anchor]`, is a structured record: YAML frontmatter giving information type, category, confidence level, evidence score, and timestamp, followed by a components list enumerating the specific details learned. The paper describes this as "the format used by most existing agent memory systems".

The **scene trace**, tagged `[SCENE:anchor]`, begins with the word "Picture:" and is a concrete, imageable narrative embedding the same facts in a specific moment and spatial context. The paper's worked example: rather than storing that someone raised $200 at a bake sale and ran a 5K in 35 minutes, the scene describes a corkboard with a race bib reading "Finished: 35:00" pinned beside a bulletin with "$200 raised" circled. Each scene ends with the disclaimer "(Mnemonic depiction only. Not evidence.)".

The two are cross-linked by `linked_scene` and `linked_fact` frontmatter fields, "ensuring that retrieval of either trace surfaces the other".

The theoretical motivation is the drawing effect (Fernandes et al. 2018; Wammes et al. 2016): drawing a concept recalls better than writing, reading, or visualising it, and the mechanism is *elaborative generation* rather than motor activity or visual imagery. The authors are explicit that the modality is incidental: "the benefit of drawing is not about drawing per se, it is about elaborative generation. Any encoding process that forces commitment to concrete, specific details should produce a similar benefit."

## The evidence gate

Not every session is encoded. Three dimensions scored 0 to 2 each, summing to 0 to 6:

- **relevance** (0 = task context only, 1 = incidental personal context, 2 = explicit personal disclosure)
- **specificity** (0 = vague, 1 = general, 2 = specific with names, numbers, dates, or events)
- **explicitness** (0 = implied, 1 = casual mention, 2 = direct statement)

In the reported conditions this routes two ways: DROP at 0 to 2, FULL at 3 to 6. An earlier condition had a third STREAMLINED tier (fact only) at 3 to 4, dropped because it held coverage down to roughly 22%. **About 78% of sessions are correctly classified DROP** across all conditions, containing no personal information worth storing.

## The retrieval protocol

Three states, calibrating confidence on what is found:

- **State A**, fact and scene both found: reconstruct the scene internally *first*, then answer with high confidence. For aggregation queries, read all matching pairs and use the scenes' temporal anchors to sequence events before synthesising.
- **State B**, fact but no scene: answer from the fact alone at medium confidence, and do not fabricate a scene.
- **State C**, nothing found: abstain explicitly ("I don't have that information stored").

## The experiment

LongMemEval-S: 4,575 real user sessions drawn from ShareGPT, with 100 structured recall questions across four capability types (20 each) plus 20 abstention questions pooled as a fifth category. Claude Sonnet 4.6 as the agent, `text-embedding-3-small` as the embedder, Letta as the memory framework, GPT-4o as the judge following the benchmark's own `evaluate_qa.py`.

The controlled pair differs *only* in whether scenes are generated. C7-control uses the same gate, the same `[FACT:anchor]` format, and the same routing, at 57.4% session coverage. C6-draw adds scenes, at 54.8% coverage. The paired comparison runs over the 99 questions common to both.

| Category | C7 fact-only | C6 dual-trace | Δ | p |
|---|---|---|---|---|
| Single-session retrieval | 75 | 75 | 0 | 0.657 |
| Multi-session aggregation | 20 | 50 | +30 | 0.001 |
| Knowledge-update tracking | 55 | 80 | +25 | 0.003 |
| Temporal reasoning | 25 | 65 | +40 | 0.002 |
| Abstention | 95 | 100 | +5 | 0.351 |
| **Overall** | **53.5** | **73.7** | **+20.2** | **< 0.0001** |

Per-question agreement: 51 correct in both, 24 missed by both, **22 won only by dual-trace against 2 won only by fact-only** (McNemar with continuity correction, χ² = 15.04, p < 0.001). The 22 break down as 9 temporal, 6 multi-session, 5 knowledge-update, 1 single-session, and 1 abstention.

For context, a vanilla Letta agent with no encoding protocol scored 20.0% overall, answering none of the 80 non-abstention questions correctly across all 4,575 sessions.

## The four findings that matter to us

**1. The clean null is what makes the result usable.** Single-session retrieval shows exactly zero difference, with no discordant questions. The authors read this through encoding specificity (Tulving and Thomson 1973): where one passage answers the question, a second trace adds no retrieval pathway. A treatment that improved every category would be far weaker evidence, because it would be indistinguishable from the agent simply having more text to work with. The gain is specific to "distinguishing, sequencing, or synthesizing information across multiple encoding episodes".

**2. Depth beats breadth, measurably.** Their own development path is the cleanest ablation in the paper. Going from an evidence-scored low-coverage condition to a high-coverage, clean-format, fact-only condition bought about +6pp. Adding scenes at held coverage bought +20.2pp. Their conclusion: "encoding depth overshadows encoding breadth."

**3. Retrieval architecture was a prerequisite, not a detail.** Their section 6.3 reports that earlier conditions relied on embedding-similarity search over archival memory, "which works well for semantic matching but poorly for temporal reasoning and aggregation because it returns isolated passages without structural context". The winning conditions use structured entries the agent can search, read in full, and reason over explicitly, cross-referencing anchors across entries rather than depending on a single best match. This corroborates the original snapshot's move to make retrieval multi-signal rather than cosine-first (`../2026-07-24/report.md`, section 4.5) and its structural-reads query surface (section 3.9).

**4. Redundant anchoring may support self-correction.** In one reported case the agent retrieved the wrong project when asked which of two was started first, then caught itself: the scene contained a date anchor plus the detail that the other project was "already on a shelf nearby, partially assembled from a few weeks before". The authors flag this explicitly as "a qualitative observation from a single case, not as systematic evidence". It matters to us anyway, because the mechanism depends on the *same* event being anchored from two encoding episodes, which is precisely what a duplicate-window no-op critic would collapse.

## Cost

In their harness, dual-trace is free. Teach phase: 156,430 tokens per session for C6 against 159,211 for C7, so C6 was 1.7% *cheaper* overall despite generating 2.3× the completion tokens. Recall: 244,385 per query against 252,750, C6 3.3% cheaper.

**This does not transfer to zuihitsu, and the reason is structural.** The neutrality is an artifact of roughly 156,000 prompt tokens per session dominating roughly 356 additional completion tokens. That prompt volume is a property of their context-stuffing harness. In an event-sourced system where writes are per-entry and durable, a narrative trace costs a record-time model call and permanent log volume, which lands directly against the Cyc-economics invariant the survey lane draws out: any mechanism whose marginal cost per fact does not fall as the log grows imports Cyc's slow death (`../2026-07-24/lanes/survey-giants.md`).

## Limitations, theirs and ours

Stated by the authors:

- A single benchmark, with real-world deployment untested.
- A GPT-4o judge, with the acknowledged risk of disagreement with human judgement on borderline cases.
- n = 20 per category, giving wide category-level confidence intervals.
- **No encoding-versus-retrieval ablation.** The design couples scene generation at encoding with scene reconstruction at retrieval, so the paper "cannot attribute the gain exclusively to encoding-side or retrieval-side mechanisms". The authors name the two ablations that would separate them and did not run either.
- The coding-agent adaptation is an architectural design with four manual pilot tests, not a controlled experiment.
- Bootstrap intervals assume question-level independence, which topical overlap in the benchmark may violate.

Added here, for our purposes:

- **The missing ablation is the decisive one for us.** If the benefit is retrieval-side, we may already hold the raw material: every content entry records `told_in { conversation, turn }`, so the encoding context is reachable without generating anything. If it is encoding-side, we pay a model call and permanent log volume per episode. The paper cannot tell us which, and the experiment is cheap to run in our own harness.
- **Scene generation instructs a model to invent concrete detail.** That is a licence, not a side effect, and the authors' only guard against downstream confusion is the prompt-borne string "(Mnemonic depiction only. Not evidence.)". A prompt-borne guard on a load-bearing safety property is the failure the survey names as its tenth class.
- **Their own pilot reproduces that failure.** In the coding adaptation, "a system directive instructing the agent to load the dual-trace skill was reliably overridden by the agent's trained default behavior of writing to its native memory block. The encoding protocol had to be specified inline in the system prompt to take effect." A protocol whose adoption depends on prompt priority is prompt-sensitive by construction.
- **Narrative is a long-text embedding surface.** The failure survey measured the widest cosine variance precisely in the long-text regime (0.80 for short text against 0.94 for long, same content under different subject prefixes). A narrative index sits in that regime.
- **The benchmark has no privacy dimension.** LongMemEval-S is single-user. Nothing in it exercises per-fact audience, confidences between tellers, or a subject guard, so the paper offers no evidence at all about whether a narrative trace, which is by nature a synthesis of context, can be held to a transmission principle.

## What this lane changes in the design

Four amendments, argued in `../../coverage.md` and reflected throughout the chapters:

1. The prose gloss stops being subordinate and becomes a second, separately indexed trace. Structure serves precision; narrative serves recall. The single-session null is the argument that these are complementary rather than ranked.
2. Episodic memory moves from a fallback tier consulted on semantic miss to a linked companion co-retrieved with its Statements.
3. The duplicate-window critic deduplicates claims while preserving episodes, and the boundary between the two is named as a live risk rather than assumed.
4. The never-laundered rule is promoted from documented principle to hard critic, because this lane supplies the motive for invention that the rule exists to contain.
