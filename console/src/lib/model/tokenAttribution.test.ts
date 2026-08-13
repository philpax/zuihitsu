import { describe, expect, it } from "vitest";

import type { Message } from "@zuihitsu/wire/types/Message.ts";
import { call, message } from "./callFixtures.ts";
import { deriveCachePaths } from "./cachePath.ts";
import type { ModelInteraction } from "./interactions.ts";
import { attributeTokens, estimateTokens } from "./tokenAttribution.ts";

function usage(
  prompt: number | null,
  completion: number | null = null,
  cacheRead: number | null = null,
) {
  return {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: null,
    cache_read_tokens: cacheRead,
    cache_write_tokens: null,
  };
}

function attribution(calls: ModelInteraction[]) {
  return attributeTokens(calls, deriveCachePaths(calls, []));
}

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

describe("attributeTokens", () => {
  it("states the provider's total for the call", () => {
    const calls = [call({ seq: 1, messages: [message("user", "hello")], usage: usage(12_880) })];
    const [only] = attribution(calls);
    expect(only.total).toBe(12_880);
    expect(only.totalProvenance).toBe("measured");
  });

  it("makes the opening prompt one measured block whose parts are shares of it", () => {
    // Nothing separates the system prompt, the tools, and the first message: they arrived together,
    // so each states a ratio and the block states the tokens.
    const calls = [call({ seq: 1, messages: [message("user", "hello")], usage: usage(12_880) })];
    const [only] = attribution(calls);
    const opening = only.blocks.find((block) => block.key === "block:opening");
    expect(opening?.tokens).toBe(12_880);

    for (const row of only.rows) {
      expect(row.cost.kind).toBe("share");
      if (row.cost.kind === "share") expect(row.cost.blockTokens).toBe(12_880);
    }
    const shares = only.rows.map((row) => (row.cost.kind === "share" ? row.cost.percent : 0));
    expect(shares.reduce((a, b) => a + b, 0)).toBeCloseTo(100, 5);
  });

  it("prices a message that arrived alone by the growth it caused", () => {
    // The real shape from an eval run: a turn's reply and the next participant message enter the
    // prompt together, and the cache boundary says how much of the growth was the reply.
    const base = call({
      seq: 1,
      messages: [message("user", "hello")],
      usage: usage(13_569, 36),
    });
    const next = call({
      seq: 2,
      messages: [...base.messages, message("assistant", "a reply"), message("user", "and more")],
      usage: usage(13_669, 118, 13_605),
    });
    const [, second] = attribution([base, next]);

    // 13,605 cached = the prior prompt plus the 36 tokens it generated, so the reply is measured …
    const reply = second.rows.find((row) => row.messageIndex === 1);
    expect(reply?.cost).toEqual({ kind: "measured", tokens: 36 });
    // … and what is left of the 100-token growth is the new message, alone in the remainder.
    const arrived = second.rows.find((row) => row.messageIndex === 2);
    expect(arrived?.cost).toEqual({ kind: "measured", tokens: 64 });
  });

  it("shares a block between messages that arrived together", () => {
    // Without a cache boundary isolating the reply, the growth covers both messages and nothing
    // measures either alone — so the block states 186 and the rows state ratios.
    const base = call({ seq: 1, messages: [message("user", "hello")], usage: usage(13_302, 117) });
    const next = call({
      seq: 2,
      messages: [
        ...base.messages,
        message("assistant", "a reply the model reasoned its way to"),
        message("user", "a message carrying a picture"),
      ],
      usage: usage(13_488, 183, 13_302),
    });
    const [, second] = attribution([base, next]);

    const block = second.blocks.find((candidate) => candidate.key === "block:1");
    expect(block?.tokens).toBe(186);
    for (const index of [1, 2]) {
      const row = second.rows.find((candidate) => candidate.messageIndex === index);
      expect(row?.cost.kind).toBe("share");
    }
    // The completion of 117 tokens is never used to pin the reply: it counts reasoning the prompt
    // does not carry, and pinning to it would rob the message beside it.
    const reply = second.rows.find((row) => row.messageIndex === 1);
    expect(reply?.cost).not.toEqual({ kind: "measured", tokens: 117 });
  });

  it("prices an image when cutting a shared block", () => {
    // The image part serialises to an address and a media type; the model is billed for the picture.
    const shown: Message = {
      ...message("user", "look at this"),
      images: [{ blob: "a".repeat(64), mime: "image/png" }],
    } as Message;
    const base = call({ seq: 1, messages: [message("user", "hello")], usage: usage(1_000) });
    const next = call({
      seq: 2,
      messages: [...base.messages, message("assistant", "a reply of some length here"), shown],
      usage: usage(1_400, null, 1_000),
    });
    const [, second] = attribution([base, next]);
    const image = second.rows.find((row) => row.messageIndex === 2);
    const reply = second.rows.find((row) => row.messageIndex === 1);
    if (image?.cost.kind !== "share" || reply?.cost.kind !== "share") throw new Error("shares");
    expect(image.cost.percent).toBeGreaterThan(reply.cost.percent);
  });

  it("keeps a tool step's messages in the block they entered", () => {
    const base = call({ seq: 1, messages: [message("user", "hello")], usage: usage(12_880) });
    const next = call({
      seq: 2,
      record: "continuation",
      messages: [...base.messages, toolCall, toolResult],
      appendedFrom: 1,
      usage: usage(13_044, null, 12_880),
    });
    const [, second] = attribution([base, next]);
    expect(second.blocks.map((block) => block.tokens)).toEqual([12_880, 164]);
    // The opening block still holds the first message; the appended pair holds the rest.
    expect(second.blocks[0].rows).toContain("message:0");
    expect(second.blocks[1].rows).toEqual(["message:1", "message:2"]);
  });

  it("estimates only when the provider reported no usage at all", () => {
    const calls = [call({ seq: 1, messages: [message("user", "hello there")] })];
    const [only] = attribution(calls);
    expect(only.totalProvenance).toBe("estimated");
    expect(only.total).toBeGreaterThan(0);
  });

  it("estimates by Unicode scalar values with the shared ceiling rule", () => {
    expect(estimateTokens("abcd")).toBe(1);
    expect(estimateTokens("abc")).toBe(1);
    expect(estimateTokens("🐚🐚🐚🐚")).toBe(1);
    expect(estimateTokens("")).toBe(0);
  });
});
