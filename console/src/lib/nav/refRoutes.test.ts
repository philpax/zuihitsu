import { describe, expect, it } from "vitest";

import { rewriteStateUrls, stateHandleFromUrl } from "./refRoutes.ts";

// Route matching is the frontend's own concern: a console State-view deep link is unprojected back to
// the memory handle it names, through the console's own route codec — no token-syntax parsing.
describe("stateHandleFromUrl", () => {
  it("returns the handle a State deep link names, under each URL grammar", () => {
    // Embedded build: the stream sits at the root.
    expect(stateHandleFromUrl("http://localhost:7777/state/context%2Fdiscord%3Adm%2F123")).toBe(
      "context/discord:dm/123",
    );
    // Live agent frame.
    expect(stateHandleFromUrl("https://host/live/state/person%2Frowan?seq=4")).toBe("person/rowan");
    // Eval run frame.
    expect(stateHandleFromUrl("https://host/eval/greeting/2/state/topic%2Fclimbing")).toBe(
      "topic/climbing",
    );
  });

  it("returns null for anything that is not a State deep link", () => {
    for (const url of [
      "https://host/live/conversation?turn=abc",
      "https://host/state", // no selection segment
      "https://host/trends",
      "not a url at all",
    ]) {
      expect(stateHandleFromUrl(url)).toBeNull();
    }
  });
});

describe("rewriteStateUrls", () => {
  it("rewrites a resolved State link to its token and keeps trailing punctuation", () => {
    const out = rewriteStateUrls("see http://host/state/person%2Frowan.", (handle) =>
      handle === "person/rowan" ? "«minted-token»" : null,
    );
    expect(out).toBe("see «minted-token».");
  });

  it("leaves an unresolved or non-State URL untouched", () => {
    const text = "look at http://host/state/person%2Funknown and http://host/other for context";
    expect(rewriteStateUrls(text, () => null)).toBe(text);
  });
});
