---
description: Run the eval suite — discover scenarios, determine scope and rounds with the operator, then run to completion with no timeout so it can fully finish.
---

# Running an eval

This skill runs the zuihitsu eval suite against the configured local model. The eval drives a local
inference server, so it needs a GPU; it is kept out of `cargo test`.

## Step 1 — discover what's available

**Always** run `eval list` first to see the current scenarios, their categories, bars, and
descriptions:

```
cargo run -p zuihitsu-eval --bin eval -- list
```

Do not assume the scenario names from memory — the corpus grows over time. Read the output and use
it to inform the scope discussion with the operator.

## Step 2 — infer scope from context

Before asking, look for signals in the conversation that narrow what to run:

- **A code change was just made** — what did it touch?
  - If it's a narrow, self-contained change (one scenario's behavior), propose just that scenario.
  - If it's a broad change (a refactor, a prompt edit, a shared path), the full suite is the
    safe choice — but **ask before running it**, because a full suite at high N can take hours of
    GPU time.
- **The operator said "quick"** → `--runs 1`, scoped to the touched area.
- **The operator said "full eval"** → all scenarios, `--runs 20` — but **confirm before running**.
- **A specific scenario was named** → use `--scenario <substring>`.

If the context makes scope and rounds unambiguous and low-cost (e.g. the operator explicitly said
"quick, one scenario, N=1"), skip the questions and go straight to the confirmation in step 4.

## Step 3 — ask (when scope is not obvious)

If step 2 did not fully resolve the scope, use `ask_user_question` with up to two questions:

1. **Scope** — which scenarios to run. Frame the options from the `eval list` output. Options:
   - "Full suite" — every scenario from `eval list`.
   - "Scoped to the change" — only the scenarios implied by the recent change (name them).
   - Free-text — the operator names scenarios or substrings.
2. **Rounds** — how many runs per scenario. Options:
   - "N=1 (quick)" — a fast check, one run per scenario.
   - "N=20 (full)" — the statistical N for a real measurement.
   - Free-text — the operator names a number.

## Step 4 — confirm parameters before running

Use `ask_user_question` to show the exact command and ask for confirmation. The explainer must
include:

- The **name** of the run (a bare filename, no path or extension). Prefix it with today's date in
  `YYYY-MM-DD` form, then a descriptor derived from the change being evaluated — e.g.
  `2026-07-12-post-god-class-refactor`, `2026-07-12-tag-privacy-fix`, `2026-07-12-full-n10`. If the
  descriptor has no context, ask.
- The **scenarios** (all, or the `--scenario` filter with substrings).
- The **runs** (`--runs N`).

The command shape:

```
cargo run -p zuihitsu-eval --bin eval -- run --name <name> --runs <N> [--scenario <substrings>]
```

Serving is always on by default at `127.0.0.1:7878` — the console fills in live as runs complete.
Do not pass `--no-serve`. The config is at the repo root (`config.toml` is the default) — do not
pass `--config`.

## Step 5 — run with no timeout

The eval must run to completion. Run it as a background job or with a very large `timeout_seconds`
(at least 7200 for a full suite). If the tool's timeout elapses, the process continues as a
background job — poll it with `job_status` / `job_block` until it finishes, then retrieve the
result with `job_result`. **Never** treat a timeout as completion — a partial eval package is
useless for gating.

The eval writes `eval/<name>.jsonl` as it goes and `eval/<name>.json` on completion. The `.jsonl`
is resumable: if the run was interrupted, re-run with `--resume` to continue from where it left
off.

## Step 6 — report the result

The eval's **exit code is the gating signal**:

- **Exit 0** — every gating oracle held across all runs. Report success.
- **Exit non-zero** — at least one gating oracle regressed. Report the failure and offer to run the
  analysis:

  ```
  cargo run -p zuihitsu-eval --bin eval -- analyze eval/<name>.json -f
  ```

  The `-f` / `--failures` flag dumps the failed runs' complete deliberation traces — the agent's
  reasoning, the Lua it ran, and what came back — so the operator can see exactly what broke.

Report per-scenario gating status. If the `.json` was not written (process killed before
completion), parse the `.jsonl` to extract per-run `gating_passed` fields and report those.
