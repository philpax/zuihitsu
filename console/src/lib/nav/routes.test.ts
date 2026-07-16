import { describe, expect, it } from "vitest";

import {
  conversationPath,
  eventsPath,
  runBase,
  runPath,
  settingsPath,
  statePath,
  viewPath,
} from "./routes.ts";

// The builders are the one place a stream URL's shape is decided, so this pins the two things that
// break silently: a free-form segment (a memory, a room key) must be percent-encoded so a slash or a
// separator stays a single path segment, and the cursor must ride the search only when pinned.

describe("the stream link builders", () => {
  it("encodes a memory with a slash into a single State segment, cursor in search", () => {
    expect(statePath("/live", "person/rowan", 3)).toEqual({
      to: "/live/state/person%2Frowan",
      search: { seq: 3 },
    });
  });

  it("drops the cursor from search when the view follows its head", () => {
    expect(statePath("/live", "self", null)).toEqual({ to: "/live/state/self", search: {} });
    expect(viewPath("/live", "events")).toEqual({ to: "/live/events", search: {} });
  });

  it("encodes a room key's spaces and separator, carrying a turn highlight and cursor", () => {
    expect(conversationPath("/eval/scn/0", { room: "direct · rowan", turn: "T1", seq: 2 })).toEqual(
      {
        to: "/eval/scn/0/conversation/direct%20%C2%B7%20rowan",
        search: { turn: "T1", seq: 2 },
      },
    );
  });

  it("builds a bare, roomless conversation deep link — the turn resolves the room", () => {
    expect(conversationPath("/live", { turn: "T9" })).toEqual({
      to: "/live/conversation",
      search: { turn: "T9" },
    });
  });

  it("builds correct paths off the embedded build's empty base", () => {
    expect(conversationPath("", { room: "operator · imprint" })).toEqual({
      to: "/conversation/operator%20%C2%B7%20imprint",
      search: {},
    });
    expect(viewPath("", "state", 4)).toEqual({ to: "/state", search: { seq: 4 } });
  });

  it("pins the Events view to a memory's events", () => {
    expect(eventsPath("/live", { focus: "mem/1", seq: 7 })).toEqual({
      to: "/live/events",
      search: { focus: "mem/1", seq: 7 },
    });
  });

  it("builds the Settings tab and the eval-run paths, encoding the scenario name", () => {
    expect(settingsPath("/live", "environment")).toEqual({ to: "/live/settings/environment" });
    expect(runBase("a scenario", 1)).toBe("/eval/a%20scenario/1");
    expect(runPath("a scenario", 1, "events")).toEqual({
      to: "/eval/a%20scenario/1/events",
      search: {},
    });
  });
});
