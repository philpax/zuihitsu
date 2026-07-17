import { describe, expect, it } from "vitest";

import { STREAM_VIEWS, AGENT_VIEW_IDS, SELECTION_VIEWS, type ViewId } from "./streamViews.ts";
import {
  type AppLocation,
  type Mode,
  type StreamSearch,
  type StreamView,
  buildPath,
  parsePath,
  streamLocation,
} from "./location.ts";

// The router's correctness rests on one property: `parsePath` and `buildPath` are inverse over
// canonical URLs. Rather than sample it, enumerate the whole discrete space — every frame, every view,
// selection present and absent, and a spread of search shapes (including the encoding-tricky ones) —
// and assert the round-trip holds for each. If the codec is a bijection, routing is correct.

const SEARCHES: StreamSearch[] = [
  {},
  { seq: 0 },
  { seq: 42 },
  { turn: "01HTURN" },
  { focus: "person/rowan" },
  { relations: "origin,part_of", sameAs: "off", expand: "cluster (3)" },
  { seq: 7, turn: "01HTURN", focus: "self" },
];

// Selection values chosen to exercise encoding: a slash, the room separator with spaces, a plain slug.
const SELECTIONS = ["person/rowan", "direct · rowan@x", "environment"];

/// Every stream view worth round-tripping for a frame that admits `views`: each view bare, and each
/// selection-bearing view opened on each tricky selection — every one crossed with every search.
function streamsFor(views: readonly ViewId[]): StreamView[] {
  const out: StreamView[] = [];
  for (const view of views) {
    for (const search of SEARCHES) {
      out.push({ view, search });
      // Drawn from the codec's own set, so a new selection view cannot silently shrink this coverage.
      if (SELECTION_VIEWS.has(view)) {
        for (const selection of SELECTIONS) out.push({ view, selection, search });
      }
    }
  }
  return out;
}

const STREAM_VIEW_IDS = STREAM_VIEWS.map((entry) => entry.id);
const ALL_VIEW_IDS: ViewId[] = [...STREAM_VIEW_IDS, ...AGENT_VIEW_IDS];

/// Every location worth round-tripping, paired with the mode its grammar lives in.
function allLocations(): { location: AppLocation; mode: Mode }[] {
  const cases: { location: AppLocation; mode: Mode }[] = [];
  const console = (location: AppLocation) => cases.push({ location, mode: "console" });

  console({ kind: "landing" });
  console({ kind: "trends" });
  console({ kind: "evalOverview" });

  // Eval runs carry only the stream views; the scenario name is free-form (so, encoding-tricky).
  for (const scenario of ["scenario-a", "scn/with slash", "a b"]) {
    for (const run of [0, 3, 12]) {
      for (const stream of streamsFor(STREAM_VIEW_IDS)) {
        console(streamLocation({ kind: "evalRun", scenario, run }, stream));
      }
    }
  }

  // The live and embedded frames admit the agent-only views too.
  for (const stream of streamsFor(ALL_VIEW_IDS)) console(streamLocation({ kind: "live" }, stream));
  for (const stream of streamsFor(ALL_VIEW_IDS)) {
    cases.push({ location: streamLocation({ kind: "embedded" }, stream), mode: "embedded" });
  }

  return cases;
}

describe("the location codec", () => {
  it("round-trips every location through its canonical URL", () => {
    for (const { location, mode } of allLocations()) {
      const path = buildPath(location);
      expect(parsePath(path, mode), `${JSON.stringify(location)} → ${path}`).toEqual(location);
    }
  });

  it("keeps a slashed or spaced segment a single path segment", () => {
    expect(
      buildPath(
        streamLocation({ kind: "live" }, { view: "state", selection: "person/rowan", search: {} }),
      ),
    ).toBe("/live/state/person%2Frowan");
    expect(
      buildPath(
        streamLocation(
          { kind: "evalRun", scenario: "a b", run: 1 },
          { view: "events", search: { focus: "m" } },
        ),
      ),
    ).toBe("/eval/a%20b/1/events?focus=m");
  });

  it("tolerates non-canonical inbound URLs, defaulting the bare frame to the conversation", () => {
    expect(parsePath("/live", "console")).toEqual(
      streamLocation({ kind: "live" }, { view: "conversation", search: {} }),
    );
    expect(parsePath("/", "embedded")).toEqual(
      streamLocation({ kind: "embedded" }, { view: "conversation", search: {} }),
    );
  });

  it("rejects a URL that names no reachable place", () => {
    expect(parsePath("/nonsense", "console")).toBeNull();
    expect(parsePath("/eval/scn/not-a-number/state", "console")).toBeNull();
    expect(parsePath("/live/events/stray-selection", "console")).toBeNull(); // Events takes no selection
    expect(parsePath("/live/settings", "console")).toEqual(
      streamLocation({ kind: "live" }, { view: "settings", search: {} }),
    );
    expect(parsePath("/settings", "console")).toBeNull(); // agent views are not top-level in the console
  });

  it("drops a malformed cursor rather than admitting a fractional or negative seq", () => {
    for (const seq of ["-1", "1.5", "abc"]) {
      expect(parsePath(`/live/state/self?seq=${seq}`, "console")).toEqual(
        streamLocation({ kind: "live" }, { view: "state", selection: "self", search: {} }),
      );
    }
  });
});
