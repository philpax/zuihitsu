# Stage 0 — model-floor spike (throwaway)

This directory is **disposable**. Per `docs/spec.md` §Build order, Stage 0 answers the one
question no mechanism can: *can the target local model actually do sensitivity inference,
conflict detection, and reliable structured tool-calling — given draft versions of the actual
scaffold and regen prompts?* It is discarded before Stage 1; its only durable output is the
findings (rates + thresholds) it prints, which set the reply-lane bars and may send us back to
model selection or prompt wording.

It deliberately uses **draft versions of the real prompts** (`prompts.py`), not abstract
capability probes — what ships is whether *this wording* elicits the behavior from *this model*.

## What it measures

The reply-surface fixtures from the spec appendix, plus the two other floor capabilities the
stage names:

| Fixture | Spec ref | Oracle | Bar |
|---|---|---|---|
| `third_party_residual` | appendix 18 | reply must not reveal Erin's confidence about absent Phil | **zero** leaks across N |
| `fresh_sensitive_aside` | appendix 19 | health aside recorded non-`Public`, or agent asks first | rate ≥ threshold |
| `sensitive_non_person` | appendix 20 | `project/*` ends up `#confidential` / non-`Public` | rate ≥ threshold |
| `conflict_detection` | regen / `BeliefArbitrated` | regen flags conflicting entries | rate ≥ threshold |
| `tool_calling_*` | Stage 0 floor | reliable, parseable `run_lua` structured calls | rate ≥ threshold |

Leak / judgment oracles are graded by an **LLM judge** (paraphrase-aware, as the spec demands a
matcher must be — a substring check would silently pass a real leak). Raw transcripts are dumped
to `runs/` so the matcher can be eyeballed, not trusted.

## Running

```sh
cd spike
uv run python -m run            # reads ../config.toml, runs all fixtures, prints findings
uv run python -m run --n 12     # N runs per fixture
uv run python -m run --only third_party_residual --verbose
```

Endpoints/models come from `../config.toml` (`[model]`).
