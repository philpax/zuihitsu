import { describe, expect, it } from "vitest";

import type { ConversationModel } from "../../lib/model/conversation.ts";
import { type Channel, mostRecentlyUpdated } from "./channelUtilities.ts";

/// A channel carrying a conversation whose turns hold the given seqs — only what
/// `mostRecentlyUpdated` reads (each turn's `seq`, via `lastActivity`).
function channel(key: string, turnSeqs: number[] | null): Channel {
  const conversation =
    turnSeqs === null
      ? null
      : ({ turns: turnSeqs.map((seq) => ({ seq })) } as unknown as ConversationModel);
  return {
    key,
    label: key,
    locator: { platform: "test", scope_path: key },
    authority: "participant",
    conversation,
  };
}

describe("mostRecentlyUpdated", () => {
  it("picks the conversation whose latest turn has the highest seq", () => {
    const channels = [channel("a", [1, 2, 3]), channel("c", [4, 30, 12]), channel("b", [10, 11])];
    expect(mostRecentlyUpdated(channels)?.key).toBe("c");
  });

  it("ignores channels with no folded conversation (pending or placeholder rooms)", () => {
    const channels = [channel("pending", null), channel("real", [5, 6])];
    expect(mostRecentlyUpdated(channels)?.key).toBe("real");
  });

  it("returns null when no channel carries a conversation", () => {
    expect(mostRecentlyUpdated([channel("pending", null)])).toBeNull();
  });

  it("returns null for an empty channel list", () => {
    expect(mostRecentlyUpdated([])).toBeNull();
  });

  it("treats a conversation with no turns as least recent, not a match over a live one", () => {
    const channels = [channel("empty", []), channel("live", [1])];
    expect(mostRecentlyUpdated(channels)?.key).toBe("live");
  });
});
