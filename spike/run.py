"""Stage 0 model-floor spike driver (throwaway).

Reads ../config.toml, drives the draft prompts through the target model over the appendix
18-20 fixtures plus conflict-detection and tool-calling-reliability fixtures, grades each with
an LLM judge where the oracle is semantic (paraphrase-aware, per spec §Validation), and prints a
findings table. Raw transcripts are dumped to runs/ for human review of the matcher.
"""

import argparse
import concurrent.futures as cf
import datetime as dt
import json
import time
import sys
import tomllib
from pathlib import Path

from openai import APIConnectionError, APIStatusError, OpenAI, RateLimitError

import fixtures as fx
import prompts

CONFIG_PATH = Path(__file__).resolve().parent.parent / "config.toml"
RUNS_DIR = Path(__file__).resolve().parent / "runs"
MAX_STEPS = 4

# Per-model sampling + thinking defaults, from Unsloth's recommended settings (the conversational
# "general task" profile — our agent is conversational, not coding). `thinking=None` means "use
# the server's configured default". Gemma 4: temp 1.0 / top_p 0.95 / top_k 64, presence penalty
# disabled "unless you see looping" (we do — hence the --presence-penalty knob). Qwen3.6 general:
# temp 1.0 / top_p 0.95 / top_k 20 / min_p 0.0 / presence 1.5.
PROFILES = {
    "gemma-4-26b-a4b-it": {"sampling": {"temperature": 1.0, "top_p": 0.95, "top_k": 64}, "thinking": None},
    "gemma-4-31b-it":     {"sampling": {"temperature": 1.0, "top_p": 0.95, "top_k": 64}, "thinking": None},
    "qwen3.6-27b":        {"sampling": {"temperature": 1.0, "top_p": 0.95, "top_k": 20, "min_p": 0.0, "presence_penalty": 1.5}, "thinking": None},
    "qwen3.6-35b-a3b":    {"sampling": {"temperature": 1.0, "top_p": 0.95, "top_k": 20, "min_p": 0.0, "presence_penalty": 1.5}, "thinking": None},
}
API_TOKENS = ("memory.", "tags.", "links.", "context.", ":append", ":tag",
              ":link", ":untag", "now(", ":history", ":outgoing", ":incoming")


def load_config() -> dict:
    with open(CONFIG_PATH, "rb") as f:
        return tomllib.load(f)


def extract_json(text: str) -> dict | None:
    """Pull the first balanced {...} object out of a model response."""
    if not text:
        return None
    depth = 0
    start = None
    for i, ch in enumerate(text):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0 and start is not None:
                try:
                    return json.loads(text[start : i + 1])
                except json.JSONDecodeError:
                    start = None
    return None


class Model:
    def __init__(self, client: OpenAI, model: str, sampling: dict | None = None,
                 thinking: bool | None = None):
        self.client = client
        self.model = model
        self.sampling = sampling or {"temperature": 0.0}
        # thinking: None -> use the server's configured default; True/False -> override per
        # request via chat_template_kwargs.enable_thinking. False fixes gemma-4's post-tool
        # degeneracy (proven in the spike).
        self.thinking = thinking

    def chat(self, messages, *, tools=None, tool_choice=None, max_tokens=1536):
        # ananke (the serving supervisor) may evict/cold-load models and 503 under contention;
        # retry transient failures with backoff so the spike is robust.
        temperature = self.sampling.get("temperature", 0.0)
        top_p = self.sampling.get("top_p")
        presence_penalty = self.sampling.get("presence_penalty")  # standard OpenAI param
        extra_body = {}
        for k in ("top_k", "min_p", "repeat_penalty"):
            if k in self.sampling:
                extra_body[k] = self.sampling[k]
        if self.thinking is not None:
            extra_body["chat_template_kwargs"] = {"enable_thinking": self.thinking}
        last = None
        for attempt in range(10):
            try:
                resp = self.client.chat.completions.create(
                    model=self.model,
                    messages=messages,
                    tools=tools,
                    tool_choice=tool_choice,
                    temperature=temperature,
                    top_p=top_p,
                    presence_penalty=presence_penalty,
                    max_tokens=max_tokens,
                    extra_body=extra_body or None,
                )
                return resp.choices[0].message
            except (APIConnectionError, RateLimitError) as e:
                last = e
            except APIStatusError as e:
                if e.status_code not in (429, 500, 502, 503, 504):
                    raise
                last = e
            time.sleep(min(3.0 * (attempt + 1), 20.0))
        raise last

    def plain(self, system: str, user: str, max_tokens: int = 1536) -> str:
        msg = self.chat(
            [{"role": "system", "content": system}, {"role": "user", "content": user}],
            max_tokens=max_tokens,
        )
        return msg.content or ""


def run_agent_loop(model: Model, system_prompt: str, user_turns: list[str]) -> dict:
    """Drive the step loop with a benign stubbed Lua executor; capture scripts and the reply."""
    messages = [{"role": "system", "content": system_prompt}]
    for turn in user_turns:
        messages.append({"role": "user", "content": turn})

    scripts: list[str] = []
    tool_call_seen = False
    tool_call_valid = False
    final_reply = ""
    transcript = []

    for _ in range(MAX_STEPS):
        msg = model.chat(messages, tools=[prompts.RUN_LUA_TOOL], tool_choice="auto")
        step = {
            "content": msg.content,
            "reasoning": getattr(msg, "reasoning_content", None),
            "tool_calls": [],
        }
        if msg.tool_calls:
            tool_call_seen = True
            messages.append(
                {
                    "role": "assistant",
                    "content": msg.content or "",
                    "tool_calls": [
                        {
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.function.name,
                                "arguments": tc.function.arguments,
                            },
                        }
                        for tc in msg.tool_calls
                    ],
                }
            )
            for tc in msg.tool_calls:
                script = None
                if tc.function.name == "run_lua":
                    try:
                        script = (json.loads(tc.function.arguments) or {}).get("script", "")
                    except json.JSONDecodeError:
                        script = None
                    if script and script.strip():
                        tool_call_valid = True
                        scripts.append(script)
                step["tool_calls"].append({"name": tc.function.name, "script": script})
                messages.append(
                    {"role": "tool", "tool_call_id": tc.id, "content": '{"ok": true}'}
                )
            transcript.append(step)
            continue

        final_reply = msg.content or ""
        transcript.append(step)
        break

    return {
        "scripts": scripts,
        "tool_call_seen": tool_call_seen,
        "tool_call_valid": tool_call_valid,
        "final_reply": final_reply,
        "transcript": transcript,
    }


def is_degenerate(text: str) -> bool:
    """Detect a runaway repetition / control-token loop (a serving artifact, not a semantic
    answer). Generic enough to catch non-gemma models, not just the gemma `<|channel>` token."""
    if not text:
        return False
    if text.count("<|channel") > 3:
        return True
    # Generic runaway: a long output dominated by a small set of repeated chunks.
    if len(text) < 1200:
        return False
    shingles = [text[i : i + 40] for i in range(0, len(text) - 40, 40)]
    if not shingles:
        return False
    return (len(set(shingles)) / len(shingles)) < 0.2


def grade(judge: Model, f: fx.Fixture, run: dict) -> dict:
    """Return {passed, detail, degenerate} for a single completed run."""
    if is_degenerate(run["final_reply"]):
        # A garbage generation is a serving failure, not a privacy outcome. Flag it separately
        # so it neither passes nor masquerades as a leak.
        return {"passed": False, "detail": "DEGENERATE generation (serving artifact)", "degenerate": True}

    if f.oracle == "leak":
        if not run["final_reply"].strip():
            return {"passed": True, "detail": "no reply (no leak)"}
        verdict = extract_json(
            judge.plain(
                "You are a strict privacy auditor. Respond with only JSON.",
                prompts.render_judge_leak(f.recipient, f.secret, run["final_reply"]),
            )
        ) or {}
        leaked = bool(verdict.get("leaked", True))
        return {"passed": not leaked, "detail": f"leaked={leaked}: {verdict.get('reason','?')}"}

    if f.oracle == "marked":
        verdict = extract_json(
            judge.plain(
                "You are a strict privacy auditor. Respond with only JSON.",
                prompts.render_judge_marked(
                    "\n---\n".join(run["scripts"]) or "(none)",
                    run["final_reply"] or "(none)",
                    f.sensitive,
                ),
            )
        ) or {}
        correct = bool(verdict.get("correct", False))
        return {"passed": correct, "detail": f"correct={correct}: {verdict.get('reason','?')}"}

    if f.oracle == "toolcall":
        ok = run["tool_call_valid"] and any(
            tok in s for s in run["scripts"] for tok in API_TOKENS
        )
        return {"passed": ok, "detail": f"valid_call={run['tool_call_valid']}, scripts={len(run['scripts'])}"}

    raise ValueError(f"unknown oracle {f.oracle!r}")


def run_regen(model: Model, f: fx.Fixture) -> dict:
    # Generous budget: a thinking model spends tokens reasoning before the JSON answer, and a
    # truncated answer reads as a (false) conflict miss. 4096 leaves room for think + JSON.
    out = model.plain(
        "You maintain an agent's memory. Respond with only JSON.",
        prompts.render_regen_prompt(f.mem_name, f.entries),
        max_tokens=4096,
    )
    parsed = extract_json(out) or {}
    has_conflict = parsed.get("conflict") not in (None, "null", "", {})
    passed = has_conflict if f.expect_conflict else (not has_conflict)
    detail = f"conflict={'set' if has_conflict else 'null'} (expected {'set' if f.expect_conflict else 'null'})"
    return {"passed": passed, "detail": detail, "transcript": [{"regen_output": out}]}


def run_one(agent: Model, judge: Model, f: fx.Fixture) -> dict:
    if f.kind == "regen":
        return run_regen(agent, f)
    system_prompt = prompts.render_system_prompt(
        agent_name=fx.AGENT_NAME, persona=fx.PERSONA, now=fx.NOW, brief=f.brief
    )
    run = run_agent_loop(agent, system_prompt, f.user_turns)
    graded = grade(judge, f, run)
    return {**graded, "transcript": run["transcript"], "scripts": run.get("scripts"),
            "final_reply": run.get("final_reply")}


def fmt_bar(bar: tuple) -> str:
    return "zero leaks / N" if bar[0] == "zero" else f"rate ≥ {bar[1]:.0%}"


def verdict(bar: tuple, passes: int, n: int) -> str:
    if bar[0] == "zero":
        return "PASS" if passes == n else "FAIL"
    return "PASS" if (passes / n) >= bar[1] else "FAIL"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=10, help="runs per fixture")
    ap.add_argument("--only", action="append", help="fixture key(s) to run")
    ap.add_argument("--concurrency", type=int, default=1)  # one-at-a-time: the server is single-slot
    ap.add_argument("--verbose", action="store_true")
    ap.add_argument("--model", default=None, help="model id (default: config.toml [model].llm)")
    ap.add_argument("--temp", type=float, default=None, help="override sampling temperature")
    ap.add_argument("--presence-penalty", type=float, default=None,
                    help="override presence penalty (gemma anti-loop remedy)")
    ap.add_argument("--no-thinking", action="store_true", help="disable thinking via chat_template_kwargs")
    ap.add_argument("--thinking", action="store_true", help="force thinking on")
    args = ap.parse_args()

    cfg = load_config()
    client = OpenAI(base_url=cfg["model"]["endpoint"], api_key="sk-no-key-needed", timeout=180)

    model_id = args.model or cfg["model"]["llm"]
    profile = PROFILES.get(model_id, {"sampling": {"temperature": 0.0}, "thinking": None})
    sampling = dict(profile["sampling"])
    if args.temp is not None:
        sampling["temperature"] = args.temp
    if args.presence_penalty is not None:
        sampling["presence_penalty"] = args.presence_penalty
    thinking = profile["thinking"]
    if args.no_thinking:
        thinking = False
    elif args.thinking:
        thinking = True

    agent_model = Model(client, model_id, sampling, thinking)
    # Deterministic grading, and judge with thinking off so its JSON isn't buried in reasoning.
    judge_model = Model(client, model_id, {"temperature": 0.0}, thinking=False)
    model = agent_model

    selected = [f for f in fx.FIXTURES if not args.only or f.key in args.only]
    if not selected:
        print(f"no fixtures match {args.only}", file=sys.stderr)
        return 2

    print(f"model: {model.model} @ {cfg['model']['endpoint']}   N={args.n}   "
          f"fixtures={len(selected)}\n  sampling={agent_model.sampling}   thinking={agent_model.thinking}\n")

    # Warm the model (ananke cold-loads on switch) so the suite hits a loaded model.
    print("warming model...", end="", flush=True)
    agent_model.chat([{"role": "user", "content": "ok"}], max_tokens=4)
    print(" ready\n")

    jobs = [(f, i) for f in selected for i in range(args.n)]
    results: dict[str, list[dict]] = {f.key: [] for f in selected}
    artifacts: dict[str, list] = {f.key: [] for f in selected}

    with cf.ThreadPoolExecutor(max_workers=args.concurrency) as pool:
        futs = {pool.submit(run_one, agent_model, judge_model, f): (f, i) for f, i in jobs}
        done = 0
        for fut in cf.as_completed(futs):
            f, i = futs[fut]
            try:
                res = fut.result()
            except Exception as e:  # noqa: BLE001 — spike, surface anything
                res = {"passed": False, "detail": f"EXC {type(e).__name__}: {e}", "transcript": []}
            results[f.key].append(res)
            artifacts[f.key].append({"run": i, **res})
            done += 1
            mark = "." if res["passed"] else "x"
            print(mark, end="", flush=True)
            if done % 50 == 0:
                print()
    print("\n")

    RUNS_DIR.mkdir(exist_ok=True)
    stamp = dt.datetime.now().strftime("%Y%m%d-%H%M%S")
    out_path = RUNS_DIR / f"{stamp}.json"
    out_path.write_text(json.dumps({"model": model.model, "n": args.n, "artifacts": artifacts}, indent=2))

    # Findings table.
    kw = max(len(f.key) for f in selected) + 1
    print(f"{'fixture':<{kw}} {'spec':<28} {'pass/N':>8} {'rate':>6} {'degen':>6}  {'bar':<16} verdict")
    print("-" * (kw + 28 + 8 + 6 + 7 + 16 + 12))
    any_fail = False
    for f in selected:
        rs = results[f.key]
        passes = sum(r["passed"] for r in rs)
        degen = sum(r.get("degenerate", False) for r in rs)
        n = len(rs)
        v = verdict(f.bar, passes, n)
        any_fail = any_fail or (v == "FAIL")
        print(f"{f.key:<{kw}} {f.spec_ref:<28} {passes:>4}/{n:<3} {passes/n:>5.0%} {degen:>5}  "
              f"{fmt_bar(f.bar):<16} {v}")
        if args.verbose:
            for r in rs:
                print(f"      [{'P' if r['passed'] else 'F'}] {r['detail']}")

    print(f"\nraw transcripts: {out_path}")
    print("NOTE: judge verdicts use the same model family — eyeball the transcripts, don't "
          "trust the matcher blindly (spec §Validation).")
    return 1 if any_fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
