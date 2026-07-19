import type { SectionKind } from "@zuihitsu/wire/types/SectionKind.ts";
import type { VisibilityDecision } from "@zuihitsu/wire/types/VisibilityDecision.ts";

/// Whether a verdict surfaced the entry, and a short human reason for the trace.
export function decisionInfo(decision: VisibilityDecision): { visible: boolean; reason: string } {
  switch (decision) {
    case "Public":
      return { visible: true, reason: "public" };
    case "Attributed":
      return { visible: true, reason: "attributed" };
    case "TellerPresent":
      return { visible: true, reason: "teller present" };
    case "NotExcluded":
      return { visible: true, reason: "no excludee present" };
    case "Superseded":
      return { visible: false, reason: "superseded" };
    case "TellerAbsent":
      return { visible: false, reason: "teller absent" };
    case "SubjectPresent":
      return { visible: false, reason: "subject present" };
    case "ExcludeePresent":
      return { visible: false, reason: "excludee present" };
  }
}

/// The role each section played, for its label.
export function sectionLabel(kind: SectionKind): string {
  switch (kind) {
    case "SelfBrief":
      return "self";
    case "CurrentRoom":
      return "current room";
    case "Participant":
      return "present";
    case "ActiveThread":
      return "active thread";
  }
}
