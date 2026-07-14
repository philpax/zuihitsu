import { describe, expect, it } from "vitest";

import type { Message } from "@zuihitsu/wire/types/Message.ts";
import { call, message } from "./callFixtures.ts";
import { deriveCachePaths } from "./cachePath.ts";
import type { ModelInteraction } from "./interactions.ts";
import { attributeTokens, estimateTokens } from "./tokenAttribution.ts";

function usage(prompt: number | null, completion: number | null = null) {
  return {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: null,
    cache_read_tokens: null,
    cache_write_tokens: null,
  };
}

/// A base + continuation pair forming a warm chain: the continuation appends the prior assistant
/// reply and a tool result.
function warmChain(basePrompt: number, nextPrompt: number, completion: number | null) {
  const toolCall: Message = {
    role: "assistant",
    content: "",
    tool_calls: [{ id: "call_1", name: "run_lua", arguments: "{}" }],
    tool_call_id: null,
  };
  const toolResult: Message = {
    role: "tool",
    content: "the block returned a value",
    tool_calls: [],
    tool_call_id: "call_1",
  };
  const base = call({
    seq: 1,
    messages: [message("user", "hello")],
    usage: usage(basePrompt, completion),
  });
  const next = call({
    seq: 2,
    record: "continuation",
    messages: [...base.messages, toolCall, toolResult],
    appendedFrom: 1,
    usage: usage(nextPrompt),
  });
  return [base, next];
}

function attribution(calls: ModelInteraction[]) {
  return attributeTokens(calls, deriveCachePaths(calls, []));
}

describe("attributeTokens", () => {
  it("measures a warm continuation's appended slice by the prompt delta (AC4.1)", () => {
    const [, next] = attribution(warmChain(1000, 1180, 120));
    expect(next.total).toBe(1180);
    expect(next.totalProvenance).toBe("measured");

    // The prior completion pins the assistant message; the residue lands on the tool result.
    const assistant = next.rows.find((row) => row.label.startsWith("assistant"));
    expect(assistant).toMatchObject({ tokens: 120, provenance: "measured" });
    const toolResult = next.rows.filter(
      (row) => row.messageIndex !== undefined && row.messageIndex >= 1 && row !== assistant,
    );
    expect(toolResult.reduce((sum, row) => sum + row.tokens, 0)).toBe(60);
    expect(toolResult.every((row) => row.provenance === "apportioned")).toBe(true);

    // The shared prefix is itemized like a base call — sections and earlier messages apportioned
    // within the prior call's measured total, not hidden behind an opaque lump.
    const prefix = next.rows.filter(
      (row) => row.messageIndex === undefined || row.messageIndex < 1,
    );
    expect(prefix.length).toBeGreaterThan(1);
    expect(prefix.reduce((sum, row) => sum + row.tokens, 0)).toBe(1000);
    expect(prefix.every((row) => row.provenance === "apportioned")).toBe(true);
  });

  it("apportions a base lump by char share, summing exactly to the measurement (AC4.2)", () => {
    const base = call({
      seq: 1,
      messages: [message("user", "plan the migration")],
      usage: usage(3112),
    });
    const [only] = attribution([base]);
    expect(only.total).toBe(3112);
    expect(only.rows.reduce((sum, row) => sum + row.tokens, 0)).toBe(3112);
    expect(only.rows.every((row) => row.provenance === "apportioned")).toBe(true);
    // Proportions follow char share: the longer part gets more tokens.
    const [a, b] = [...only.rows].sort((x, y) => y.tokens - x.tokens);
    expect(a.tokens).toBeGreaterThanOrEqual(b.tokens);
  });

  it("falls back to chars/4 estimates when usage is absent (AC4.3)", () => {
    const base = call({ seq: 1, messages: [message("user", "hello there")] });
    const [only] = attribution([base]);
    expect(only.totalProvenance).toBe("estimated");
    expect(only.rows.every((row) => row.provenance === "estimated")).toBe(true);
    expect(only.total).toBe(only.rows.reduce((sum, row) => sum + row.tokens, 0));
  });

  it("always displays the provider's total when reported (AC4.4)", () => {
    const sequences = [
      warmChain(1000, 1180, 120),
      warmChain(50, 51, null),
      [call({ seq: 1, usage: usage(7) })],
      [call({ seq: 1, usage: usage(999) }), call({ seq: 2, system: "changed", usage: usage(3) })],
    ];
    for (const calls of sequences) {
      const attributions = attribution(calls);
      calls.forEach((one, i) => {
        if (one.usage.prompt_tokens !== null) {
          expect(attributions[i].total).toBe(one.usage.prompt_tokens);
          expect(attributions[i].rows.reduce((sum, row) => sum + row.tokens, 0)).toBe(
            one.usage.prompt_tokens,
          );
        }
      });
    }
  });

  it("drops to estimates on a non-positive delta rather than showing negative rows", () => {
    const [, next] = attribution(warmChain(1000, 900, 120));
    expect(next.totalProvenance).toBe("estimated");
    expect(next.rows.every((row) => row.tokens >= 0)).toBe(true);
  });

  it("estimates by code points, mirroring the agent's chars/4", () => {
    expect(estimateTokens("abcd")).toBe(1);
    expect(estimateTokens("🐚🐚🐚🐚")).toBe(1);
    expect(estimateTokens("")).toBe(0);
  });
});
