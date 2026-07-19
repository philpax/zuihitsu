import { createContext } from "react";

import type { RefSegment } from "@zuihitsu/wire/wasm/console_wasm.js";
import type { TurnModel } from "../model/conversation.ts";
import { constructTurnRef, normalizeRefTokens, scanRefs } from "../replica/replica.ts";
import { rewriteTurnUrls } from "../nav/refRoutes.ts";
import { MEM_CHIP_SCHEME } from "./memRefs.ts";

// The non-component half of turn references (spec §Conversations → Transcript references): the
// lookup context the Conversation view fills, the remark plugin that lifts scanned references out of an
// agent turn's Markdown, and the composer's send-time normalization. The chips themselves live in
// `components/TurnRefs.tsx`. The one remark pass lifts both reference kinds in a single combined wasm
// scan (`scanRefs`) — both token vocabularies — dispatching only on the returned `kind`, never
// inspecting token syntax. A deep-link URL routes by console page, not by token syntax, so it is matched
// by the transcript's anchor override (route matching), not lifted here: a memory's State-view link by
// handle, a conversation link by its pinned `?turn=` moment.

/// Normalize a console-composed message's turn references before it posts — the send-time counterpart to
/// `normalizeMemRefs`. Every console Conversation-view deep link on an origin the console owns (`origins`)
/// whose `?turn=` id is well-formed collapses to its canonical turn-reference token; a foreign link, or
/// one with a malformed id, is left untouched. Any reference token already in the text is canonicalized
/// too. So a message that leaves the console carries only token syntax.
export function normalizeTurnRefs(text: string, origins: readonly string[]): string {
  const withTokens = rewriteTurnUrls(text, mintTurnRef, origins);
  return normalizeRefTokens(withTokens);
}

/// Mint the canonical turn-reference token for a pinned turn id, or `null` when the id is malformed
/// (`constructTurnRef` throws), so the deep link is left as an ordinary URL rather than a broken token.
function mintTurnRef(id: string): string | null {
  try {
    return constructTurnRef(id);
  } catch {
    return null;
  }
}

/// The URL scheme the remark plugin smuggles a reference through react-markdown with: a scanned ref
/// becomes a link node `turn-chip:<id>`, which the anchor override renders as a chip.
export const TURN_CHIP_SCHEME = "turn-chip:";

/// Where a referenced turn lives in the folded log: the moment itself, the room that holds it (as
/// the sidebar's channel key, for the deep link), and its immediate neighbors for the hover preview.
export interface TurnRefTarget {
  turn: TurnModel;
  roomKey: string;
  /// The focal turn and up to two neighbors either side, in transcript order.
  window: TurnModel[];
  /// The focal turn's index within `window`.
  focusIndex: number;
}

/// Every folded turn by id, built by the Conversation view at the current cursor — so a chip
/// resolves against exactly what the timeline shows, and an id past the cursor reads as unknown.
export const TurnRefs = createContext<Map<string, TurnRefTarget>>(new Map());

/// The chip and popup label for a turn's speaker, matching the transcript's own labels.
export function speakerLabel(turn: TurnModel): string {
  if (turn.role === "Agent") return "the agent";
  if (turn.role === "System") return "system";
  return turn.speaker ?? "someone";
}

/// A minimal mdast node — just the fields the reference splitter walks, so the plugin does not pull
/// the mdast type packages in for four properties.
interface MdNode {
  type: string;
  value?: string;
  url?: string;
  children?: MdNode[];
}

/// A remark plugin that turns scanned references in an agent turn's Markdown into `turn-chip:` and
/// `mem-chip:` link nodes, which the anchor override renders as chips. Operates on the mdast text nodes,
/// so code blocks and inline code are naturally untouched (their content is not a text node).
export function remarkTurnRefs() {
  return (tree: MdNode) => splitRefs(tree);
}

function splitRefs(node: MdNode): void {
  if (!node.children) return;
  const next: MdNode[] = [];
  for (const child of node.children) {
    // A link node carries a URL, never a bracket token, so there is nothing to lift out of it here: a
    // deep-link URL becomes a chip by route matching in the anchor override, and no chip is ever nested
    // inside another anchor — so links are kept whole and never descended into.
    if (child.type === "link") {
      next.push(child);
      continue;
    }
    if (child.type === "text" && child.value) {
      const nodes = splitRefText(child.value);
      if (nodes) {
        next.push(...nodes);
        continue;
      }
    }
    splitRefs(child);
    next.push(child);
  }
  node.children = next;
}

/// Split a text node's value into prose and reference link nodes, or `null` when it carries none — one
/// combined wasm scan over both token vocabularies, the caller dispatching only on `kind`.
function splitRefText(value: string): MdNode[] | null {
  const nodes: MdNode[] = [];
  let any = false;
  for (const segment of scanRefs(value)) {
    if (segment.kind === "prose") {
      nodes.push({ type: "text", value: segment.text });
    } else {
      nodes.push(refChipLink(segment));
      any = true;
    }
  }
  return any ? nodes : null;
}

/// The link node a scanned reference becomes — a `turn-chip:` or `mem-chip:` scheme the anchor override
/// catches and renders as the matching chip.
function refChipLink(segment: Exclude<RefSegment, { kind: "prose" }>): MdNode {
  const scheme = segment.kind === "turn" ? TURN_CHIP_SCHEME : MEM_CHIP_SCHEME;
  return {
    type: "link",
    url: scheme + segment.id,
    children: [{ type: "text", value: segment.id }],
  };
}
