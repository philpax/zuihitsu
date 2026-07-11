// The shapes the console-wasm `Replica` returns from its graph queries. Every field type is a
// generated ts-rs binding (so a Rust change is caught here); only the struct groupings — which
// mirror the wrapper's small composed DTOs — are written by hand. TagName and RelationName are
// inlined as `string` by ts-rs, so they appear as `string` below.
//
// TODO: when the core view types and the wrapper DTOs grow ts-rs derives, these become generated
// too and this file goes away.

import type { Cardinality } from "../../types/Cardinality.ts";
import type { ConversationId } from "../../types/ConversationId.ts";
import type { ConversationRef } from "../../types/ConversationRef.ts";
import type { EntryId } from "../../types/EntryId.ts";
import type { MemoryId } from "../../types/MemoryId.ts";
import type { MemoryName } from "../../types/MemoryName.ts";
import type { MergeProposalSource } from "../../types/MergeProposalSource.ts";
import type { SessionId } from "../../types/SessionId.ts";
import type { Teller } from "../../types/Teller.ts";
import type { Timestamp } from "../../types/Timestamp.ts";
import type { Visibility } from "../../types/Visibility.ts";
import type { Volatility } from "../../types/Volatility.ts";

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
  told_in: ConversationRef | null;
  visibility: Visibility;
  superseded_by: EntryId | null;
}

export interface LinkView {
  from: MemoryId;
  to: MemoryId;
  relation: string;
  told_by: Teller | null;
  told_in: ConversationRef | null;
  visibility: Visibility;
}

export interface RelationView {
  name: string;
  inverse: string;
  from_card: Cardinality;
  to_card: Cardinality;
  symmetric: boolean;
  reflexive: boolean;
  description: string;
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
  // Entry ids currently under an unresolved belief arbitration — the view marks these as disputed.
  disputed: EntryId[];
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

/// Where a merge proposal stands at the fold cursor — pending an operator or adjudicator decision,
/// merged (the two stubs now share a `same_as` class), or rejected (a refusal was recorded).
export type MergeStatus = "pending" | "merged" | "rejected";

/// One cross-platform merge proposal as the replica derives it from the folded log: the two stubs
/// (by handle and id), who raised it, the proposer's stated grounds if any, and its resolution state.
export interface MergeProposalView {
  from: MemoryName;
  to: MemoryName;
  from_id: MemoryId;
  to_id: MemoryId;
  source: MergeProposalSource;
  rationale: string | null;
  status: MergeStatus;
  // Whether each stub is currently its class's primary — the id class-level reads resolve through.
  from_primary: boolean;
  to_primary: boolean;
  // Whether the operator has pinned each stub as its class's primary, as opposed to it winning by the
  // earliest-ULID default. A pinned stub can be released; an unpinned, non-primary stub can be pinned.
  from_designated: boolean;
  to_designated: boolean;
}

export interface AgendaItem {
  /// The instant the item next occurs, in epoch milliseconds.
  when: number;
  /// The occurrence is a whole day or fuzzier span, not a precise instant, so it renders without a
  /// clock time (a day-level reference sorts at noon — not a stated time).
  all_day: boolean;
  memory: string;
  text: string;
  recurring: boolean;
}
