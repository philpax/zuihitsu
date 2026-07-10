import { describe, expect, it } from "vitest";

import type { PromptSectionSpan } from "../../types/PromptSectionSpan.ts";
import { resolveSections } from "./promptSections.ts";

/// A realistic assembled prompt with all six sections, matching `assemble()`'s emission shape.
const FULL_PROMPT =
  "You are a careful assistant." +
  "\n\n# Who you are\n\nI keep confidences." +
  "\n\n# What you can do\n\nMethod notation legend.\n\nrun_lua(script)" +
  "\n\n# Tags\n- keepsake: a thing to keep." +
  "\n\n# What you know right now\n\n## person/staff_engineer\nStaff engineer." +
  "\n\n# Current time\n\nThe session begins on 2026-07-10.";

function tiles(sections: Array<{ start: number; end: number }>, length: number) {
  expect(sections[0]?.start).toBe(0);
  for (let i = 1; i < sections.length; i += 1) {
    expect(sections[i].start).toBe(sections[i - 1].end);
  }
  expect(sections[sections.length - 1]?.end).toBe(length);
}

describe("resolveSections with recorded spans", () => {
  it("passes recorded spans through with recorded provenance", () => {
    const system = "scaffold\n\n# Current time\n\nnow.";
    const bytes = new TextEncoder().encode(system).length;
    const recorded: PromptSectionSpan[] = [
      { kind: "Scaffold", start: 0, end: 8 },
      { kind: "CurrentTime", start: 8, end: bytes },
    ];
    const sections = resolveSections(system, recorded);
    expect(sections).toHaveLength(2);
    expect(sections.every((section) => section.provenance === "recorded")).toBe(true);
    expect(system.slice(sections[0].start, sections[0].end)).toBe("scaffold");
    tiles(sections, system.length);
  });

  it("converts byte offsets to string indices across multibyte text", () => {
    // The scaffold ends with a four-byte emoji (two UTF-16 units), so byte and string offsets
    // diverge; the recorded span is in bytes and the resolved one must slice correctly.
    const scaffold = "scaffold 🐚";
    const time = "\n\n# Current time\n\nnow.";
    const system = scaffold + time;
    const scaffoldBytes = new TextEncoder().encode(scaffold).length;
    const totalBytes = new TextEncoder().encode(system).length;
    const sections = resolveSections(system, [
      { kind: "Scaffold", start: 0, end: scaffoldBytes },
      { kind: "CurrentTime", start: scaffoldBytes, end: totalBytes },
    ]);
    expect(system.slice(sections[0].start, sections[0].end)).toBe(scaffold);
    expect(system.slice(sections[1].start, sections[1].end)).toBe(time);
    tiles(sections, system.length);
  });
});

describe("resolveSections heuristic fallback", () => {
  it("infers all six sections from a full prompt", () => {
    const sections = resolveSections(FULL_PROMPT, []);
    expect(sections.map((section) => section.kind)).toEqual([
      "Scaffold",
      "Identity",
      "ApiReference",
      "Vocabulary",
      "Brief",
      "CurrentTime",
    ]);
    expect(sections.every((section) => section.provenance === "inferred")).toBe(true);
    tiles(sections, FULL_PROMPT.length);
  });

  it("tolerates absent conditional sections and still tiles", () => {
    const system =
      "scaffold only" + "\n\n# What you can do\n\nrun_lua" + "\n\n# Current time\n\nnow.";
    const sections = resolveSections(system, []);
    expect(sections.map((section) => section.kind)).toEqual([
      "Scaffold",
      "ApiReference",
      "CurrentTime",
    ]);
    tiles(sections, system.length);
  });

  it("recognizes a relations-first vocabulary residue", () => {
    const system =
      "scaffold" +
      "\n\n# What you can do\n\nrun_lua" +
      "\n\n# Relations\n- knows: acquaintance." +
      "\n\n# Current time\n\nnow.";
    const sections = resolveSections(system, []);
    expect(sections.map((section) => section.kind)).toContain("Vocabulary");
    tiles(sections, system.length);
  });

  it("returns nothing for an empty system", () => {
    expect(resolveSections("", [])).toEqual([]);
  });
});
