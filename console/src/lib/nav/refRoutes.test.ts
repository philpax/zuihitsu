import { describe, expect, it } from "vitest";

import {
  rewriteStateUrls,
  rewriteTurnUrls,
  stateHandleFromUrl,
  turnIdFromUrl,
} from "./refRoutes.ts";

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
    const out = rewriteStateUrls(
      "see http://host/state/person%2Frowan.",
      (handle) => (handle === "person/rowan" ? "«minted-token»" : null),
      ["http://host"],
    );
    expect(out).toBe("see «minted-token».");
  });

  it("leaves an unresolved or non-State URL untouched", () => {
    const text = "look at http://host/state/person%2Funknown and http://host/other for context";
    expect(rewriteStateUrls(text, () => null, ["http://host"])).toBe(text);
  });
});

// A console Conversation deep link pins its moment in a `?turn=<id>` query. Recognizing one is route
// matching against the console's own grammar, not a generic query-param sniff — so a foreign URL that
// merely carries a `?turn=` parameter is left alone. The id is not validated here; the caller's wasm
// constructor rejects a malformed one.
describe("turnIdFromUrl", () => {
  it("returns the turn id a Conversation deep link pins, under each URL grammar", () => {
    // Embedded build: the stream sits at the root.
    expect(turnIdFromUrl("http://localhost:7777/conversation?turn=abc")).toBe("abc");
    // Live agent frame, alongside another query parameter and a fragment.
    expect(turnIdFromUrl("https://host/live/conversation?seq=4&turn=xyz#foot")).toBe("xyz");
    // Eval run frame, with a selected room.
    expect(turnIdFromUrl("https://host/eval/greeting/2/conversation/climbing?turn=def")).toBe(
      "def",
    );
  });

  it("returns null for anything that is not a Conversation deep link with a turn", () => {
    for (const url of [
      "https://host/live/conversation", // no turn parameter
      "https://host/live/state/person%2Frowan?turn=abc", // a State route, not Conversation
      "https://example.com/article?turn=abc", // a foreign path that merely carries ?turn=
      "https://host/trends",
      "not a url at all",
    ]) {
      expect(turnIdFromUrl(url)).toBeNull();
    }
  });
});

describe("rewriteTurnUrls", () => {
  it("rewrites a resolved Conversation link to its token and keeps trailing punctuation", () => {
    const out = rewriteTurnUrls(
      "see http://host/live/conversation?turn=good.",
      (id) => (id === "good" ? "«minted-token»" : null),
      ["http://host"],
    );
    expect(out).toBe("see «minted-token».");
  });

  it("leaves a malformed id or a foreign ?turn= URL untouched", () => {
    // The malformed id is recognized as the pinned moment but declined by the minter (as the wasm
    // constructor would), so the link stays prose; the foreign URL is never recognized at all.
    const text =
      "bad http://host/live/conversation?turn=BAD and http://example.com/x?turn=good end";
    expect(
      rewriteTurnUrls(text, (id) => (id === "good" ? "«minted-token»" : null), ["http://host"]),
    ).toBe(text);
  });
});

// The rewrite that replaces a URL with a token gates on origin, since route matching alone ignores host:
// a deep link is rewritten only on an origin the console owns (its own, or its configured backend), so a
// foreign URL that merely shares the console's path shape is left as prose rather than destroyed.
describe("rewrite origin gating", () => {
  const mintState = (handle: string) => (handle === "person/rowan" ? "«token»" : null);
  const mintTurn = (id: string) => (id === "good" ? "«token»" : null);

  it("rewrites a State link on the page's own origin", () => {
    expect(
      rewriteStateUrls("at http://host/state/person%2Frowan here", mintState, ["http://host"]),
    ).toBe("at «token» here");
  });

  it("rewrites a link on the configured backend origin (the cross-origin dev console)", () => {
    // The page runs on one origin and reaches its backend on another (the dev console against :7878);
    // a deep link on the backend origin is the console's own and is rewritten.
    const origins = ["http://console.local", "http://localhost:7878"];
    expect(
      rewriteStateUrls(
        "at http://localhost:7878/live/state/person%2Frowan here",
        mintState,
        origins,
      ),
    ).toBe("at «token» here");
    expect(
      rewriteTurnUrls(
        "at http://localhost:7878/live/conversation?turn=good end",
        mintTurn,
        origins,
      ),
    ).toBe("at «token» end");
  });

  it("leaves a foreign origin with a console-shaped path untouched", () => {
    // The handle would resolve and the path matches a State route, but the origin is not the console's,
    // so the URL is left as prose rather than rewritten into a token.
    const foreign = "see https://othersite.io/live/state/person%2Frowan for context";
    expect(rewriteStateUrls(foreign, mintState, ["http://host"])).toBe(foreign);
    const foreignTurn = "see https://othersite.io/live/conversation?turn=good for context";
    expect(rewriteTurnUrls(foreignTurn, mintTurn, ["http://host"])).toBe(foreignTurn);
  });
});
