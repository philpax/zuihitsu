import type { Message } from "../../types/Message.ts";
import type { ModelInteraction } from "./interactions.ts";

/// A minimal `ModelInteraction` for the derivation tests, defaulting to a lone base call with a
/// two-section system prompt. Fixtures only — never imported by view code.
export function call(overrides: Partial<ModelInteraction>): ModelInteraction {
  return {
    seq: 1,
    conversation: "conv",
    turnId: "turn",
    phase: "Step",
    system: "scaffold\n\n# Current time\n\nnow.",
    systemSections: [],
    messages: [],
    appendedFrom: 0,
    record: "base",
    tools: [],
    completion: { Reply: "ok" },
    reasoning: null,
    finishReason: null,
    usage: {
      prompt_tokens: null,
      completion_tokens: null,
      total_tokens: null,
      cache_read_tokens: null,
      cache_write_tokens: null,
    },
    durationMs: 0,
    ...overrides,
  };
}

export function message(role: Message["role"], content: string): Message {
  return { role, content, tool_calls: [], tool_call_id: null };
}
