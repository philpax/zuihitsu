import { describe, expect, it } from "vitest";

import type { ScenarioReport } from "../../types/ScenarioReport.ts";
import { groupScenariosByCategory } from "./scenarioGroups.ts";

/// A minimal report carrying only what grouping reads; the cast keeps the fixture honest about
/// exercising the runtime shape rather than the full type.
function report(name: string, category: string): ScenarioReport {
  return { meta: { name, category } } as ScenarioReport;
}

describe("groupScenariosByCategory", () => {
  it("groups in the enum's order and preserves package indices", () => {
    const groups = groupScenariosByCategory([
      report("b", "privacy"),
      report("a", "recall"),
      report("c", "privacy"),
    ]);
    expect(groups.map((group) => group.category)).toEqual(["recall", "privacy"]);
    expect(groups[1].entries.map((entry) => entry.index)).toEqual([0, 2]);
  });

  it("maps legacy category names onto their absorbing category", () => {
    const groups = groupScenariosByCategory([
      report("old-sched", "scheduling"),
      report("old-compaction", "compaction"),
      report("old-description", "description"),
      report("old-arbitration", "arbitration"),
    ]);
    expect(groups.map((group) => group.category)).toEqual(["time", "sessions", "synthesis"]);
    expect(groups[2].entries.map((entry) => entry.scenario.meta.name)).toEqual([
      "old-description",
      "old-arbitration",
    ]);
  });

  it("keeps a genuinely unknown category at the end rather than dropping it", () => {
    const groups = groupScenariosByCategory([
      report("future", "holograms"),
      report("normal", "recall"),
    ]);
    expect(groups.map((group) => group.category)).toEqual(["recall", "holograms"]);
    expect(groups[1].entries).toHaveLength(1);
  });
});
