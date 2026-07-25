import { describe, expect, it } from "vitest";

import { buildPageList } from "./pagerUtilities.ts";

describe("buildPageList", () => {
  const radius = 3;
  const slots = 2 * radius + 5;

  it("lists every page when they all fit", () => {
    expect(buildPageList(0, 5, radius)).toEqual([0, 1, 2, 3, 4]);
    expect(buildPageList(4, slots, radius)).toEqual(Array.from({ length: slots }, (_, i) => i));
  });

  it("anchors the last page with one gap near the start", () => {
    const list = buildPageList(1, 40, radius);
    expect(list).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, null, 39]);
    expect(list).toHaveLength(slots);
  });

  it("anchors the first page with one gap near the end", () => {
    const list = buildPageList(38, 40, radius);
    expect(list).toEqual([0, null, 31, 32, 33, 34, 35, 36, 37, 38, 39]);
    expect(list).toHaveLength(slots);
  });

  it("anchors both ends around a mid-run window", () => {
    const list = buildPageList(20, 40, radius);
    expect(list).toEqual([0, null, 17, 18, 19, 20, 21, 22, 23, null, 39]);
    expect(list).toHaveLength(slots);
  });

  it("keeps a constant width as the current page moves", () => {
    for (let page = 0; page < 40; page++) {
      expect(buildPageList(page, 40, radius)).toHaveLength(slots);
    }
  });
});
