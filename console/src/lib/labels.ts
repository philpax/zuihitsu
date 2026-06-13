import type { Completion } from "../types/Completion.ts";
import type { Teller } from "../types/Teller.ts";
import type { Visibility } from "../types/Visibility.ts";

/// Presentation helpers shared by the State, Conversation, and Events views, so a teller, a
/// visibility, or a completion reads the same wherever it surfaces.

export function tellerLabel(teller: Teller, nameById: Map<string, string>): string {
  if (teller === "Agent") return "the agent";
  if (teller === "Bootstrap") return "genesis";
  return nameById.get(teller.Participant) ?? teller.Participant;
}

export function visibilityLabel(visibility: Visibility, nameById: Map<string, string>): string {
  if (visibility === "Public") return "public";
  if (visibility === "PrivateToTeller") return "teller-private";
  const names = visibility.Exclude.map((id) => nameById.get(id) ?? id);
  return `excludes ${names.join(", ")}`;
}

/// Whether a visibility is anything other than public — the cue to mark it.
export function isPrivate(visibility: Visibility): boolean {
  return visibility !== "Public";
}

export function completionSummary(completion: Completion): string {
  if (completion === "Silent") return "stayed silent";
  if ("Reply" in completion) return "replied";
  const count = completion.ToolCalls.length;
  return `${count} tool call${count > 1 ? "s" : ""}`;
}
