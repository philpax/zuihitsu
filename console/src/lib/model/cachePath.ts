import type { Message } from "@zuihitsu/wire/types/Message.ts";
import type { PromptSectionKind } from "@zuihitsu/wire/types/PromptSectionKind.ts";
import type { ModelInteraction } from "./interactions.ts";
import { resolveSections } from "./promptSections.ts";

/// Why a call could not extend the previous call's prompt, most specific first.
/// `tool-ids-reminted` is the near-lossless case: the rebuilt buffer re-minted tool-call ids while
/// everything else extended — most chat templates tokenize little or none of the id, so the
/// provider's measured warmth stays high despite the value-level difference. The agent now
/// normalizes ids so live and re-rendered exchanges match byte for byte; this cause survives for
/// logs recorded before that.
export type CacheCause =
  | "first-call"
  | "system-changed"
  | "tools-changed"
  | "tool-ids-reminted"
  | "new-session"
  | "buffer-rewritten";

/// The self-derived cache verdict for one call. "Warm" means the previous call's entire prompt is a
/// strict prefix of this one — the necessary condition for a server-side prefix-cache hit, not a
/// guaranteed one (the server may still have evicted); provider counts, when present, are ground
/// truth and disagreements are flagged rather than reconciled.
export interface CacheVerdict {
  path: "warm" | "cold";
  cause?: CacheCause;
  divergence?: { sectionKind: PromptSectionKind | null; offset: number };
  providerDisagreement?: "no-read-despite-warm";
}

/// Derive a verdict per call by comparing each call's reconstructed prompt against the previous
/// call's, across the whole conversation. `sessionSeams` is the set of `SessionStarted` seqs, used
/// to attribute a rebuilt buffer to a session seam rather than an unexplained rewrite.
export function deriveCachePaths(
  calls: ModelInteraction[],
  sessionSeams: number[],
): CacheVerdict[] {
  const seams = [...sessionSeams].sort((a, b) => a - b);
  return calls.map((call, index) => {
    const verdict = compare(index > 0 ? calls[index - 1] : null, call, seams);
    return withDisagreement(verdict, call);
  });
}

function compare(
  prior: ModelInteraction | null,
  call: ModelInteraction,
  seams: number[],
): CacheVerdict {
  if (prior === null) {
    return { path: "cold", cause: "first-call" };
  }
  if (call.system !== prior.system) {
    const offset = firstDifference(prior.system, call.system);
    const section = resolveSections(call.system, call.systemSections).find(
      (candidate) => offset >= candidate.start && offset < candidate.end,
    );
    return {
      path: "cold",
      cause: "system-changed",
      divergence: { sectionKind: section?.kind ?? null, offset },
    };
  }
  if (JSON.stringify(call.tools) !== JSON.stringify(prior.tools)) {
    return { path: "cold", cause: "tools-changed" };
  }
  if (extendsMessages(prior.messages, call.messages, false)) {
    return { path: "warm" };
  }
  // The rebuilt buffer re-renders tool exchanges with fresh call ids; when the ids are the only
  // difference, the wire tokens barely move — attribute that precisely rather than as a rebuild.
  if (extendsMessages(prior.messages, call.messages, true)) {
    return { path: "cold", cause: "tool-ids-reminted" };
  }
  // The buffer was rebuilt: a session seam between the two calls explains it; otherwise the
  // rewrite is surfaced as its own cause.
  const seamBetween = seams.some((seq) => seq > prior.seq && seq < call.seq);
  return { path: "cold", cause: seamBetween ? "new-session" : "buffer-rewritten" };
}

/// Whether `current` extends `prior` append-only: every prior message equal in place, and at least
/// as many messages. An identical buffer still counts — a retry resends the same prompt, and the
/// prefix is exactly as reusable. The reconstruction shares message objects along a group's prefix,
/// so reference equality settles the common case in a pointer compare; the structural fallback only
/// runs where the objects genuinely differ (across group boundaries). With `ignoreToolIds`, the
/// tool-call ids are blanked before comparing, isolating the re-minted-id case.
function extendsMessages(prior: Message[], current: Message[], ignoreToolIds: boolean): boolean {
  if (current.length < prior.length) return false;
  for (let i = 0; i < prior.length; i += 1) {
    if (prior[i] === current[i]) continue;
    const a = ignoreToolIds ? withoutToolIds(prior[i]) : prior[i];
    const b = ignoreToolIds ? withoutToolIds(current[i]) : current[i];
    if (JSON.stringify(a) !== JSON.stringify(b)) return false;
  }
  return true;
}

function withoutToolIds(message: Message): Message {
  if (message.tool_calls.length === 0 && message.tool_call_id === null) return message;
  return {
    ...message,
    tool_calls: message.tool_calls.map((call) => ({ ...call, id: "" })),
    // The paired tool-result message carries the same re-minted id in `tool_call_id`.
    tool_call_id: message.tool_call_id === null ? null : "",
  };
}

function firstDifference(a: string, b: string): number {
  const shorter = Math.min(a.length, b.length);
  for (let i = 0; i < shorter; i += 1) {
    if (a[i] !== b[i]) return i;
  }
  return shorter;
}

/// Cross-check the verdict against the provider's reported cache reads. Only a warm verdict with a
/// read of exactly zero is a disagreement — the strictest honest reading of "near-zero", and an
/// unreported count (`null`) never disagrees (unknown, not zero). A read on a *cold* call is not a
/// disagreement at all: a prefix cache reuses the longest common prefix, so a rebuilt buffer still
/// reads the shared head — the count itself tells that story.
function withDisagreement(verdict: CacheVerdict, call: ModelInteraction): CacheVerdict {
  const read = call.usage.cache_read_tokens;
  if (read === null || read === undefined) return verdict;
  if (verdict.path === "warm" && read === 0) {
    return { ...verdict, providerDisagreement: "no-read-despite-warm" };
  }
  return verdict;
}
