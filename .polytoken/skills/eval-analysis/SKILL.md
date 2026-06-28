---
description: Analyse an eval package — surface failures first, then scan successful runs for aberrations and misbehaviours the oracles don't catch. Use eval analyze and disposable scripts over the package JSON.
---

# Analysing eval results

An eval package (`eval/<name>.json`) is the agent's complete event log per run, plus verdicts. The goal is not just to confirm pass/fail — it's to understand *how* the agent behaved, whether the oracles caught everything they should, and whether the agent's behaviour matches the system's design intent even when it passes.

## Step 1 — start with the summary and the failures

Read the per-scenario summary first, then dig into the failures:

```
cargo run -p zuihitsu-eval --bin eval -- analyze eval/<name>.json
cargo run -p zuihitsu-eval --bin eval -- analyze eval/<name>.json -f
```

The `-f` (failures) dump starts with a cross-scenario rollup of every missed verdict, grouped by criterion. For each failed run it prints the missed oracles with their rationale, then the complete deliberation: the agent's reasoning, the Lua it ran, and what came back. Useful flags:

- `-s <substring>` — filter to scenarios whose name contains the substring.
- `-e <type>` — add a compact summary of events whose payload type name contains the substring (e.g. `LinkCreated`, `LuaExecuted`, `MemoryContentAppended`).
- `--limit N` — cap the failed runs shown per scenario.
- `--truncate 0` — keep reasoning and scripts whole (the default truncates long ones).

If comparing against a baseline, add `-b eval/<base>.json` to the summary command to see the Δ.

## Step 2 — scan beyond the failures

Passing the evals doesn't mean the agent is behaving as intended — it means the oracles held. The oracles assert specific structural signals (a link was created, a search was called, a reply reflected a fact), but they don't catch every class of misbehaviour. After the failures are understood, scan the *successful* runs for aberrations the oracles missed.

The Rust `analyze` tool answers "what passed, what failed, and what did the agent say?" It does not answer structural questions the oracles don't assert — which links were created and between what kinds of memory, which Lua calls crashed and why, whether the agent misused an API surface in a way that happened to work. Those questions are specific to the behaviour being investigated, so they're best answered with a disposable script over the package JSON rather than a general-purpose flag.

### The package structure

The package is a single JSON object: `meta`, then `scenarios[]`, each with `runs[]`, each with `events[]` and `verdicts[]`. The event payloads are the same `EventPayload` variants the core crate serializes. A script reads them with `json` and a little knowledge of the shapes.

### What to look for

Scan every run — not just the failed ones — for these classes of aberration:

**Lua failures.** A `LuaExecuted` event with a non-null `terminal_cause` is a crashed block. The `script` field shows what the agent tried; the `Error` string shows why. Group these across runs to spot a recurring misuse of an API surface — calling a method that doesn't exist on a returned object, passing the wrong type to a parameter, misusing a date or handle object. A crash the agent recovered from on retry is still a signal: the agent reached for something that wasn't there, which means the API surface or the scaffold guidance has a gap.

**Relation misuse.** Build an id → name map from the run's `MemoryCreated` events, then project each `LinkCreated` as `from_name --relation--> to_name`. Look for relations used between kinds of memory they weren't designed for — a person-to-person relation on an event, compaction plumbing used for attendance, a relation stretched to cover a meaning it wasn't built for. The oracles check whether a link was created under the right relation, but they don't catch a link created under the *wrong* relation that happens to satisfy the oracle's structural check.

**API surface gaps.** When the agent crashes or improvises (coining a new relation, wrapping a value in a table it shouldn't, calling a global where a method is expected), it's reaching for something the system doesn't provide. The crash or improvisation is the symptom; the cause is a missing concept, a missing method, or scaffold guidance that teaches the wrong shape. Trace the agent's intent — what was it trying to do — and consider whether the system should provide it directly.

**Prompt leakage across sessions.** In scenarios with compaction, check whether the message buffer grows unboundedly across sessions — the pre-compaction transcript should be trimmed to a carryover tail, not accumulated. Compare the message count and system-prompt length across sessions within a run; if they grow without bound, the compaction buffer management has a bug.

**Inconsistency across runs.** The same scenario run N times should produce structurally similar behaviour. If one run crashes where another doesn't, or one coins a new relation where another reuses a seed relation, the scaffold guidance is ambiguous enough that the model resolves it differently each time. Consistency is a signal of clear guidance; variance is a signal of a gap.

### A disposable analysis script

The script is throwaway — paste it into a scratch file, adapt it to the question, discard it when done. The point is to answer one question fast, not to build a reusable tool. A template to start from:

```python
import json

with open("eval/<name>.json") as f:
    data = json.load(f)

for scenario in data["scenarios"]:
    for run in scenario["runs"]:
        # Build the id → name map from this run's MemoryCreated events.
        names = {
            e["payload"]["id"]: e["payload"]["name"]
            for e in run["events"]
            if e["payload"]["type"] == "MemoryCreated"
        }
        # Project links to readable form.
        for e in run["events"]:
            p = e["payload"]
            if p["type"] == "LinkCreated":
                fr = names.get(p["from"], p["from"][:8])
                to = names.get(p["to"], p["to"][:8])
                print(f"  {fr} --{p['relation']}--> {to}")
```

Adapt the projection to the question: extract `LuaExecuted` failures, cross-tabulate relations against namespace prefixes, compare message counts across sessions, or whatever the specific aberration calls for.

## Step 3 — distinguish symptoms from causes

When an aberration is found, trace it to its root cause before proposing a fix. The layers, from symptom to cause:

1. **The agent's action** — what it wrote or called. This is what the script surfaces.
2. **The agent's intent** — what it was trying to do. Read the reasoning in the `ModelCalled` event that precedes the `LuaExecuted`.
3. **The guidance gap** — why the agent reached for the wrong thing. Was a concept missing from the vocabulary? Did the scaffold teach the wrong shape? Did the API surface lack a method the agent intuitively reached for?
4. **The systematic issue** — is this a one-off, or does it reflect a design gap that will recur across scenarios? A relation misused for event attendance in one scenario will likely be misused wherever events and people co-occur.

Surface systematic issues to the operator — they may warrant a broader fix than the immediate scenario suggests. But don't over-generalise from a single run; check whether the pattern repeats across runs and scenarios before concluding it's systematic.

## Step 4 — propose fixes that generalise

When proposing a fix, apply these principles:

**Never overfit to the eval.** Don't encode specific facts about a scenario into the prompt or the code. A tweak that makes one scenario pass by naming its entities or hardcoding its logic is worthless — it doesn't generalise and it makes the eval measure nothing. Every change should be generic: it should improve the agent's behaviour for the *class* of situation, not the specific instance.

**Fix the cause, not the symptom.** If the agent misused a relation because the right relation didn't exist, add the relation — don't patch the prompt to steer around the gap. If the agent crashed because a method doesn't exist, consider whether the method should exist or whether the scaffold should teach the alternative — don't just add a warning against calling it.

**Prefer structural fixes to prompt fixes.** A new relation, a new API method, or a rendering change (surfacing descriptions in the prompt) is more robust than scaffold text. The agent reads the registry; it doesn't always read the fine print.

**Keep the three gates in lockstep.** When adding or changing a Lua API feature, the Lua registration, the API reference, and the scaffold dotpoints must all agree. A function installed but not documented is undiscoverable; one documented but not installed is a confusing error.

## Step 5 — consider the analyze tool

If a disposable script answers a question that would be useful every time you analyse an eval — not just this one — consider whether it belongs in the Rust `analyze` tool. But the bar is cohesion: `analyze` can't become a grabbag of one-off analysis scripts. A new flag or filter earns its place when it answers a structural question the oracles systematically miss, in a shape that's useful across scenarios. Link resolution (projecting `from`/`to` ids to names) is a good candidate; a filter that checks whether `event/ --knows--> person/` appeared is too specific.

When in doubt, keep the script disposable. The skill of analysis is knowing which questions to ask, not building tools to ask them.
