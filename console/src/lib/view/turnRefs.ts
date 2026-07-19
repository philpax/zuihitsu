import { createContext } from "react";

import type { RefSegment } from "@zuihitsu/wire/wasm/console_wasm.js";
import type { TurnModel } from "../model/conversation.ts";
import { scanRefs } from "../replica/replica.ts";
import { MEM_CHIP_SCHEME } from "./memRefs.ts";

// The non-component half of turn references (spec §Conversations → Transcript references): the
// lookup context the Conversation view fills, and the remark plugin that lifts scanned references
// out of an agent turn's Markdown. The chips themselves live in `components/TurnRefs.tsx`. The one
// remark pass lifts both reference kinds in a single combined wasm scan (`scanRefs`) — both
// token vocabularies — dispatching only on the returned `kind`, never inspecting token syntax.
// A memory's console State-view deep-link URL routes by handle, so it is matched by the transcript's
// anchor override (route matching, not token parsing), not lifted here.

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
    // A GFM-autolinked deep-link URL becomes a chip for the whole link. A link the author labeled
    // (`[see here](…?turn=…)`) keeps its label and renders as an ordinary anchor, and no chip is
    // ever nested inside another anchor — so links are never descended into.
    if (child.type === "link") {
      const link = autolinkRef(child);
      next.push(link ?? child);
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

/// The reference chip an autolinked URL resolves to, or `null` when the URL carries no token reference
/// (so it renders as an ordinary anchor — a State-view URL, matched by handle, is caught there by the
/// anchor override instead). The whole URL is one scannable token: a `?turn=` deep link scans to a
/// single turn reference, which becomes the chip.
function autolinkRef(child: MdNode): MdNode | null {
  const autolink =
    child.children?.length === 1 &&
    child.children[0].type === "text" &&
    child.children[0].value === child.url;
  if (!autolink) return null;
  const segments = scanRefs(child.url ?? "");
  const [only] = segments;
  return segments.length === 1 && only.kind !== "prose" ? refChipLink(only) : null;
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
