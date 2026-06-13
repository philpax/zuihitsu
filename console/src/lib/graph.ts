// The shapes the console-wasm `Replica` returns from its graph queries. Every field type is a
// generated ts-rs binding (so a Rust change is caught here); only the struct groupings — which
// mirror the wrapper's small composed DTOs — are written by hand. TagName and RelationName are
// inlined as `string` by ts-rs, so they appear as `string` below.
//
// TODO: when the core view types and the wrapper DTOs grow ts-rs derives, these become generated
// too and this file goes away.

import type { Cardinality } from "../types/Cardinality.ts";
import type { ConversationId } from "../types/ConversationId.ts";
import type { EntryId } from "../types/EntryId.ts";
import type { MemoryId } from "../types/MemoryId.ts";
import type { MemoryName } from "../types/MemoryName.ts";
import type { SessionId } from "../types/SessionId.ts";
import type { Teller } from "../types/Teller.ts";
import type { Timestamp } from "../types/Timestamp.ts";
import type { Visibility } from "../types/Visibility.ts";
import type { Volatility } from "../types/Volatility.ts";

export interface MemoryView {
  id: MemoryId;
  name: MemoryName;
  description: string;
  volatility: Volatility;
  created_at: Timestamp;
  tags: string[];
}

export interface EntryView {
  entry_id: EntryId;
  asserted_at: Timestamp;
  occurred_sort: Timestamp | null;
  text: string;
  told_by: Teller;
  told_in: MemoryId | null;
  visibility: Visibility;
  superseded_by: EntryId | null;
}

export interface LinkView {
  from: MemoryId;
  to: MemoryId;
  relation: string;
}

export interface RelationView {
  name: string;
  inverse: string;
  from_card: Cardinality;
  to_card: Cardinality;
  symmetric: boolean;
  reflexive: boolean;
}

export interface TagVocabularyEntry {
  name: string;
  description: string;
  count: number;
}

export interface MemoryDetail {
  memory: MemoryView;
  entries: EntryView[];
  history: EntryView[];
  links: LinkView[];
  class: MemoryView[];
}

export interface SessionSummary {
  id: SessionId;
  started_at: Timestamp;
  brief: string;
  participants: string[];
}

export interface ConversationDetail {
  id: ConversationId;
  platform: string;
  scope_path: string;
  context_name: string | null;
  sessions: SessionSummary[];
}
