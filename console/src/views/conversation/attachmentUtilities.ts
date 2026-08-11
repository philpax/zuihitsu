import type { BlobHash } from "@zuihitsu/wire/types/BlobHash.ts";
import type { OutboundAttachment } from "../../lib/api/participant.ts";
import { OCTET_STREAM } from "../../lib/api/blobs.ts";

/// A file the composer holds for the message being written. The bytes go up as the file is added, so
/// a pending attachment is one of three things: still uploading, stored and ready to be named by the
/// message, or failed with the reason.
export interface PendingAttachment {
  /// A client-side identity, minted per pick. Not the content address: the address is unknown until
  /// the upload lands, and two picks of the same bytes are two chips the sender can remove
  /// independently.
  id: string;
  name: string;
  mime: string;
  byteLen: number;
  /// The file itself, kept so a failed upload retries without re-picking it.
  file: File;
  status: PendingStatus;
}

export type PendingStatus =
  | { state: "uploading" }
  | { state: "ready"; blob: BlobHash }
  | { state: "failed"; error: string };

/// A picked file, before its bytes have gone anywhere.
export function pendingAttachment(file: File): PendingAttachment {
  return {
    id: crypto.randomUUID(),
    name: file.name,
    mime: file.type || OCTET_STREAM,
    byteLen: file.size,
    file,
    status: { state: "uploading" },
  };
}

/// The same list with one member's status replaced — the one shape every upload transition takes.
export function withStatus(
  attachments: PendingAttachment[],
  id: string,
  status: PendingStatus,
): PendingAttachment[] {
  return attachments.map((attachment) =>
    attachment.id === id ? { ...attachment, status } : attachment,
  );
}

/// Whether the message is waiting on its files. A send holds while any attachment is unsettled: an
/// upload in flight has no address to name, and a failed one would otherwise be dropped silently from
/// a message the sender watched themselves attach it to. Removing (or retrying) the offending chip
/// releases the send — the failure blocks its own file, not the message's other files.
export function attachmentsHoldSend(attachments: PendingAttachment[]): boolean {
  return attachments.some((attachment) => attachment.status.state !== "ready");
}

/// A file handed to the send: the two fields the wire carries, plus what the browser knew about it.
/// The extras never reach the server — the stored blob is authoritative for both — but the optimistic
/// echo needs them to announce the file before the real turn, carrying the server's classification,
/// folds in behind it.
export interface SentAttachment extends OutboundAttachment {
  mime: string;
  byteLen: number;
}

/// The stored files a message names, in the order they were attached. Callers reach this only once
/// [`attachmentsHoldSend`] is false, so every attachment contributes.
export function outboundAttachments(attachments: PendingAttachment[]): SentAttachment[] {
  return attachments.flatMap((attachment) =>
    attachment.status.state === "ready"
      ? [
          {
            name: attachment.name,
            mime: attachment.mime,
            byteLen: attachment.byteLen,
            blob: attachment.status.blob,
          },
        ]
      : [],
  );
}
