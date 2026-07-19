import { createContext } from "react";

import { rewriteStateUrls } from "../nav/refRoutes.ts";
import type { MemRefResolution } from "@zuihitsu/wire/wasm/console_wasm.js";
import { type Replica, constructMemRef, normalizeRefTokens } from "../replica/replica.ts";

// The non-component half of memory references: the resolver context the workspace fills, the
// console-internal chip scheme the remark pass smuggles a scanned reference through react-markdown
// with, and the composer's send-time normalization. The chip itself lives in
// `views/conversation/MemRefs.tsx`. A scanned memory-reference token, or
// a console State-view deep link the composer maps to one, renders as an inline chip linking to the
// memory's State view — the memory counterpart to turn references (`turnRefs.ts`).

/// The console-internal rendering scheme the remark pass smuggles a scanned memory reference through
/// react-markdown with — plumbing between the remark pass and the anchor override, never serialized
/// anywhere. A scanned reference becomes a link node `mem-chip:<id>` (an id payload, resolved
/// through the graph); a transcript's autolinked State-view URL, matched by the anchor override,
/// becomes `mem-chip:@<handle>` (a handle payload, resolved and linked directly), distinguished by the
/// leading `@`, which never begins a memory id. The anchor override renders either as a chip.
export const MEM_CHIP_SCHEME = "mem-chip:";

/// The prefix that marks a handle-carrying chip payload (`mem-chip:@<handle>`), as opposed to an
/// id-carrying one (`mem-chip:<id>`).
export const MEM_CHIP_HANDLE_SIGIL = "@";

/// Resolve a memory reference to its display target, or `null` when it names no memory in the folded
/// graph (so the chip degrades). `byId` resolves a scanned token's id to its `same_as` class primary;
/// `byHandle` resolves a State-view URL's handle directly. Filled by the workspace from the replica at
/// the current fold cursor; the default resolves nothing, for a tree rendered without a provider.
export interface MemRefResolver {
  byId: (id: string) => MemRefResolution | null;
  byHandle: (handle: string) => MemRefResolution | null;
}

export const MemRefs = createContext<MemRefResolver>({
  byId: () => null,
  byHandle: () => null,
});

/// Normalize a console-composed message's memory references before it posts — the send-time counterpart
/// to `normalizeTurnRefs`. Every console State-view deep link on an origin the console owns (`origins`)
/// whose handle resolves to a memory (by current name, then by a former-name alias, so a stale pasted
/// link still normalizes) collapses to its canonical memory-reference token; a foreign or unresolved link
/// is left untouched. Any reference token already in the text is canonicalized too. So a message that
/// leaves the console carries only token syntax.
export function normalizeMemRefs(
  text: string,
  replica: Replica,
  origins: readonly string[],
): string {
  const withTokens = rewriteStateUrls(
    text,
    (handle) => {
      const id = replica.memoryIdByName(handle) ?? replica.memoryIdForFormerName(handle);
      return id === null ? null : constructMemRef(id);
    },
    origins,
  );
  return normalizeRefTokens(withTokens);
}
