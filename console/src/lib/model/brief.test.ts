import { describe, expect, it } from "vitest";

import { type VisibilityDecision, decisionInfo } from "./brief.ts";

const DECISIONS: VisibilityDecision[] = [
  "Public",
  "Attributed",
  "TellerPresent",
  "NotExcluded",
  "Superseded",
  "TellerAbsent",
  "SubjectPresent",
  "ExcludeePresent",
];

describe("decisionInfo", () => {
  it("maps every verdict to a non-empty plain-words reason (AC6.4)", () => {
    for (const decision of DECISIONS) {
      expect(decisionInfo(decision).reason.length).toBeGreaterThan(0);
    }
  });

  it("splits verdicts into visible and filtered", () => {
    expect(DECISIONS.filter((decision) => decisionInfo(decision).visible)).toEqual([
      "Public",
      "Attributed",
      "TellerPresent",
      "NotExcluded",
    ]);
  });
});
