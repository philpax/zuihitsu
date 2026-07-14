import { createContext } from "react";

import type { DigestStatus } from "../../lib/replica/replica.ts";
import type { ContextDebug } from "../../lib/model/contextDebug.ts";
import type { LiveConnection } from "../../lib/api/live.ts";

/// The participate capability the agent frame hands the Conversation view (absent in the eval frame,
/// which is a finished log and so read-only). `sender` is the handle you converse under as a
/// participant, lifted to the frame so it survives view switches. Whether the cursor is at the head
/// — the gate on speaking into the present, and on following the live tail — rides the view's own
/// `atHead` prop, since a read-only eval run at its head follows the tail too.
export interface Participation {
  connection: LiveConnection;
  sender: string;
  setSender: (value: string) => void;
}

/// The reconstructed model calls with their derived context debugging — per-call cache verdicts,
/// token attributions, digest verifications, and the denominators in effect — so a turn's
/// deliberation can show what each call fed the model, how it was assembled, and whether the prefix
/// cache survived, without drilling the lookups through every layer of the transcript.
export const ModelCalls = createContext<ContextDebug & { digestBySeq: Map<number, DigestStatus> }>({
  bySeq: new Map(),
  verdictBySeq: new Map(),
  attributionBySeq: new Map(),
  denominatorsBySeq: new Map(),
  denominators: { budget: null, contextLength: null },
  digestBySeq: new Map(),
});

/// The id → handle map at the cursor, so a turn's outcome rows can expand into the event viewer (which
/// resolves memory and participant ids) without drilling the map through the transcript.
export const Names = createContext<Map<string, string>>(new Map());

/// The conversation id → context memory name map at the cursor, so `ConversationRef` links in
/// event detail panels can resolve the room name without a separate prop chain.
export const ConversationNames = createContext<Map<string, string>>(new Map());
