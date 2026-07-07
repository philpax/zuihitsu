import type { MemoryName } from "../../types/MemoryName.ts";
import type { Visibility } from "../../types/Visibility.ts";

// The shapes the console-wasm `brief()` returns — the composed brief plus the trace of how it was
// built. Field types are generated ts-rs bindings; only the groupings are hand-written (mirroring
// zuihitsu_core::brief), as with graph.ts.

export type VisibilityDecision =
  | "Public"
  | "Attributed"
  | "TellerPresent"
  | "NotExcluded"
  | "Superseded"
  | "TellerAbsent"
  | "SubjectPresent"
  | "ExcludeePresent";

export type SectionKind = "SelfBrief" | "CurrentRoom" | "Participant" | "ActiveThread";

export interface EntryTrace {
  text: string;
  visibility: Visibility;
  decision: VisibilityDecision;
  in_brief: boolean;
}

export interface BriefSectionTrace {
  kind: SectionKind;
  memory: MemoryName;
  confidential: boolean;
  entries: EntryTrace[];
}

export interface BriefTrace {
  text: string;
  sections: BriefSectionTrace[];
}

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
