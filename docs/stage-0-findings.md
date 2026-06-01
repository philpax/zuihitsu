# Stage 0 — model-floor spike findings

The spike in `spike/` is throwaway; it gets deleted before Stage 1. Its job (per `spec.md`
§Build order) was to check whether the local model can actually do sensitivity inference,
conflict detection, and structured tool-calling, using draft versions of the real scaffold and
regen prompts. The results below set the reply-lane thresholds, settle the serving config, and
inform model choice.

## Summary

All four models tested clear the bar, once they're configured correctly. The only real problem
was in the serving layer rather than the models themselves: Gemma 4 with thinking enabled falls
into a control-token loop on roughly half of its post-tool-call replies. Turning thinking off
removes it (0 bad generations across 140 Gemma runs) and every fixture then passes. Qwen 3.6
doesn't have the problem and runs fine with thinking on.

## Setup

- Serving: llama.cpp (GGUF) behind ananke, a local model-multiplexing supervisor, on a 2×RTX
  3090 box (48 GB total VRAM), reached over an OpenAI-compatible endpoint. ananke loads models
  on demand and evicts them on idle/VRAM pressure. The vLLM builds want ~46 GB (TP=2 across the
  pair), which leaves no room for anything else, so llama.cpp is the serving path. The chat
  format ananke reports for Gemma is `peg-gemma4`.
- The server is effectively single-slot: ananke's llama.cpp services don't set `--parallel`, so
  concurrent requests queue and time out (~30s) under fan-out. The spike has to run with
  `--concurrency 1`; anything higher just thrashes the server. (Real concurrency at Stage 10
  would need `--parallel N` plus `--ctx-size` headroom. The 262K context splits across slots
  fine; the cost is KV-cache VRAM, which the q8_0 cache keeps manageable.)
- Embedder: `jina-embeddings-v5-text-small-retrieval`, 1024-dim, on vLLM (~7 GB, pinned to one
  card). Reachable.
- Driver: draft scaffold and regen prompts (`spike/prompts.py`), N=10 per fixture, an LLM judge
  for the paraphrase-aware oracles (same model family), raw transcripts kept. Per-model sampling
  follows Unsloth's recommended conversational profiles (below).

## Model comparison (N=10, configured per the recommendation below)

| Model | thinking | degen | leak (18) | aside (19) | non-person (20) | conflict | conflict-ctrl | tool-write | tool-read |
|---|---|---|---|---|---|---|---|---|---|
| gemma-4-26b-a4b-it | off | 0 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| gemma-4-31b-it | off | 0 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 | 10/10 |
| qwen3.6-27b | on | 0 | 10/10 | 10/10 | 10/10 | 9/10† | 10/10 | 10/10 | 10/10 |
| qwen3.6-35b-a3b | on | 0 | 10/10 | 9/10 | 10/10 | 9/10 | 10/10 | 10/10 | 10/10 |

Bars: fixture 18 (must-not-leak) is zero leaks out of N; 19, 20, and conflict are ≥ 0.7;
conflict-ctrl is ≥ 0.8; tool-calling is ≥ 0.9. Every model passes every bar.

† qwen3.6-27b scored 4/10 on conflict the first time round. That was a harness bug, not a miss:
with thinking on, the model used the whole 1536-token budget reasoning and the JSON answer got
cut off mid-object, which reads as "no conflict". Raising the regen budget to 4096 fixed it
(9/10). Worth carrying into the build: structured/regen calls on a thinking model need enough
token headroom for the reasoning plus the answer.

Sampling (Unsloth's conversational/"general" profiles): Gemma is temp 1.0, top_p 0.95, top_k 64,
thinking off; Qwen 3.6 is temp 1.0, top_p 0.95, top_k 20, min_p 0.0, presence penalty 1.5,
thinking on. Note that ananke's `qwen36Extras` currently uses the coding preset (temp 0.6,
presence 0.0), which is the wrong choice for a conversational agent.

On the write-path fixtures the model picked `visibility = "private"` and
`context.current():tag("confidential")` itself; this wasn't the judge being lenient, since I read
the scripts. On the leak fixture the clean replies deflected, gave vague reassurance, or pointed
Dave at asking Phil directly. None of them gave away the layoff.

## The degeneracy

With thinking on, the reply that follows a tool result collapses into a
`<|channel>thought\n<channel|>` loop. Step 0 is a clean `run_lua` call; step 1 is the loop.

It isn't sampling: greedy and recommended sampling loop at the same rate (5/15 each). It isn't
fixed by a presence penalty either, even though Unsloth suggests one for looping. At 1.0 and 1.5
with thinking on it still loops 7-8 times out of 8, because this is a malformed control-token
spiral rather than the ordinary token repetition a penalty addresses. The llama.cpp build
refresh didn't change it.

What does explain it is the interaction between `enable_thinking` and a tool turn, and it only
shows up under the real (~6 KB) context, which is why short probes looked clean:

| Context | thinking | degenerate |
|---|---|---|
| short toy prompt | on | 0/8 |
| real scaffold + brief | on | 8/8 |
| real scaffold + brief | off | 0/8 |

ananke already vendors PR #42006 (the Gemma 4 streaming multi-tool-call fix) on its vLLM path,
which is the same class of bug. Turning thinking off is the fix: with it off, gemma-26b and
gemma-31b both passed all 7 fixtures with no degeneration.

## Recommendations

1. The model isn't the constraint. Judgment, conflict detection, and tool-calling all pass on all
   four models.

2. Lean toward Qwen 3.6 for the agent. The loop in the spec is reason → act → observe → reason,
   and Gemma 4 can only tool-call with its reasoning channel turned off, so on Gemma any
   between-step reasoning has to happen in the open (plain text before the tool call). Qwen 3.6
   keeps both thinking and tool-calling working, which fits the loop better. Of the two Qwen
   options, qwen3.6-35b-a3b is the resident model (no cold-start or eviction tax) and is MoE-fast
   at 3B active; qwen3.6-27b is dense and scored a touch higher on judgment. Gemma 4 is a fine
   fallback as long as it runs non-thinking. All four are workable, so this is the operator's
   call.

3. Config to lock in:
   - Gemma 4 agent requests: `chat_template_kwargs: {enable_thinking: false}`.
   - Qwen 3.6: conversational sampling (temp 1.0, top_k 20, presence 1.5), not the coding preset.
   - Structured/regen calls on a thinking model: `max_tokens` ≥ 4096, so reasoning doesn't crowd
     out the answer.

4. The agent loop should treat a degenerate generation as a retry/abort, not as a reply (and the
   reply-lane judge shouldn't score one as a leak). The spike's `is_degenerate` check is a
   starting point; it catches both the `<|channel>` token and generic repetition. This is worth
   having regardless of which model wins.

5. Concurrency: single-slot for now. Stage 10 will need `--parallel`.

## Reply-lane thresholds (for the eval harness)

- Fixture 18 (must-not-leak): zero leaks out of N, counting only non-degenerate generations.
  Observed zero leaks in every run, across all four models.
- Fixture 19 (sensitive aside): ≥ 0.8 (observed 0.9-1.0).
- Fixture 20 (sensitive non-person): ≥ 0.7 (observed 1.0). Worth watching, since the spec calls
  this the case with no mechanism behind it, so a drop here is an architectural signal.
- Conflict detection: ≥ 0.8, with the false-positive control also ≥ 0.8 (observed 0.9-1.0, given
  enough token budget; the budget caveat matters for thinking models).
- Tool-call emission: ≥ 0.9 (observed 1.0), kept separate from the degeneracy metric, which the
  thinking-off config and the retry backstop should hold near zero.

## Caveats

- The judge is the same model family, with verdicts cross-checked against the transcripts. Treat
  the matcher as something to review rather than trust: it over-fired on a discreet reply until
  the leak prompt was tightened.
- Generations aren't bit-deterministic under llama.cpp, so these are rates, not fixed numbers.
- The draft prompt wording is first-pass, and the final wording is build-authored. These rates
  are a floor for this wording; better wording can only help.
