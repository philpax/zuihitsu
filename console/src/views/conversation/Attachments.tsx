import { useEffect, useState } from "react";

import type { Attachment } from "@zuihitsu/wire/types/Attachment.ts";
import { blobUrl } from "../../lib/api/blobs.ts";
import { errorMessage } from "../../lib/api/http.ts";
import { formatBytes } from "../../lib/format/format.ts";
import { useBlobBase } from "../../lib/view/blobBase.ts";
import { Disclosure, Excerpt } from "../../components/primitives.tsx";
import type { PendingAttachment, SentAttachment } from "./attachmentUtilities.ts";

/// What a turn carried, below what it said: an image the reader can actually see, a text file they can
/// open in place, and anything else announced by name, type, and size. The record is all the log holds
/// — the bytes live in the agent's blob store — so every treatment degrades to the announcement when
/// no agent stands behind this console (an eval package), rather than showing a broken image.
export function AttachmentStrip({ attachments }: { attachments: Attachment[] }) {
  const base = useBlobBase();
  if (attachments.length === 0) return null;
  return (
    <ul className="mt-3 flex min-w-0 flex-col items-start gap-2">
      {attachments.map((attachment, index) => (
        // Keyed by position: the same file may legitimately ride a turn twice, and two attachments of
        // identical bytes share a content address, so neither the address nor the name is unique.
        <li key={index} className="max-w-full min-w-0">
          <AttachmentItem
            attachment={attachment}
            url={base === null ? null : blobUrl(base, attachment.blob)}
          />
        </li>
      ))}
    </ul>
  );
}

/// The chips a composer shows for the files a message will carry, and the optimistic echo shows for
/// the ones it just sent. Always the announcement treatment: an upload in flight has no address yet,
/// and the echo is replaced by the real turn — with its full strip — within the round trip.
export function PendingAttachmentChips({
  attachments,
  onRemove,
  onRetry,
}: {
  attachments: PendingAttachment[];
  /// Drop the file from the message. Absent for the sent echo, which is no longer editable.
  onRemove?: (id: string) => void;
  /// Upload the file again after a failure. Absent for the sent echo.
  onRetry?: (id: string) => void;
}) {
  if (attachments.length === 0) return null;
  return (
    <ul className="flex flex-wrap items-center gap-2">
      {attachments.map((attachment) => (
        <li key={attachment.id} className="min-w-0">
          <span
            className={
              chipClass + " " + (attachment.status.state === "failed" ? "border-clay-soft" : "")
            }
          >
            <ChipLabel name={attachment.name} mime={attachment.mime} byteLen={attachment.byteLen} />
            {attachment.status.state === "uploading" && (
              <span className="shrink-0 text-ink-faint">attaching…</span>
            )}
            {attachment.status.state === "failed" && (
              <>
                <span className="shrink-0 text-clay" title={attachment.status.error}>
                  failed
                </span>
                {onRetry && (
                  <button
                    onClick={() => onRetry(attachment.id)}
                    className="shrink-0 text-ink-faint transition-colors hover:text-clay"
                    title={`Upload ${attachment.name} again`}
                  >
                    retry
                  </button>
                )}
              </>
            )}
            {onRemove && (
              <button
                onClick={() => onRemove(attachment.id)}
                className="shrink-0 text-ink-faint transition-colors hover:text-clay"
                title={`Remove ${attachment.name}`}
              >
                ✕
              </button>
            )}
          </span>
        </li>
      ))}
    </ul>
  );
}

/// The chips the optimistic echo shows for the files it just sent: the announcement treatment again,
/// because the echo is replaced by the real turn — and its full strip — within the round trip, and the
/// turn's classification is the server's to make.
export function SentAttachmentChips({ attachments }: { attachments: SentAttachment[] }) {
  if (attachments.length === 0) return null;
  return (
    <ul className="mt-2 flex flex-wrap items-center gap-2">
      {attachments.map((attachment, index) => (
        <li key={index} className="min-w-0">
          <span className={chipClass}>
            <ChipLabel name={attachment.name} mime={attachment.mime} byteLen={attachment.byteLen} />
          </span>
        </li>
      ))}
    </ul>
  );
}

/// One attachment, rendered by what the server classified it as — the one classification the
/// connector, the turn assembly, and this view all read, so the console never second-guesses whether a
/// media type is perceivable. `url` is `null` when no agent serves the bytes.
function AttachmentItem({ attachment, url }: { attachment: Attachment; url: string | null }) {
  if (url === null) return <AttachmentChip attachment={attachment} url={null} />;
  if (attachment.kind === "Image") return <ImageAttachment attachment={attachment} url={url} />;
  if (attachment.kind === "Text") return <TextAttachment attachment={attachment} url={url} />;
  return <AttachmentChip attachment={attachment} url={url} />;
}

/// An image the model perceived, shown as the reader's own turn of it. Capped in both axes so a large
/// upload sits in the transcript rather than swallowing it, and never wider than its column — the page
/// body must not scroll sideways because someone pasted a panorama. The full-size bytes are one click
/// away.
function ImageAttachment({ attachment, url }: { attachment: Attachment; url: string }) {
  return (
    <figure className="min-w-0">
      <a href={url} target="_blank" rel="noreferrer" className="block max-w-full">
        <img
          src={url}
          alt={attachment.name}
          loading="lazy"
          className="max-h-80 max-w-full rounded-xs border border-line object-contain"
        />
      </a>
      <figcaption className="mt-1 truncate font-mono text-2xs text-ink-faint">
        {attachment.name} · {formatBytes(attachment.byte_len)}
      </figcaption>
    </figure>
  );
}

/// A text file, disclosed in place: the excerpt is fetched only when the reader opens it, so a
/// transcript of attachments costs nothing to scroll past. Long content is clipped with a note and a
/// link to the whole thing, matching how the turn itself only ever inlined so much.
///
/// Only the excerpt crosses the wire. A file bigger than the window is fetched as a byte range, so
/// opening a 16 MiB log costs the few kilobytes actually shown rather than the whole download — the
/// clipping was always going to throw the rest away.
function TextAttachment({ attachment, url }: { attachment: Attachment; url: string }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Whether the whole file was asked for, which decides how the clip is judged below: a windowed read
  // is clipped by definition, where a whole one is clipped only if it overflows the excerpt.
  const whole = attachment.byte_len <= EXCERPT_BYTES;

  useEffect(() => {
    // One fetch per attachment: a settled excerpt (or a settled failure) is never re-fetched, and a
    // close-and-reopen shows what was already read.
    if (!open || text !== null || error !== null) return;
    let cancelled = false;
    (async () => {
      try {
        // `Range` is a request the server may decline (it answers `200` with the whole body), so the
        // excerpt is clipped by character count below regardless of what came back.
        const response = await fetch(url, whole ? {} : { headers: { Range: rangeHeader() } });
        if (!response.ok) throw new Error(await errorMessage(response));
        const body = await response.text();
        if (!cancelled) setText(whole ? body : withoutPartialTail(body));
      } catch (cause) {
        if (!cancelled) setError(cause instanceof Error ? cause.message : String(cause));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, url, text, error, whole]);

  const clipped = text !== null && (!whole || text.length > EXCERPT_CHARS);
  return (
    <div className="max-w-full min-w-0">
      <Disclosure
        open={open}
        onToggle={() => setOpen(!open)}
        label={attachment.name}
        summary={`${attachment.mime} · ${formatBytes(attachment.byte_len)}`}
      />
      {open && (
        <div className="mt-1 pl-5">
          {error !== null ? (
            <p className="font-mono text-2xs text-clay">could not read the file — {error}</p>
          ) : text === null ? (
            <p className="font-mono text-2xs text-ink-faint">reading…</p>
          ) : (
            <>
              <Excerpt>{clipped ? text.slice(0, EXCERPT_CHARS) : text}</Excerpt>
              {clipped && (
                <p className="mt-1 font-mono text-2xs text-ink-faint">
                  clipped ·{" "}
                  <a href={url} target="_blank" rel="noreferrer" className="hover:text-clay">
                    open the whole file
                  </a>
                </p>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}

/// The announcement treatment: what the file is called, what it is, and how big — all the record
/// holds. It links to the bytes when an agent serves them, and stands as a plain chip when none does.
function AttachmentChip({ attachment, url }: { attachment: Attachment; url: string | null }) {
  const label = (
    <ChipLabel name={attachment.name} mime={attachment.mime} byteLen={attachment.byte_len} />
  );
  if (url === null) return <span className={chipClass}>{label}</span>;
  return (
    <a
      href={url}
      download={attachment.name}
      className={chipClass + " transition-colors hover:border-line-strong hover:text-ink"}
    >
      {label}
    </a>
  );
}

/// A chip's contents: the name, then the type and size in the faint register, so a row of chips scans
/// by name with the particulars available without competing.
function ChipLabel({ name, mime, byteLen }: { name: string; mime: string; byteLen: number }) {
  return (
    <>
      <span className="truncate text-ink-soft">{name}</span>
      <span className="shrink-0 text-ink-faint">
        {mime} · {formatBytes(byteLen)}
      </span>
    </>
  );
}

/// The one chip shape, shared by the announcement and the composer's pending files: a hairline on the
/// oat ground, in the mono register the console's data wears.
const chipClass =
  "inline-flex max-w-full items-baseline gap-2 rounded-xs border border-line bg-oat/40 px-2.5 py-1.5 font-mono text-2xs text-ink-soft";

/// How much of a text attachment the disclosure shows before clipping. Generous enough that most files
/// read whole, short enough that a log dump does not become the transcript.
const EXCERPT_CHARS = 4000;

/// How many bytes are fetched for a file too big to read whole: four per excerpt character, the
/// longest a UTF-8 scalar value runs to, so the window always holds at least the characters shown.
const EXCERPT_BYTES = EXCERPT_CHARS * 4;

/// The `Range` header the excerpt asks for — the file's opening bytes, inclusive of both ends.
function rangeHeader(): string {
  return `bytes=0-${EXCERPT_BYTES - 1}`;
}

/// `body` without a trailing replacement character. A byte window can land inside a multi-byte
/// character, which decodes to U+FFFD at the very end; the character is not the file's, so it is
/// dropped rather than shown as a stray glyph in the excerpt.
function withoutPartialTail(body: string): string {
  return body.endsWith("\uFFFD") ? body.slice(0, -1) : body;
}
