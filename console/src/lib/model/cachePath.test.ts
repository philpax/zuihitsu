import { describe, expect, it } from "vitest";

import type { PromptSectionSpan } from "../../types/PromptSectionSpan.ts";
import { call, message } from "./callFixtures.ts";
import { deriveCachePaths } from "./cachePath.ts";

const byteLength = (text: string) => new TextEncoder().encode(text).length;

/// A system prompt whose brief section carries `brief`, with recorded spans.
function sectionedSystem(brief: string): { system: string; spans: PromptSectionSpan[] } {
  const scaffold = "You are a careful assistant.";
  const briefBlock = `\n\n# What you know right now\n\n${brief}`;
  const time = "\n\n# Current time\n\nnow.";
  const system = scaffold + briefBlock + time;
  const scaffoldEnd = byteLength(scaffold);
  const briefEnd = scaffoldEnd + byteLength(briefBlock);
  return {
    system,
    spans: [
      { kind: "Scaffold", start: 0, end: scaffoldEnd },
      { kind: "Brief", start: scaffoldEnd, end: briefEnd },
      { kind: "CurrentTime", start: briefEnd, end: briefEnd + byteLength(time) },
    ],
  };
}

describe("deriveCachePaths", () => {
  it("marks an append-only continuation chain warm after the cold base (AC3.1)", () => {
    const base = call({
      seq: 1,
      messages: [message("user", "hello")],
      usage: { ...call({}).usage, prompt_tokens: 100 },
    });
    const chain = [base];
    for (let i = 0; i < 3; i += 1) {
      const prior = chain[chain.length - 1];
      chain.push(
        call({
          seq: prior.seq + 1,
          record: "continuation",
          messages: [...prior.messages, message("assistant", `step ${i}`)],
          appendedFrom: prior.messages.length,
        }),
      );
    }
    const verdicts = deriveCachePaths(chain, []);
    expect(verdicts.map((verdict) => verdict.path)).toEqual(["cold", "warm", "warm", "warm"]);
    expect(verdicts[0].cause).toBe("first-call");
  });

  it("attributes a brief edit to the Brief section with a char offset (AC3.2)", () => {
    const before = sectionedSystem("## person/staff_engineer\nStaff engineer.");
    const after = sectionedSystem("## person/staff_engineer\nStaff engineer, on leave.");
    const calls = [
      call({ seq: 1, system: before.system, systemSections: before.spans }),
      call({ seq: 2, system: after.system, systemSections: after.spans }),
    ];
    const [, verdict] = deriveCachePaths(calls, []);
    expect(verdict.path).toBe("cold");
    expect(verdict.cause).toBe("system-changed");
    expect(verdict.divergence?.sectionKind).toBe("Brief");
    expect(verdict.divergence?.offset).toBeGreaterThan(0);
  });

  it("attributes a changed tool list (AC3.3)", () => {
    const shared = { system: "same", messages: [message("user", "hi")] };
    const calls = [
      call({ seq: 1, ...shared, tools: [] }),
      call({
        seq: 2,
        ...shared,
        tools: [{ name: "run_lua", description: "run a block", parameters: {} }],
      }),
    ];
    const [, verdict] = deriveCachePaths(calls, []);
    expect(verdict).toMatchObject({ path: "cold", cause: "tools-changed" });
  });

  it("attributes a buffer whose only difference is re-minted tool-call ids", () => {
    // The rebuilt buffer re-renders the same tool exchange with a fresh call id; most templates
    // tokenize little of it, so this must not read as a full rebuild.
    const toolCall = (id: string) => ({
      role: "assistant" as const,
      content: "",
      tool_calls: [{ id, name: "run_lua", arguments: '{"script":"return 1"}' }],
      tool_call_id: null,
    });
    const first = call({ seq: 1, messages: [message("user", "hello"), toolCall("wire-id")] });
    const rebuilt = call({
      seq: 2,
      messages: [message("user", "hello"), toolCall("call_1_0"), message("user", "next")],
    });
    const [, verdict] = deriveCachePaths([first, rebuilt], []);
    expect(verdict).toMatchObject({ path: "cold", cause: "tool-ids-reminted" });
  });

  it("distinguishes a session seam from an unexplained rewrite (AC3.4)", () => {
    const first = call({ seq: 10, messages: [message("user", "one"), message("user", "two")] });
    const rebuilt = call({ seq: 20, messages: [message("user", "carried tail")] });
    const [, seam] = deriveCachePaths([first, rebuilt], [15]);
    expect(seam).toMatchObject({ path: "cold", cause: "new-session" });
    const [, rewrite] = deriveCachePaths([first, rebuilt], []);
    expect(rewrite).toMatchObject({ path: "cold", cause: "buffer-rewritten" });
  });

  it("flags a provider disagreement without reconciling, and never flags unknown (AC3.5)", () => {
    const base = call({ seq: 1, messages: [message("user", "hello")] });
    const warmNoRead = call({
      seq: 2,
      record: "continuation",
      messages: [...base.messages, message("assistant", "hi")],
      appendedFrom: 1,
      usage: { ...call({}).usage, cache_read_tokens: 0 },
    });
    const verdicts = deriveCachePaths([base, warmNoRead], []);
    expect(verdicts[1].path).toBe("warm");
    expect(verdicts[1].providerDisagreement).toBe("no-read-despite-warm");

    const warmUnknown = { ...warmNoRead, usage: { ...warmNoRead.usage, cache_read_tokens: null } };
    expect(deriveCachePaths([base, warmUnknown], [])[1].providerDisagreement).toBeUndefined();

    // A read on a cold call is partial prefix reuse, not a disagreement — a prefix cache serves
    // the longest common prefix even when the buffer was rebuilt.
    const coldWithRead = call({
      seq: 2,
      system: "different",
      usage: { ...call({}).usage, cache_read_tokens: 512 },
    });
    expect(deriveCachePaths([base, coldWithRead], [])[1].providerDisagreement).toBeUndefined();
  });
});
