import { describe, expect, it } from "vitest";

import {
  CHUNK_TURNS,
  INITIAL_WINDOW_TURNS,
  MAX_WINDOW_TURNS,
  clampWindow,
  followWindow,
  initialWindow,
  pageInEnd,
  pageInStart,
  tailWindow,
  unseenTailCount,
} from "./transcriptWindowUtilities.ts";

describe("initialWindow", () => {
  it("opens on the tail when no turn is focused", () => {
    expect(initialWindow(200, null)).toEqual({ start: 200 - INITIAL_WINDOW_TURNS, end: 200 });
  });

  it("keeps a short transcript whole", () => {
    expect(initialWindow(10, null)).toEqual({ start: 0, end: 10 });
  });

  it("is empty for an empty transcript", () => {
    expect(initialWindow(0, null)).toEqual({ start: 0, end: 0 });
  });

  it("centres a span on a deep-linked turn so history sits either side", () => {
    const win = initialWindow(500, 300);
    expect(win.start).toBe(300 - CHUNK_TURNS);
    expect(win.end).toBe(300 + CHUNK_TURNS + 1);
    expect(300).toBeGreaterThanOrEqual(win.start);
    expect(300).toBeLessThan(win.end);
  });

  it("clamps a deep-linked window at the head of the transcript", () => {
    const win = initialWindow(500, 5);
    expect(win.start).toBe(0);
    expect(win.end).toBe(5 + CHUNK_TURNS + 1);
  });

  it("clamps a deep-linked window at the tail of the transcript", () => {
    const win = initialWindow(500, 498);
    expect(win.end).toBe(500);
    expect(win.start).toBe(498 - CHUNK_TURNS);
  });
});

describe("pageInStart", () => {
  it("extends the window backwards by a chunk", () => {
    expect(pageInStart({ start: 100, end: 150 })).toEqual({ start: 100 - CHUNK_TURNS, end: 150 });
  });

  it("stops at the head of the transcript", () => {
    expect(pageInStart({ start: 10, end: 60 })).toEqual({ start: 0, end: 60 });
  });

  it("pages the tail out once the span would exceed the maximum", () => {
    // A window already at the maximum: extending the head trims the tail (the reader scrolled away
    // from it), so the span stays bounded.
    const win = { start: 40, end: 40 + MAX_WINDOW_TURNS };
    const next = pageInStart(win);
    expect(next.start).toBe(40 - CHUNK_TURNS);
    expect(next.end - next.start).toBe(MAX_WINDOW_TURNS);
    expect(next.end).toBe(next.start + MAX_WINDOW_TURNS);
  });
});

describe("pageInEnd", () => {
  it("extends the window forwards by a chunk toward the tail", () => {
    expect(pageInEnd({ start: 100, end: 150 }, 500)).toEqual({
      start: 100,
      end: 150 + CHUNK_TURNS,
    });
  });

  it("stops at the tail of the transcript", () => {
    expect(pageInEnd({ start: 460, end: 490 }, 500)).toEqual({ start: 460, end: 500 });
  });

  it("pages the head out once the span would exceed the maximum", () => {
    const win = { start: 100, end: 100 + MAX_WINDOW_TURNS };
    const next = pageInEnd(win, 1000);
    expect(next.end).toBe(100 + MAX_WINDOW_TURNS + CHUNK_TURNS);
    expect(next.end - next.start).toBe(MAX_WINDOW_TURNS);
  });
});

describe("followWindow", () => {
  it("snaps the end to the tail and bounds the head", () => {
    expect(followWindow(500)).toEqual({ start: 500 - MAX_WINDOW_TURNS, end: 500 });
  });

  it("keeps a short transcript whole", () => {
    expect(followWindow(20)).toEqual({ start: 0, end: 20 });
  });
});

describe("tailWindow", () => {
  it("is the newest initial-window turns", () => {
    expect(tailWindow(300)).toEqual({ start: 300 - INITIAL_WINDOW_TURNS, end: 300 });
  });
});

describe("clampWindow", () => {
  it("pulls a window back when the turn count shrank under it (a cursor scrub)", () => {
    expect(clampWindow({ start: 180, end: 230 }, 100)).toEqual({ start: 100, end: 100 });
  });

  it("leaves an in-range window untouched", () => {
    expect(clampWindow({ start: 50, end: 100 }, 200)).toEqual({ start: 50, end: 100 });
  });
});

describe("unseenTailCount", () => {
  it("counts the turns past the window's end", () => {
    expect(unseenTailCount({ start: 100, end: 150 }, 172)).toBe(22);
  });

  it("is zero when the window reaches the tail", () => {
    expect(unseenTailCount({ start: 100, end: 200 }, 200)).toBe(0);
  });
});
