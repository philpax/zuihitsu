import type { Completion } from "@zuihitsu/wire/types/Completion.ts";
import type { MemoryView } from "./graph.ts";
import type { Teller } from "@zuihitsu/wire/types/Teller.ts";
import type { TerminalCause } from "@zuihitsu/wire/types/TerminalCause.ts";
import type { Visibility } from "@zuihitsu/wire/types/Visibility.ts";

/// Presentation helpers shared by the State, Conversation, and Events views, so a teller, a
/// visibility, or a completion reads the same wherever it surfaces.

/// The id → handle map the views resolve memory ids through (a teller, a link target, a brief's
/// present set). Built once per render from the folded memory list, so every view names ids the
/// same way.
export function nameById(memories: MemoryView[]): Map<string, string> {
  return new Map(memories.map((memory) => [memory.id, memory.name]));
}

export function tellerLabel(teller: Teller, nameById: Map<string, string>): string {
  if (teller === "Agent") return "the agent";
  if (teller === "Bootstrap") return "genesis";
  return nameById.get(teller.Participant) ?? teller.Participant;
}

export function visibilityLabel(visibility: Visibility, nameById: Map<string, string>): string {
  if (visibility === "Public") return "public";
  if (visibility === "Attributed") return "attributed";
  if (visibility === "PrivateToTeller") return "teller-private";
  const names = visibility.Exclude.map((id) => nameById.get(id) ?? id);
  return `excludes ${names.join(", ")}`;
}

/// Whether a visibility is anything other than public — the cue to mark a confidence or an
/// attributed (secondhand) entry, both of which carry provenance the operator should see.
export function isPrivate(visibility: Visibility): boolean {
  return visibility !== "Public";
}

/// How a Lua block ended when it did not run to completion — an error or a deliberate abort, read
/// the same wherever a terminal cause surfaces (the log summary, the event detail, the transcript).
export function terminalCauseLabel(cause: TerminalCause): string {
  return "Error" in cause ? `error: ${cause.Error}` : `aborted: ${cause.Aborted}`;
}

export function completionSummary(completion: Completion): string {
  if (completion === "Silent") return "stayed silent";
  if ("Reply" in completion) return "replied";
  const count = completion.ToolCalls.length;
  return `${count} tool call${count > 1 ? "s" : ""}`;
}
